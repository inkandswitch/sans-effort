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
    /// Slots sorted by id, so a lookup is a binary search; ids are minted in
    /// order, so opening a slot is almost always a push. A slot taken from
    /// anywhere but the end is closed where it stands, and the closed ones
    /// are swept out together once they outnumber the open: taking a slot is
    /// O(1) amortized, in whatever order a host answers. Not indexed by id:
    /// one long-lived request would keep every later slot.
    slots: Vec<Slot>,
    /// How many of `slots` are open.
    open: usize,
    closed: Vec<u64>,
    next: u64,
}

struct Slot {
    id: u64,
    /// `false` once taken: closed in place until the next sweep.
    open: bool,
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
            open: 0,
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
            open: true,
            value: None,
            check,
            waker: None,
        };
        if self.slots.last().is_none_or(|last| id > last.id) {
            self.slots.push(slot);
        } else {
            let at = self.slots.partition_point(|other| other.id < id);
            self.slots.insert(at, slot);
        }
        self.open += 1;
    }

    /// Take the open slot at `at` (from [`position`](Self::position)): its
    /// value, if a reply arrived. The last slot is popped, with any closed
    /// ones before it; any other is closed in place, and the closed swept out
    /// once they outnumber the open.
    fn take(&mut self, at: usize) -> Option<Value> {
        let slot = self.slots.get_mut(at).filter(|slot| slot.open)?;
        slot.open = false;
        slot.waker = None;
        let value = slot.value.take();
        self.open -= 1;
        while self.slots.last().is_some_and(|last| !last.open) {
            self.slots.pop();
        }
        if self.slots.len() - self.open > self.open {
            self.slots.retain(|slot| slot.open);
        }
        value
    }

    /// Where request `id`'s open slot is. The newest first — almost always
    /// the one asked about — then a binary search.
    fn position(&self, id: u64) -> Option<usize> {
        let newest = self.slots.len().checked_sub(1)?;
        if self.slots.get(newest).is_some_and(|slot| slot.id == id) {
            return Some(newest);
        }
        let at = self.slots.binary_search_by_key(&id, |slot| slot.id).ok()?;
        self.slots.get(at).filter(|slot| slot.open).map(|_| at)
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
            return self.take(at);
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
            && self.take(at).is_none()
        {
            self.closed.push(id);
        }
    }

    pub(super) fn take_closed(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.closed)
    }

    pub(super) const fn outstanding(&self) -> usize {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    /// The structure's own invariants: ids strictly increasing, the count of
    /// open slots right, the last slot open, and never more closed slots
    /// than open ones (or a sweep is overdue).
    fn assert_shape(mail: &Mail) {
        assert!(
            mail.slots
                .windows(2)
                .all(|w| matches!(w, [a, b] if a.id < b.id))
        );
        let open = mail.slots.iter().filter(|slot| slot.open).count();
        assert_eq!(open, mail.open);
        assert!(mail.slots.last().is_none_or(|last| last.open));
        assert!(mail.slots.len() - mail.open <= mail.open);
    }

    /// Any sequence of opens, deliveries, collections, and closes behaves as
    /// a map of the open requests does, in whatever order requests are
    /// answered and dropped.
    #[test]
    fn the_mailbox_behaves_as_a_map_of_open_requests() {
        bolero::check!()
            .with_type::<Vec<(u8, u8)>>()
            .for_each(|ops| {
                let mut mail = Mail::new();
                let mut model: BTreeMap<u64, Option<u64>> = BTreeMap::new();
                let mut minted = Vec::new();
                let mut closed = Vec::new();
                for &(op, key) in ops {
                    let id = minted
                        .get(usize::from(key) % minted.len().max(1))
                        .copied()
                        .unwrap_or(0);
                    match op % 4 {
                        0 => {
                            let id = mail.mint::<u64>().id();
                            mail.open(id, crate::reply::check::<u64>);
                            minted.push(id);
                            model.insert(id, None);
                        }
                        1 => {
                            let reply = ReplyHandle::<u64>::mint(mail.driver, id);
                            let delivered = matches!(
                                mail.deliver(reply, Value::U64(id)),
                                Delivery::Delivered(_)
                            );
                            assert_eq!(delivered, model.contains_key(&id));
                            if let Some(slot) = model.get_mut(&id) {
                                *slot = Some(id);
                            }
                        }
                        2 => {
                            let expected = match model.get(&id) {
                                Some(Some(value)) => Some(*value),
                                _ => None,
                            };
                            let got = mail.collect(id, None).map(|value| match value {
                                Value::U64(n) => n,
                                Value::Bytes(_) | Value::Str(_) | Value::Unit => u64::MAX,
                            });
                            assert_eq!(got, expected);
                            if expected.is_some() {
                                model.remove(&id);
                            }
                        }
                        _ => {
                            mail.close(id);
                            if let Some(value) = model.remove(&id)
                                && value.is_none()
                            {
                                closed.push(id);
                            }
                        }
                    }
                    assert_eq!(mail.outstanding(), model.len());
                    assert_shape(&mail);
                }
                assert_eq!(mail.take_closed(), closed);
            });
    }
}
