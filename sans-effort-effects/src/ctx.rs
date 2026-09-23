//! The reifying context: every call records an effect for a host.

use crate::request::{Asked, Request};
use alloc::{boxed::Box, rc::Rc};
use sans_effort::{
    driver::{ask::Ask, outbox::Outbox},
    reply::handle::ReplyHandle,
};

/// A context that serves every capability by asking the host.
///
/// Generic over the host's vocabulary `E`. Each trait in this crate is
/// implemented for `Ctx<E>` exactly when `E` can carry that trait's effect —
/// `E: From<Asked<time::effect::Sleep>>` for [`Sleep`](crate::time::Sleep) —
/// so the set of traits `Ctx<E>` implements _is_ the set of capabilities the
/// host has agreed to provide.
///
/// The impls are written over [`AsCtx`], which `Ctx<E>` implements, so a
/// newtype wrapping a `Ctx` gets the same capabilities from one method.
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
    /// # use sans_effort_effects::{ctx::Ctx, request::{Asked, Request}};
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
    pub fn request<R: Request>(
        &self,
        request: R,
    ) -> Ask<E, R::Reply, impl FnOnce(ReplyHandle<R::Reply>) -> E>
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

/// Anything that can be viewed as a [`Ctx`]: the reifying context itself, a
/// reference or smart pointer to one, or a newtype wrapping one.
///
/// Every capability in this crate is implemented for any `C: AsCtx` whose
/// vocabulary can carry its effect, so implementing this one method gives a
/// newtype all of them. That is how a crate reifies a capability trait it
/// does not own: the orphan rule forbids `impl TheirTrait for Ctx<E>`, but
/// allows it for a local newtype, which then needs nothing else.
///
/// ```
/// use sans_effort_effects::ctx::{AsCtx, Ctx};
///
/// struct HostCtx<E>(Ctx<E>);
///
/// impl<E> AsCtx for HostCtx<E> {
///     type Vocabulary = E;
///
///     fn ctx(&self) -> &Ctx<E> {
///         &self.0
///     }
/// }
/// ```
///
/// Nothing else needs this trait: routines name capabilities, `Ctx<E>`
/// already implements it, and native contexts implement capabilities
/// directly. A type that implements `AsCtx` gets its stdlib capabilities
/// from it and cannot also implement them by hand.
pub trait AsCtx {
    /// The host's vocabulary the underlying [`Ctx`] writes into.
    type Vocabulary;

    /// The reifying context to record effects through.
    fn ctx(&self) -> &Ctx<Self::Vocabulary>;
}

impl<E> AsCtx for Ctx<E> {
    type Vocabulary = E;

    fn ctx(&self) -> &Self {
        self
    }
}

impl<C: AsCtx + ?Sized> AsCtx for &C {
    type Vocabulary = C::Vocabulary;

    fn ctx(&self) -> &Ctx<Self::Vocabulary> {
        (**self).ctx()
    }
}

impl<C: AsCtx + ?Sized> AsCtx for Box<C> {
    type Vocabulary = C::Vocabulary;

    fn ctx(&self) -> &Ctx<Self::Vocabulary> {
        (**self).ctx()
    }
}

impl<C: AsCtx + ?Sized> AsCtx for Rc<C> {
    type Vocabulary = C::Vocabulary;

    fn ctx(&self) -> &Ctx<Self::Vocabulary> {
        (**self).ctx()
    }
}

#[cfg(target_has_atomic = "ptr")]
impl<C: AsCtx + ?Sized> AsCtx for alloc::sync::Arc<C> {
    type Vocabulary = C::Vocabulary;

    fn ctx(&self) -> &Ctx<Self::Vocabulary> {
        (**self).ctx()
    }
}
