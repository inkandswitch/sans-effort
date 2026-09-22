//! An effect a host can receive.

use super::pending::Pending;

/// An effect a host can receive.
///
/// A host that cannot hold a Rust value — anything across an ABI — sees the
/// effect with its [`ReplyHandle`](crate::reply::handle::ReplyHandle) replaced by the request id. The handle
/// stays on this side as a [`Pending`], so the reply is still typed.
/// [`View`](Self::View) is what every skin hands on: as Python objects, as
/// Erlang terms, or, through [`Encode`](super::codec::Encode), as bytes.
///
/// A Rust host that holds the effect itself never needs this trait.
pub trait HostEffect {
    /// The handle-free view: an awaiting effect carries its `u64` id instead.
    type View;

    /// Separate the host's view from the handle the host layer keeps.
    fn split(self) -> (Self::View, Option<Pending>);
}
