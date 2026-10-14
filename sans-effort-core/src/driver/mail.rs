//! The mailbox: one slot per outstanding request.
//!
//! A slot exists from when the request is recorded until the future takes its
//! value or is dropped; slots closed by a drop are remembered in `closed`
//! until a host asks, so that a host keeping its own table of handles can
//! forget them too.
//!
//! A slot keeps the waker its future was last polled with, and a delivery
//! hands it back to be woken, so a combinator that polls only what was woken
//! (`FuturesUnordered`, `join_all`) sees the reply.
//!
//! Values are stored as [`Value`] — the closed menu — not type-erased. The
//! slot records no kind: only a [`ReplyHandle<T>`] minted for it can deliver,
//! and that handle's `T` is the proof. The kind check a foreign host needs
//! lives in the host layer, where an untyped id arrives.

use super::sync::AtomicU64;
use crate::{
    boundary::codec::DecodeError,
    reply::{handle::ReplyHandle, value::Value},
};
use alloc::vec::Vec;
use core::{sync::atomic::Ordering, task::Waker};

/// Every mailbox gets a number no other mailbox in the process has: the one
/// atomic the driver touches, once, at construction.
static DRIVERS: AtomicU64 = AtomicU64::new(1);

pub(super) struct Mail {
    driver: u64,
    /// Open slots, sorted by id, so a lookup is a binary search. Ids are
    /// minted in order, so opening a slot is almost always a push. Not
    /// indexed by id: one long-lived request would keep every later slot.
    slots: Vec<Slot>,
    closed: Vec<u64>,
    next: u64,
}

struct Slot {
    id: u64,
    value: Option<Value>,
    check: Check,
    /// The waker the request's future was last polled with, if it is still
    /// waiting.
    waker: Option<Waker>,
}

/// Whether a value is a valid answer for a slot: fixed when the request is
/// minted, run before any reply is accepted.
pub(super) type Check = fn(&Value) -> Result<(), DecodeError>;

/// What became of a reply.
pub(super) enum Delivery<T> {
    /// Stored for the routine to collect. The waker its future was last
    /// polled with, to wake once the lock is released.
    Delivered(Option<Waker>),

    /// The routine no longer waits for it; discarded.
    Gone,

    /// Not a valid answer; refused, and the handle given back.
    Refused(ReplyHandle<T>, DecodeError),
}

impl Mail {
    pub(super) fn new() -> Self {
        Self {
            driver: DRIVERS.fetch_add(1, Ordering::Relaxed),
            slots: Vec::new(),
            closed: Vec::new(),
            next: 1,
        }
    }

    /// A fresh handle, as a request is recorded. Its slot opens right after,
    /// once the effect has been built around it.
    pub(super) const fn mint<T>(&mut self) -> ReplyHandle<T> {
        let id = self.next;
        self.next += 1;
        ReplyHandle::mint(self.driver, id)
    }

    pub(super) fn open(&mut self, id: u64, check: Check) {
        let slot = Slot {
            id,
            value: None,
            check,
            waker: None,
        };
        if self.slots.last().is_none_or(|last| id > last.id) {
            self.slots.push(slot);
        } else {
            let at = self.slots.partition_point(|open| open.id < id);
            self.slots.insert(at, slot);
        }
    }

    /// Take the slot at `at` out, keeping the rest in order.
    fn remove(&mut self, at: usize) -> Option<Slot> {
        if at + 1 == self.slots.len() {
            self.slots.pop()
        } else {
            (at < self.slots.len()).then(|| self.slots.remove(at))
        }
    }

    /// Where request `id`'s slot is. The newest first — almost always the
    /// one asked about — then a binary search.
    fn position(&self, id: u64) -> Option<usize> {
        let newest = self.slots.len().checked_sub(1)?;
        if self.slots.get(newest).is_some_and(|slot| slot.id == id) {
            return Some(newest);
        }
        self.slots.binary_search_by_key(&id, |slot| slot.id).ok()
    }

    /// `false` if no request with that id is waiting: it was already
    /// answered or its future was dropped.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver.
    pub(super) fn deliver<T>(&mut self, reply: ReplyHandle<T>, value: Value) -> Delivery<T> {
        let (driver, id) = reply.into_parts();
        assert_eq!(
            driver, self.driver,
            "ReplyHandle({driver}/{id}) was minted by driver {driver} and replied to driver {}",
            self.driver
        );

        match self.position(id).and_then(|at| self.slots.get_mut(at)) {
            Some(slot) => match (slot.check)(&value) {
                Ok(()) => {
                    slot.value = Some(value);
                    Delivery::Delivered(slot.waker.take())
                }
                Err(error) => Delivery::Refused(ReplyHandle::mint(driver, id), error),
            },
            None => Delivery::Gone,
        }
    }

    /// `Some(value)` closes the slot; `None` leaves it waiting, to wake
    /// `waker`, if there is one, when the reply comes.
    pub(super) fn collect(&mut self, id: u64, waker: Option<&Waker>) -> Option<Value> {
        let at = self.position(id)?;
        let slot = self.slots.get_mut(at)?;
        if slot.value.is_some() {
            return self.remove(at).and_then(|slot| slot.value);
        }
        if let Some(waker) = waker
            && !slot
                .waker
                .as_ref()
                .is_some_and(|kept| kept.will_wake(waker))
        {
            slot.waker = Some(waker.clone());
        }
        None
    }

    /// The routine dropped a polled request. Its slot goes; if no reply had
    /// arrived, its id is reported in the next step's
    /// [`closed`](super::Yield::closed). A request whose reply arrived —
    /// collected or not — was answered, not abandoned: the host has no work
    /// left to stop, so it is not reported.
    pub(super) fn close(&mut self, id: u64) {
        if let Some(at) = self.position(id)
            && self.remove(at).is_some_and(|slot| slot.value.is_none())
        {
            self.closed.push(id);
        }
    }

    pub(super) fn take_closed(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.closed)
    }

    pub(super) const fn outstanding(&self) -> usize {
        self.slots.len()
    }
}
