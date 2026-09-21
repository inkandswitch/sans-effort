//! The greeter, exported to JS with `wasm-bindgen`.
//!
//! The routine is the unchanged `greeter` crate, under the reifying context
//! from `greeter_wire`. This crate is the boundary:
//! one JS class, [`Greeter`], whose `start()` runs the routine to its first
//! wait and whose `reply*` methods deliver one answer by request id and run
//! to the next, each returning the batch of effects emitted in between.
//!
//! Beside `../cdylib`, this is the _other_ kind of skin. There, the host gets
//! a `u64` handle and a byte buffer and decodes it with a table it wrote
//! itself. Here `wasm-bindgen` generates the class, the enum, and the typed
//! getters, so there is no handle table and no codec — the
//! [`Machine`] is held directly and its
//! [`View`]s are converted to JS objects. What does not change
//! is the contract: Rust never calls JS; the host owns the loop, the clock,
//! and all IO; requests carry ids, and the host may reply in any order.
//!
//! ```text
//!   JS:  const g = new Greeter();
//!        let { status, effects } = g.start();            // [Write, ReadLine·1]
//!        ({ status, effects } = g.replyStr(1, "bob"));   // [Lookup·2]
//! ```
//!
//! # Panics
//!
//! `wasm32-unknown-unknown` is `panic = "abort"`: a panicking routine traps,
//! the host sees a `RuntimeError`, and the instance should be discarded.
//! There is no `PANICKED` status here because there is no unwinding to catch.

#![allow(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen cannot export `const fn`"
)]
pub mod batch;
pub mod effect;
pub mod greeter;
pub mod kind;
pub mod safe_integer;
pub mod status;
