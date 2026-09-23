//! The reifying context: every call records an effect for a host.

use crate::request::{Asked, Request};
use sans_effort::driver::{awaiting::Awaiting, outbox::Outbox};

/// A context that serves every capability by asking the host.
///
/// Generic over the host's vocabulary `E`. Each trait in this crate is
/// implemented for `Ctx<E>` exactly when `E` can carry that trait's effect —
/// `E: From<Asked<time::effect::Sleep>>` for [`Sleep`](crate::time::Sleep) —
/// so the set of traits `Ctx<E>` implements _is_ the set of capabilities the
/// host has agreed to provide.
#[derive(Debug)]
pub struct Ctx<E> {
    outbox: Outbox<E>,
}

impl<E> Ctx<E> {
    /// A context writing into `outbox`.
    #[must_use]
    pub const fn new(outbox: Outbox<E>) -> Self {
        Self { outbox }
    }

    /// Ask the host `request` and await its reply: how an application
    /// implements its own capabilities for this context.
    ///
    /// ```
    /// # use sans_effort_effects::{Ctx, request::{Asked, Request}};
    /// trait Lookup { async fn lookup(&self, name: String) -> String; }
    ///
    /// struct LookupRequest(String);
    /// impl Request for LookupRequest { type Reply = String; }
    ///
    /// impl<E: From<Asked<LookupRequest>>> Lookup for Ctx<E> {
    ///     async fn lookup(&self, name: String) -> String {
    ///         self.request(LookupRequest(name)).await
    ///     }
    /// }
    /// ```
    pub fn request<R: Request>(&self, request: R) -> Awaiting<E, R::Reply>
    where
        E: From<Asked<R>>,
    {
        self.outbox.ask(|reply| E::from(Asked { request, reply }))
    }

    /// Tell the host `message`, expecting no reply.
    pub fn notify<M>(&self, message: M)
    where
        E: From<M>,
    {
        self.outbox.tell(E::from(message));
    }
}
