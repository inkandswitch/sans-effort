//! The mailbox: one slot per outstanding request.
//!
//! A slot exists from the request's first poll until the future takes its
//! value or is dropped; slots closed by a drop are remembered in `closed`
//! until a host asks, so that a host keeping its own table of handles can
//! forget them too.
//!
//! Values are stored as [`Value`] — the closed menu — not type-erased. The
//! slot records no kind: only a [`ReplyHandle<T>`] minted for it can deliver,
//! and that handle's `T` is the proof. The kind check a foreign host needs
//! lives in the host layer, where an untyped id arrives.

use super::sync::AtomicU64;
use crate::reply::{handle::ReplyHandle, value::Value};
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

/// Every mailbox gets a number no other mailbox in the process has: the one
/// atomic the driver touches, once, at construction.
static DRIVERS: AtomicU64 = AtomicU64::new(1);

pub(super) struct Mail {
    driver: u64,
    /// Open slots by id. A `Vec` scanned linearly: a routine has one or two
    /// requests in flight, and a tree or a hash costs more than two compares.
    /// Wide fan-out would want a sorted `Vec` with binary search.
    slots: Vec<Slot>,
    closed: Vec<u64>,
    next: u64,
}

struct Slot {
    id: u64,
    value: Option<Value>,
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

    /// A fresh handle. Its slot is not open yet: that happens on first poll.
    pub(super) const fn mint<T>(&mut self) -> ReplyHandle<T> {
        let id = self.next;
        self.next += 1;
        ReplyHandle::mint(self.driver, id)
    }

    pub(super) fn open(&mut self, id: u64) {
        self.slots.push(Slot { id, value: None });
    }

    fn position(&self, id: u64) -> Option<usize> {
        self.slots.iter().position(|slot| slot.id == id)
    }

    /// `false` if no request with that id is waiting: it was already
    /// answered or its future was dropped.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver.
    pub(super) fn deliver<T>(&mut self, reply: ReplyHandle<T>, value: Value) -> bool {
        let (driver, id) = reply.into_parts();
        assert_eq!(
            driver, self.driver,
            "ReplyHandle({driver}/{id}) was minted by driver {driver} and replied to driver {}",
            self.driver
        );

        match self.position(id).and_then(|at| self.slots.get_mut(at)) {
            Some(slot) => {
                slot.value = Some(value);
                true
            }
            None => false,
        }
    }

    /// `Some(value)` closes the slot; `None` leaves it waiting.
    pub(super) fn collect(&mut self, id: u64) -> Option<Value> {
        let at = self.position(id)?;
        if self.slots.get(at)?.value.is_some() {
            self.slots.swap_remove(at).value
        } else {
            None
        }
    }

    /// The routine dropped a polled request. Its slot goes, and its id is
    /// kept for [`Driver::closed`](super::Driver::closed).
    pub(super) fn close(&mut self, id: u64) {
        if let Some(at) = self.position(id) {
            self.slots.swap_remove(at);
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
