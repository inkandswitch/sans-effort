//! Direct-style async Rust, run natively or driven from an FFI host.
//!
//! _sans-io, without all the effort._ Write an ordinary `async fn` — loops,
//! `?`, `.await` — and run it two ways:
//!
//! 1. Call it from Rust as normal: on tokio it is a plain future at native
//!    speed, with no driver.
//! 2. Drive it from an FFI host language through a sans-io interface: every
//!    wait becomes a typed effect that the host answers by id, so the host
//!    owns I/O, time, and scheduling — which routine resumes, in what order
//!    replies arrive, whether the clock is real or virtual — and the output
//!    is typically byte-identical to the native run.
//!
//! An _effect routine_ — the unit this crate runs; "routine" from here on —
//! is such an `async fn`: every wait is a typed effect answered by whoever
//! drives it. It asks for traits, not effects, and cannot tell which way it
//! is running. The compiler writes the state machine; the only `Pin` is one
//! `Box::pin` at an FFI boundary, if and when there is one.
//!
//! ```text
//!   host                                    routine
//!     │                                         │
//!     │  start()                                │
//!     │────────────────────────────────────────▶│ run until input needed
//!     │                                         │
//!     │        [WriteLine, (ReadLine, 1)]       │
//!     │◀────────────────────────────────────────┊
//!     │                                         ┊ AWAITING
//!     │             reply(1, "bob")             ┊
//!     │────────────────────────────────────────▶┊
//!     │                                         │
//!     │              [(Lookup, 2)]              │
//!     │◀────────────────────────────────────────┊
//!     │                                         ┊ AWAITING
//!     │            reply(2, "Hello")            ┊
//!     │────────────────────────────────────────▶┊
//!     │                                         │
//!     │  [WriteLine]                            │
//!     │◀────────────────────────────────────────│ COMPLETE
//!     │                                         ┴
//! ```
//!
//! # Three Layers
//!
//! The _host_ is whoever polls — tokio, or an FFI host language over a
//! [`Driver`](driver::Driver). A _context_ implements the routine's traits
//! and decides what each call does: a real future on a runtime, or an effect
//! recorded for a host. The _routine_ asks for traits and never sees an
//! effect.
//!
//! ```text
//!   ┌───────────────────────────────────────┬───────────────────────────┐
//!   │ host        tokio or the JS event     │  a Driver polls; Python,  │
//!   │             loop polls — no driver    │  Java, a test… performs   │
//!   ├───────────────────────────────────────┼───────────────────────────┤
//!   │ context     impl Sleep for TokioCtx   │  impl Sleep for Ctx<E>    │
//!   │             a real future             │  records an effect, waits │
//!   ├───────────────────────────────────────┴───────────────────────────┤
//!   │ routine     Greeter<C: Sleep + Lookup + Console>: Run             │  no_std
//!   │             owns the logic; knows nothing of effects or hosts     │
//!   └───────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The routine is the foundation and depends on nothing above it. The native
//! path is why you write it this way: the routine is also a plain `async fn`,
//! usable at native speed by code that has never heard of this crate. The
//! driven path is what this crate provides.
//!
//! # The Pieces
//!
//! - [`run::Run`] is the shape of a routine: a [`step`](run::Run::step) that
//!   is one iteration of its loop, and a [`run`](run::Run::run) that repeats
//!   it until it breaks.
//! - [`driver::outbox::Outbox`] is what a _reifying context_ writes into:
//!   [`tell`](driver::outbox::Outbox::tell) an effect and move on, or
//!   [`ask`](driver::outbox::Outbox::ask) one and await the reply.
//! - [`reply::handle::ReplyHandle`] is the typed, single-use capability to answer one
//!   `ask`. It travels inside the effect to whoever performs it. What it
//!   accepts is the sealed four-kind menu, [`reply::Reply`]: `str`, `u64`,
//!   `unit`, `bytes`.
//! - [`driver::Driver`] turns a routine into something a host can resume:
//!   [`start`](driver::Driver::start), then [`reply`](driver::Driver::reply)
//!   with each handle the effects hand back, until it is finished.
//!   Each call returns a [`Step`](driver::step::Step): the effects, and the
//!   ids of any requests the routine abandoned.
//! - [`join::join`] awaits two waits at once, and [`select::select`] the
//!   first of two; the loser is abandoned and reported closed.
//! - [`boundary`] is what an effect type implements to be shown to a host
//!   that cannot hold a Rust value: [`HostEffect`](boundary::host_effect::HostEffect)
//!   (handles → ids), [`Encode`](boundary::codec::Encode) (the codec), and
//!   [`Pending`](boundary::pending::Pending).
//! - [`testing::run_now`] runs a routine against a mock context whose every
//!   future is ready, in one poll, with no driver.
//!
//! # Writing One
//!
//! The routine names what it needs as traits, and asks for a context that
//! provides them. Nothing from this crate appears in it except [`Run`](run::Run).
//!
//! ```
//! use core::ops::ControlFlow;
//! use sans_effort::run::Run;
//!
//! trait Console {
//!     async fn read_line(&self) -> String;
//!     fn write_line(&self, line: String);
//! }
//!
//! struct Greeter<C: Console> {
//!     ctx: C,
//! }
//!
//! impl<C: Console> Run for Greeter<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         self.ctx.write_line("Who are you?".into());
//!         let name = self.ctx.read_line().await;
//!         self.ctx.write_line(format!("Hello, {name}!"));
//!         ControlFlow::Break(())
//!     }
//! }
//! ```
//!
//! # A Reifying Context
//!
//! To run behind a host, a context implements the same traits by recording
//! effects: each wait becomes an effect carrying a
//! [`ReplyHandle`](reply::handle::ReplyHandle), and the routine suspends
//! until the host replies through it.
//!
//! ```
//! # use core::ops::ControlFlow;
//! # use sans_effort::run::Run;
//! # trait Console { async fn read_line(&self) -> String; fn write_line(&self, line: String); }
//! # struct Greeter<C: Console> { ctx: C }
//! # impl<C: Console> Run for Greeter<C> {
//! #     async fn step(&mut self) -> ControlFlow<()> {
//! #         self.ctx.write_line("Who are you?".into());
//! #         let name = self.ctx.read_line().await;
//! #         self.ctx.write_line(format!("Hello, {name}!"));
//! #         ControlFlow::Break(())
//! #     }
//! # }
//! use sans_effort::{
//!     driver::{Driver, outbox::Outbox, status::Status},
//!     reply::handle::ReplyHandle,
//! };
//! use std::collections::VecDeque;
//!
//! // The host's vocabulary: an effect per wait, carrying the handle that
//! // answers it, and one per message.
//! enum Effect {
//!     ReadLine(ReplyHandle<String>),
//!     WriteLine(String),
//! }
//!
//! // The reifying context: `ask` builds an effect around a fresh handle and
//! // awaits the reply; `tell` records one and moves on.
//! struct Ctx(Outbox<Effect>);
//!
//! impl Console for Ctx {
//!     async fn read_line(&self) -> String {
//!         self.0.ask(Effect::ReadLine).await
//!     }
//!
//!     fn write_line(&self, line: String) {
//!         self.0.tell(Effect::WriteLine(line));
//!     }
//! }
//!
//! // Driving it from Rust: match on the effects, reply through the handles.
//! let mut driver = Driver::new(|outbox| Greeter { ctx: Ctx(outbox) }.run());
//! let mut queue: VecDeque<Effect> = driver.start().into();
//! let mut written = Vec::new();
//!
//! while let Some(effect) = queue.pop_front() {
//!     match effect {
//!         Effect::WriteLine(text) => written.push(text),
//!         Effect::ReadLine(reply) => queue.extend(driver.reply(reply, "bob".into())),
//!     }
//! }
//!
//! assert_eq!(driver.status(), Status::Complete);
//! assert_eq!(written, ["Who are you?", "Hello, bob!"]);
//! ```
//!
//! This context names the variants of one enum, so it serves one
//! vocabulary. A context generic over _any_ vocabulary — so that the host,
//! not the routine, decides what is on offer, and one context serves every
//! host — describes each wait as a request value and states what the host
//! must carry as a `From` bound. That pattern, a reifying context built on
//! it, and a standard library of capabilities are `sans-effort-effects`.
//!
//! A host in another language cannot hold a `ReplyHandle`; see [`boundary`] and
//! the `sans-effort-host` crate for that path. A host with a runtime needs
//! none of this: implement `Console` with real futures and `tokio::spawn` the
//! routine.
//!
//! # Where the Vocabulary Lives
//!
//! The examples above keep the routine free of any effect type and put the
//! vocabulary in the context. Two other arrangements use the same mechanism
//! and are documented in the exploration this crate came from: the routine
//! may own a closed `Effect` enum and call
//! [`Outbox::ask`](driver::outbox::Outbox::ask) directly (fewest lines; no
//! native path; the enum is the spec), or state its requirements as `From`
//! bounds on the routine itself (host-chosen vocabulary without traits; see
//! `sans-effort-effects`). Pick the traits-and-context arrangement unless you
//! know why you want another.
//!
//! The names, for readers who have them: traits-and-context is the
//! _tagless-final_ style with the representation pinned to `impl Future` —
//! the traits are the algebra, the routine a term abstract in its
//! interpreter, `TokioCtx` the evaluating interpreter, and `Ctx<E>` the
//! reifying one that recovers the initial, tagged encoding. The closed-enum
//! arrangement _is_ that initial encoding. They share one mechanism because
//! the two encodings are interconvertible. Rust readers may know the shape
//! as "capability traits" or MTL-style.
//!
//! # `no_std`
//!
//! This crate is `no_std` + `alloc`. The driver has one lock: `std::sync::Mutex`
//! under the default `std` feature, `spin::Mutex` under `spin`. Targets without
//! native CAS or 64-bit atomics add `portable-atomic` (which implies `spin`) and
//! usually `critical-section`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

pub mod boundary;
pub mod driver;
pub mod join;
pub mod reply;
pub mod run;
pub mod select;
pub mod testing;
