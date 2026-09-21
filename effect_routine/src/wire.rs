//! What crosses a boundary.
//!
//! This is the `no_std` half of the boundary. The handle table, panic
//! isolation, and input decoding are `effect_routine_host`, which is `std`; a
//! routine's wire crate depends on this module without pulling that in.
//!
//! - [`menu`] is every type a reply can have — bytes, a string, a `u64`, or
//!   nothing — as the mailbox stores it and the ABI carries it.
//! - [`host_effect`] is how an effect type is shown to a host that cannot
//!   hold a Rust value: its [`ReplyHandle`](crate::reply::ReplyHandle)
//!   detached and replaced by the request id.
//! - [`pending`] is the detached handle, wrapped so the host layer can keep
//!   it in one table and type-check the reply against it.
//! - [`codec`] writes a host's view of an effect as bytes and reads a reply
//!   record back.
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
pub mod menu;
pub mod pending;
