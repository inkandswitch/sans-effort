//! The coeffects reading: requests as data, requirements as `From` bounds.
//!
//! A routine may describe each wait as a _request struct_ that names its
//! reply type through [`Request`], and state what its host must be able to
//! carry as bounds on the host's effect type: `E: From<Asked<Lookup>>`. The
//! host defines `E` and meets each bound with a `From` impl; the compiler
//! checks provision against requirement where the routine is spawned.
//!
//! ```text
//!   requirement (routine)              provision (host)
//!   E: From<Asked<ReadLine>> + …       enum Full { ReadLine(Asked<ReadLine>), … } + From impls
//!                  outbox.request(ReadLine)  →  E::from(Asked { request, reply })
//! ```
//!
//! Two things fall out. A helper `async fn` bounded on `E: From<Asked<Lookup>>`
//! serves any host that carries `Lookup`, so routines compose across hosts
//! where a closed enum would force them to share one. And a host whose `E`
//! lacks a variant _provably_ never receives that request — attenuation,
//! checked at the spawn site and visible on the wire.
//!
//! The mechanism is these two items; the methods live on
//! [`Post`](crate::post::Post) as [`request`](crate::post::Post::request) and
//! [`notify`](crate::post::Post::notify).

use crate::{reply::ReplyHandle, wire::Reply};

/// A request names the type of its reply.
pub trait Request {
    /// What the host answers with.
    type Reply: Reply;
}

/// A request in flight: what was asked, and the handle the host replies
/// through. A host's effect type carries these; the host layer splits them
/// into a view and a [`Pending`](crate::wire::Pending).
#[derive(Debug)]
pub struct Asked<R: Request> {
    /// What was asked.
    pub request: R,
    /// How to answer it.
    pub reply: ReplyHandle<R::Reply>,
}
