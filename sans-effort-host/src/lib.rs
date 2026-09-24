//! The host side of a routine, for FFI hosts that cannot hold a Rust value —
//! minus the C ABI itself.
//!
//! `abi_version()`, then `new(routine) → handle`, `start(handle) → effects`,
//! `reply(handle, record) → effects` — or `resume(handle) → effects` for a
//! routine waiting on something in the process, which a `woke` frame in some
//! call's output (or `wakes()`) names — then `free(handle)`. A reply record is
//! `kind · id · payload`, where the id came out on the wire with the effect
//! and the kind is one of the reply menu's four
//! ([`Kind`](sans_effort::reply::kind::Kind)).
//!
//! Everything a foreign host needs — the handle table, the type check on
//! replies, the encoding — over owned Rust types, in two layers:
//!
//! - [`machine::Machine`] is _typed_: [`start`](machine::Machine::start) and
//!   [`reply`](machine::Machine::reply) return `Vec<E::View>`, the effects with their
//!   handles replaced by ids. It owns the outstanding-request table and the
//!   kind check. Bindings that speak the host language's own types —
//!   wasm-bindgen, `PyO3`, Rustler — hold one directly, and reply through
//!   its concrete `reply_str`, `reply_u64`, `reply_unit`, or `reply_bytes`.
//! - [`encoded::Encoded`] is the _byte_ layer over it, with the same two
//!   calls: decode one reply record, call the typed method, encode the views.
//!   The [`table`] holds machines behind this, and a C-ABI or `erl_nif` binding
//!   calls it. Most machines migrate between threads; a pinned one, whose
//!   future is not `Send`, runs over a
//!   [`LocalDriver`](sans_effort::driver::LocalDriver) and stays on the thread
//!   that started it.
//!
//! The application adds the _binding_: the thin `unsafe` wrapper a foreign host
//! actually calls — one `#[no_mangle]` wrapper per function in [`table`], each
//! a line plus the `unsafe` needed to touch foreign memory. That keeps this
//! crate under `unsafe_code = "forbid"`.
//!
//! ```text
//!   app cdylib                                 sans-effort-host
//!   ────────────────────────────────           ──────────────────────────────────────────
//!   enum Effect { … }  impl HostEffect
//!   #[no_mangle] abi_version()     ─────────▶  contract::REVISION
//!   #[no_mangle] new()             ─────────▶  table::new(|outbox| Greeter::new(Ctx::new(outbox)).run())
//!   #[no_mangle] start(h, out*)    ─────────▶  table::start(h)          -> Result<(Vec<u8>, Status), Error>
//!   #[no_mangle] reply(h, in*, out*) ───────▶  table::reply(h, &[u8])   -> Result<(Vec<u8>, Status), Error>
//!   #[no_mangle] resume(h, out*)   ─────────▶  table::resume(h)         -> Result<(Vec<u8>, Status), Error>
//!   #[no_mangle] wakes(out*)       ─────────▶  table::wakes()           -> Vec<u8>
//!                                  ◀─────────  (bytes, status)   — app writes the out-pointers
//! ```
//!
//! # Where the Boundary Is Split
//!
//! The [`HostEffect`](sans_effort::boundary::host_effect::HostEffect) and
//! [`Encode`](sans_effort::boundary::codec::Encode) traits, [`Pending`](sans_effort::boundary::pending::Pending),
//! and the codec live in [`sans_effort::boundary`]; the reply menu
//! ([`Value`](sans_effort::reply::value::Value), [`Kind`](sans_effort::reply::kind::Kind))
//! in [`sans_effort::reply`]. Both are `no_std`, so a routine's boundary
//! crate may implement them. This crate decodes reply records (`1 id str`;
//! `2 id u64`; `3 id`; `4 id bytes`), checks the kind, and keeps the table.
//! `ABI.md` at the repository root is the contract in full.
//!
//! # `no_std`
//!
//! Everything but [`table`] is `no_std` + `alloc`. The table needs a
//! process-wide `static Mutex`, a `HashMap`, and `catch_unwind` for panic
//! isolation, so it is behind the default `std` feature; a binding on a target
//! without `std` holds its [`encoded::Encoded`] machines in statics of its own.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod contract;
pub mod encoded;
pub mod error;
pub mod machine;
pub mod status;

#[cfg(feature = "std")]
pub mod table;

#[cfg(test)]
mod fixtures;
