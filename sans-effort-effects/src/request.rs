//! Requests as values: what makes a context generic over the host's
//! vocabulary.
//!
//! The mechanism builds an effect around a handle: `outbox.ask(Effect::Ask)`,
//! where whoever calls `ask` names a variant of one concrete enum. That is
//! enough when there is one vocabulary and you own it. A context that should
//! serve _any_ vocabulary — the reifying [`Ctx`](crate::Ctx), and every
//! capability in this crate — cannot name variants. Instead it describes
//! each wait as a _request struct_ that names its reply type through
//! [`Request`], and states what the host must be able to carry as a bound:
//! `E: From<Asked<Lookup>>`. The host defines `E` and meets each bound with
//! a `From` impl; the compiler checks provision against requirement where
//! the routine is built.
//!
//! ```text
//!   requirement (context)            provision (host)
//!   E: From<Asked<ReadLine>> + …     enum Full { ReadLine(Asked<ReadLine>), … }
//!                                    + From impls
//!             ctx.request(ReadLine)  →  E::from(Asked { request, reply })
//! ```
//!
//! Two things fall out. One impl bounded on `E: From<Asked<Lookup>>` serves
//! any host that carries `Lookup`, so capabilities compose across hosts
//! where a closed enum would force them to share one. And a host whose `E`
//! lacks a variant _provably_ never receives that request — attenuation,
//! checked where the routine is built and visible on the wire.
//!
//! The methods live on [`Ctx`](crate::Ctx) as
//! [`request`](crate::Ctx::request) and [`notify`](crate::Ctx::notify).

use sans_effort::reply::{Reply, handle::ReplyHandle};

/// A request names the type of its reply.
pub trait Request {
    /// What the host answers with.
    type Reply: Reply;
}

/// A request in flight: what was asked, and the handle the host replies
/// through. A host's effect type carries these; the host layer splits them
/// into a view and a
/// [`Pending`](sans_effort::boundary::pending::Pending).
#[derive(Debug)]
pub struct Asked<R: Request> {
    /// What was asked.
    pub request: R,
    /// How to answer it.
    pub reply: ReplyHandle<R::Reply>,
}
