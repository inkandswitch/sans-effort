//! What crosses a boundary.
//!
//! This is the `no_std` half of the boundary. The handle table, panic
//! isolation, and input decoding are `effect_routine_host`, which is `std`; a
//! routine's wire crate depends on this module without pulling that in.
//!
//! - [`host_effect`] is how an effect type is shown to a host that cannot
//!   hold a Rust value: its [`ReplyHandle`](crate::reply::handle::ReplyHandle)
//!   detached and replaced by the request id.
//! - [`pending`] is the detached handle, wrapped so the host layer can keep
//!   it in one table and type-check the reply against it.
//! - [`codec`] writes a host's view of an effect as bytes and reads a reply
//!   record back.
//!
//! # Terms
//!
//! _The wire_ is this whole boundary — everything that exists because a host
//! cannot hold a Rust value — and the demo's `greeter_wire` crate is named for
//! it. A [`HostEffect`](host_effect::HostEffect) is an effect type that can
//! cross it. The [`codec`] is the narrowest sense: the bytes themselves.
//!
//! ```text
//!   Full::Lookup(Asked { request: Lookup("bob"), reply: ReplyHandle<String>{id: 2} })
//!         │
//!         │ HostEffect::split
//!         ▼
//!   ( View::Lookup { name: "bob", id: 2 } ,  Some(Pending::Str(handle)) )
//!     └────── Encode ──▶ bytes ──▶ host ┘    └── kept by the host layer ──┘
//! ```

pub mod codec;
pub mod host_effect;
pub mod pending;
