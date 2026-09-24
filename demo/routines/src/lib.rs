//! The demo routines, written against capability traits.
//!
//! Three routines share five capability traits: `Sleep`, `ReadLine`, and
//! `WriteLine` from `sans-effort-effects`, the standard library, and the
//! demo's own [`traits`], `Count` and `Lookup`. [`greeter::Greeter`] is the
//! one every host in the exploration this library came from drove: prompt,
//! read a name, look up a greeting, pause, greet, count, repeat; `quit` or
//! the end of input ends it. [`fanout::Fanout`] is the same conversation
//! with two waits in flight at once. [`ticker::Ticker`] needs only two of
//! the five traits, which is the point of it. [`ping_pong::PingPong`]
//! spawns a child and talks to it over plain `async-channel`s: two machines,
//! and messages that are never effects. [`front_desk::FrontDesk`] spawns a
//! clerk per visitor, each replying on a one-shot channel. [`ring::Ring`]
//! passes a counter around a ring of routines: the cost of one hop.
//!
//! A routine owns a context `C` and asks nothing of it beyond its trait
//! bounds. It does not know whether `read_line` awaits a tokio channel, pops
//! a line off a test script, or records an effect for a Python host and
//! suspends; each of those is a different `C`, and the routine is the same
//! code under all of them. This crate imports [`Run`](sans_effort::run::Run)
//! and [`join`](sans_effort::join::join) from the mechanism, the traits from
//! the standard library, and `async-channel`, and nothing else — no effect,
//! no handle, no driver — and it is `no_std`.
//!
//! ```text
//!   Greeter<C: Count + Lookup + ReadLine + Sleep + WriteLine>: Run
//!        │
//!        ├── C = TokioCtx   (../tokio)        tokio futures; no driver
//!        ├── C = Ctx<E>     (effects stdlib)  records effects; a Driver polls
//!        └── C = Recording  (tests)           ready at once; one poll
//! ```
//!
//! The capability traits declare their futures `Send`, so every context is
//! `Sync` and a routine over one is `Send`: `tokio::spawn` accepts a
//! `Greeter<TokioCtx>`, `Driver::new` a `Greeter<Ctx<E>>`, and `PingPong`,
//! generic as it is, can `spawn` a child that may run on any thread. A
//! context over a value tied to one thread keeps it behind a task of its own
//! and talks to it over a channel, as `greeter_wasm`'s `JsCtx` does.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod fanout;
pub mod front_desk;
pub mod greeter;
pub mod ping_pong;
pub mod ring;
pub mod ticker;
pub mod traits;

#[cfg(test)]
mod recording;

use core::time::Duration;

/// How long a routine pauses when it pauses.
pub const PAUSE: Duration = Duration::from_millis(50);
