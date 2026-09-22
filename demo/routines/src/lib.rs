//! The demo routines, written against capability traits.
//!
//! Three routines share five capability traits: `Sleep`, `ReadLine`, and
//! `WriteLine` from `sans-effort-effects`, the standard library, and the
//! demo's own [`traits`], `Count` and `Lookup`. [`greeter::Greeter`] is the
//! one every host in the exploration this library came from drove: prompt,
//! read a name, look up a greeting, pause, greet, count, repeat; `quit` or
//! the end of input ends it. [`fanout::Fanout`] is the same conversation
//! with two waits in flight at once. [`ticker::Ticker`] needs only two of
//! the five traits, which is the point of it.
//!
//! A routine owns a context `C` and asks nothing of it beyond its trait
//! bounds. It does not know whether `read_line` awaits a tokio channel, pops
//! a line off a test script, or records an effect for a Python host and
//! suspends; each of those is a different `C`, and the routine is the same
//! code under all of them. This crate imports [`Run`](sans_effort::run::Run)
//! and [`join`](sans_effort::join::join) from the mechanism and the traits
//! from the standard library, and nothing else — no effect, no handle, no
//! driver — and it is `no_std`.
//!
//! ```text
//!   Greeter<C: Count + Lookup + ReadLine + Sleep + WriteLine>: Run
//!        │
//!        ├── C = TokioCtx   (../tokio)        tokio futures; no driver
//!        ├── C = Ctx<E>     (effects stdlib)  records effects; a Driver polls
//!        └── C = Recording  (tests)           ready at once; one poll
//! ```
//!
//! No `Send` appears in the traits. Whether a routine's `run()` is `Send` is
//! decided by `C`, and the compiler works it out where `C` is concrete —
//! `tokio::spawn` accepts a `Greeter<TokioCtx>` because tokio's handles are
//! `Send`; `Driver::new` accepts a `Greeter<Ctx<E>>` for the same
//! reason; and the `Rc`-based test mock is accepted by nothing that asks.

#![no_std]

extern crate alloc;

pub mod fanout;
pub mod greeter;
pub mod ticker;
pub mod traits;

#[cfg(test)]
mod recording;

use core::time::Duration;

/// How long a routine pauses when it pauses.
pub const PAUSE: Duration = Duration::from_millis(50);
