//! The boundary between a routine and a host that cannot hold a Rust value.
//!
//! This is the `no_std` half of the boundary. The handle table, panic
//! isolation, and input decoding are `sans-effort-host`, which is `std`; a
//! routine's boundary crate depends on this module without pulling that in.
//!
//! - [`host_effect`] is how an effect type is shown to a host that cannot
//!   hold a Rust value: its [`ReplyHandle`](crate::reply::handle::ReplyHandle)
//!   detached and replaced by the request id.
//! - [`pending`] is the detached handle, wrapped so the host layer can keep
//!   it in one table and type-check the reply against it.
//! - [`codec`] writes values as bytes and reads them back: a host's view of an
//!   effect on the way out, and anything richer than the four reply kinds on
//!   the way in.
//!
//! # Terms
//!
//! _The boundary_ is everything that exists because a host cannot hold a Rust
//! value — this module, and the demo's `greeter_boundary` crate. A
//! [`HostEffect`](host_effect::HostEffect) is an effect type that can cross it.
//! _The wire_ is the narrowest sense, the bytes themselves: the [`codec`], and
//! `ABI.md`'s subject.
//!
//! ```text
//!   Effect::Lookup { name: "bob", reply: ReplyHandle<String> { id: 2 } }
//!         │
//!         │ HostEffect::split
//!         ▼
//!   ( View::Lookup { name: "bob", id: 2 } ,  Some(Pending::Str(handle)) )
//!     └────── Encode ──▶ bytes ──▶ host ┘    └── kept by the host layer ──┘
//! ```

pub mod codec;
pub mod host_effect;
pub mod pending;
