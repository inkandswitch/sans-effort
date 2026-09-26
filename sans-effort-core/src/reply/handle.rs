//! The capability to answer one `ask`.

use core::{fmt, marker::PhantomData};

/// Evidence that a request for a `T` is outstanding, and the means to answer
/// it.
///
/// Minted by [`Outbox::ask`](crate::driver::outbox::Outbox::ask), consumed by
/// [`Driver::reply`](crate::driver::Driver::reply). Not `Clone`, so it
/// answers at most once; `#[must_use]`, so dropping it silently is a
/// warning. Nothing outside this crate can construct one.
///
/// A `ReplyHandle` names one outstanding call, not a mailbox: it is Erlang's
/// `From = {Pid, Tag}`, not Akka's `replyTo: ActorRef`. The [`id`](Self::id)
/// is what crosses a wire — per driver, from 1, so a replay can address
/// requests by it; the type keeps the answer honest on the Rust side; and the
/// driver number says which mailbox minted it, so a handle given to the wrong
/// driver is a caught mistake and not a delivery to whichever slot happens to
/// share its id.
#[must_use = "a dropped reply handle leaves the routine waiting forever"]
pub struct ReplyHandle<T> {
    driver: u64,
    id: u64,
    _reply: PhantomData<fn() -> T>,
}

impl<T> ReplyHandle<T> {
    pub(crate) const fn mint(driver: u64, id: u64) -> Self {
        Self {
            driver,
            id,
            _reply: PhantomData,
        }
    }

    /// The request id, unique within the driver that minted this handle.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Consume the handle: which driver minted it, and which request.
    /// The same handle, typed as another form of its answer — its wire
    /// kind, for the host layer's table.
    pub(crate) const fn retype<U>(self) -> ReplyHandle<U> {
        ReplyHandle::mint(self.driver, self.id)
    }

    pub(crate) const fn into_parts(self) -> (u64, u64) {
        (self.driver, self.id)
    }
}

impl<T> fmt::Debug for ReplyHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReplyHandle({}/{})", self.driver, self.id)
    }
}

/// Two handles are equal when they address the same request of the same
/// driver. Identity, not structure: there is nothing else in a handle.
impl<T> PartialEq for ReplyHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.driver == other.driver && self.id == other.id
    }
}

impl<T> Eq for ReplyHandle<T> {}
