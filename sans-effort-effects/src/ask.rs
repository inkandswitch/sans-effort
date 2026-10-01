//! Requests as values: what makes a context generic over the host's
//! vocabulary.
//!
//! The mechanism builds an effect around a handle: `outbox.ask(Effect::Ask)`,
//! where whoever calls `ask` names a variant of one concrete enum. That is
//! enough when there is one vocabulary and you own it. A context that should
//! serve _any_ vocabulary — the reifying [`Ctx`](crate::ctx::Ctx), and every
//! effect trait in this crate — cannot name variants. Instead it describes
//! each wait as a _request struct_ that names its answer through [`Ask`],
//! and states what the host must be able to carry as a bound:
//! `E: From<Asked<Lookup>>`. The host defines `E` and meets each bound with
//! a `From` impl; the compiler checks provision against requirement where
//! the routine is built.
//!
//! ```text
//!   requirement (context)            provision (host)
//!   E: From<Asked<ReadLine>> + …     enum Full { ReadLine(Asked<ReadLine>), … }
//!                                    + From impls
//!             ctx.ask(ReadLine)      →  E::from(Asked { request, reply })
//! ```
//!
//! Two things fall out. One impl bounded on `E: From<Asked<Lookup>>` serves
//! any host that carries `Lookup`, so effect traits compose across hosts
//! where a closed enum would force them to share one. And a host whose `E`
//! lacks a variant _provably_ never receives that request — attenuation,
//! checked where the routine is built and visible on the wire.
//!
//! The methods live on [`Ctx`](crate::ctx::Ctx) as
//! [`ask`](crate::ctx::Ctx::ask) and
//! [`tell`](crate::ctx::Ctx::tell).

use sans_effort_core::reply::{Answer, handle::ReplyHandle};

/// A request names the type of its reply.
pub trait Ask {
    /// What the host answers with: one of the wire kinds, or a type that
    /// crosses as one — `Result<String, ReadLineError>` crosses as bytes.
    type Reply: Answer;
}

/// A request in flight: what was asked, and the handle the host replies
/// through. A host's effect type carries these; the host layer splits them
/// into a view and a
/// [`Pending`](sans_effort_core::boundary::pending::Pending).
#[derive(Debug)]
pub struct Asked<R: Ask> {
    /// What was asked.
    pub request: R,
    /// How to answer it.
    pub reply: ReplyHandle<R::Reply>,
}
