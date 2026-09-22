//! Effect routines: host-driven async coroutines.
//!
//! An _effect routine_ is a coroutine written in direct style as an ordinary
//! `async fn`, whose every wait is a typed effect answered by whoever drives
//! it. The routine has no waker, no executor, and one `Box::pin` at the
//! boundary; a host resumes it one wait at a time and supplies the answers.
//! It is a sans-io state machine that the compiler writes for you.
//!
//! ```text
//!   host                                    routine
//!     │                                        │
//!     │  start()                               │
//!     │───────────────────────────────────────▶│  runs until it needs input:
//!     │                                        │  tells WriteLine, asks ReadLine·1
//!     │  [WriteLine, ReadLine·1]  AWAITING     │
//!     │◀───────────────────────────────────────│
//!     │                                        │
//!     │  reply(1, "bob")                       │
//!     │───────────────────────────────────────▶│  resumes; asks Lookup·2
//!     │  [Lookup·2]            AWAITING        │
//!     │◀───────────────────────────────────────│
//!     │                                        │
//!     │  reply(2, "Hello")                     │
//!     │───────────────────────────────────────▶│  resumes; tells WriteLine, returns
//!     │  [WriteLine]           COMPLETE        │
//!     │◀───────────────────────────────────────│
//!     │                                        ┴
//! ```
//!
//! # Three layers
//!
//! The routine asks for _traits_ and never sees an effect. A _context_
//! implements those traits, and decides what each call does: a real future
//! on a runtime, or an effect recorded for a host. The _host_ is whoever
//! polls — tokio, or a foreign program over a [`Driver`](driver::Driver).
//!
//! ```text
//!   ┌─────────────────────────────────────────────────────────────────┐
//!   │ routine     Greeter<C: Sleep + Lookup + Console>: Run           │  no_std
//!   │             owns the logic; knows nothing of effects or hosts   │
//!   ├─────────────────────────────────────────────────────────────────┤
//!   │ context     impl Sleep for TokioCtx   │ impl Sleep for Ctx<E>   │
//!   │             a real future             │ record an effect, wait  │
//!   ├───────────────────────────────────────┼─────────────────────────┤
//!   │ host        tokio polls the task      │ a Driver polls; Python, │
//!   │             directly — no driver      │ JS, a test… performs    │
//!   └───────────────────────────────────────┴─────────────────────────┘
//! ```
//!
//! The left column is why you write the routine this way: it is also a plain
//! `async fn`, usable at native speed by code that has never heard of this
//! crate. The right column is what this crate provides.
//!
//! # The pieces
//!
//! - [`run::Run`] is the shape of a routine: a [`step`](run::Run::step) that
//!   is one iteration of its loop, and a [`run`](run::Run::run) that repeats
//!   it until it breaks.
//! - [`driver::outbox::Outbox`] is what a _reifying context_ writes into:
//!   [`tell`](driver::outbox::Outbox::tell) an effect and move on, or
//!   [`ask`](driver::outbox::Outbox::ask) one and await the reply.
//! - [`reply::handle::ReplyHandle`] is the typed, single-use capability to answer one
//!   `ask`. It travels inside the effect to whoever performs it.
//! - [`request::Request`] lets a wait be a value — `Lookup(name)` — so a
//!   context can be generic over the host's vocabulary, and a host can offer
//!   a routine less than everything.
//! - [`driver::Driver`] turns a routine into something a host can resume:
//!   [`start`](driver::Driver::start), then [`reply`](driver::Driver::reply)
//!   with each handle the effects hand back, until it is finished.
//! - [`boundary`] is what crosses a boundary: the reply menu, and the traits an
//!   effect type implements to be shown to a host that cannot hold a Rust
//!   value.
//! - [`testing::run_now`] runs a routine against a mock context whose every
//!   future is ready, in one poll, with no driver.
//!
//! # Writing one
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
//! # A reifying context
//!
//! To run behind a host, a context implements the same traits by recording
//! effects. Each wait is a [`Request`](request::Request); the context is
//! generic over any vocabulary `E` that can carry it, so the host — not the
//! routine — decides what is on offer.
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
//! use sans_effort::{driver::outbox::Outbox, request::{Asked, Request}};
//!
//! // The host vocabulary: one struct per wait, one per message.
//! struct ReadLine;
//! struct WriteLine(String);
//!
//! impl Request for ReadLine {
//!     type Reply = String;
//! }
//!
//! // The reifying context: `Console` holds for any `E` that carries both.
//! struct Ctx<E> {
//!     outbox: Outbox<E>,
//! }
//!
//! impl<E: From<Asked<ReadLine>> + From<WriteLine>> Console for Ctx<E> {
//!     async fn read_line(&self) -> String {
//!         self.outbox.request(ReadLine).await
//!     }
//!
//!     fn write_line(&self, line: String) {
//!         self.outbox.notify(WriteLine(line));
//!     }
//! }
//!
//! // A host's vocabulary, and the two `From` impls that admit it.
//! enum Effect {
//!     ReadLine(Asked<ReadLine>),
//!     WriteLine(WriteLine),
//! }
//!
//! impl From<Asked<ReadLine>> for Effect {
//!     fn from(asked: Asked<ReadLine>) -> Self { Effect::ReadLine(asked) }
//! }
//!
//! impl From<WriteLine> for Effect {
//!     fn from(write: WriteLine) -> Self { Effect::WriteLine(write) }
//! }
//!
//! // Driving it from Rust: match on the effects, reply through the handles.
//! use sans_effort::driver::{Driver, status::Status};
//! use std::collections::VecDeque;
//!
//! let mut driver = Driver::<Effect>::new(|outbox| Greeter { ctx: Ctx { outbox } }.run());
//! let mut queue: VecDeque<Effect> = driver.start().into();
//! let mut written = Vec::new();
//!
//! while let Some(effect) = queue.pop_front() {
//!     match effect {
//!         Effect::WriteLine(WriteLine(text)) => written.push(text),
//!         Effect::ReadLine(Asked { reply, .. }) => {
//!             queue.extend(driver.reply(reply, "bob".into()));
//!         }
//!     }
//! }
//!
//! assert_eq!(driver.status(), Status::Complete);
//! assert_eq!(written, ["Who are you?", "Hello, bob!"]);
//! ```
//!
//! A host in another language cannot hold a `ReplyHandle`; see [`boundary`] and
//! the `sans_effort_host` crate for that path. A host with a runtime needs
//! none of this: implement `Console` with real futures and `tokio::spawn` the
//! routine.
//!
//! # Where the vocabulary lives
//!
//! The example above keeps the routine free of any effect type, and puts the
//! vocabulary in the context, chosen by the host. Two other arrangements use
//! the same mechanism and are documented in the exploration this crate came
//! from: the routine may own a closed `Effect` enum and call
//! [`Outbox::ask`](driver::outbox::Outbox::ask) directly (fewest lines; no native path; the
//! enum is the spec), or state its requirements as `E: From<Asked<…>>` bounds
//! on the routine itself (host-chosen vocabulary without traits). Pick the
//! traits-and-context arrangement unless you know why you want another.
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
pub mod request;
pub mod run;
pub mod testing;
