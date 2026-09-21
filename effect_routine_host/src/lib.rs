//! The host side of an effect routine, for hosts that cannot hold a Rust
//! value — minus the C ABI.
//!
//! `new(routine) → handle`, `start(handle) → effects`, `reply(handle, record)
//! → effects`, `free(handle)`. A reply record is `kind · id · payload`, where
//! the id came out on the wire with the effect and the kind is one of the
//! reply menu's ([`Pending`](effect_routine::wire::pending::Pending)).
//!
//! Everything a foreign host needs — the handle table, the type check on
//! replies, the encoding — over owned Rust types, in two layers:
//!
//! - [`machine::Machine`] is _typed_: [`start`](machine::Machine::start) and
//!   [`reply`](machine::Machine::reply) return `Vec<E::View>`, the effects with their
//!   handles replaced by ids. It owns the outstanding-request table and the
//!   kind check. Skins that speak the host language's own types —
//!   wasm-bindgen, `PyO3`, Rustler — hold one directly.
//! - [`start_encoded`](machine::Machine::start_encoded) and
//!   [`reply_encoded`](machine::Machine::reply_encoded) are the _byte_ layer over it:
//!   decode one reply record, call the typed method, encode the views. The
//!   [`table`] holds machines behind this, and a C-ABI or `erl_nif` skin
//!   calls it.
//!
//! The application adds the skin: one `#[no_mangle]` wrapper per function in
//! [`table`], each a line plus the `unsafe` needed to touch foreign memory.
//! That keeps this crate under `unsafe_code = "forbid"`.
//!
//! ```text
//!   app cdylib                                 effect_routine_host
//!   ────────────────────────────────           ──────────────────────────────────────────
//!   enum Effect { … }  impl HostEffect
//!   #[no_mangle] new()             ─────────▶  table::new(|outbox| Greeter::new(Ctx::new(outbox)).run())
//!   #[no_mangle] start(h, out*)    ─────────▶  table::start(h)          -> Result<(Vec<u8>, Status), Error>
//!   #[no_mangle] reply(h, in*, out*) ───────▶  table::reply(h, &[u8])   -> Result<(Vec<u8>, Status), Error>
//!                                  ◀─────────  (bytes, status)   — app writes the out-pointers
//! ```
//!
//! # Wire
//!
//! The codec, the reply menu ([`Value`](effect_routine::reply::value::Value)), and
//! the [`HostEffect`](effect_routine::wire::host_effect::HostEffect)/[`Encode`](effect_routine::wire::codec::Encode) traits live in [`effect_routine::wire`] — the
//! `no_std` half of the boundary, so a routine's wire crate may implement
//! them. This crate decodes reply records (`1 id str`; `2 id u64`; `3 id`;
//! `4 id bytes`) and keeps the table. `ABI.md` at the repository root is the
//! contract in full.
pub mod code;
pub mod error;
pub mod machine;
pub mod status;
pub mod table;

#[cfg(test)]
mod fixtures;
