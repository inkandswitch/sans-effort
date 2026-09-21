//! Effect routines: host-driven async coroutines.
//!
//! An _effect routine_ is a coroutine written in direct style as an ordinary
//! `async fn`, whose every wait is a typed effect answered by whoever drives
//! it. The routine has no waker, no executor, and one `Box::pin` at the
//! boundary; the host pulls it forward one step at a time and supplies the
//! answers. It is a sans-io state machine that the compiler writes for you.
//!
//! ```text
//!   host                                    routine
//!     │                                        │
//!     │  step()                                │
//!     │───────────────────────────────────────▶│  runs until it needs input:
//!     │                                        │  tells Write, asks ReadLine·1
//!     │  [Write, ReadLine·1]   AWAITING        │
//!     │◀───────────────────────────────────────│
//!     │                                        │
//!     │  reply(1, "bob")                       │
//!     │───────────────────────────────────────▶│  resumes; asks Lookup·2
//!     │  [Lookup·2]            AWAITING        │
//!     │◀───────────────────────────────────────│
//!     │                                        │
//!     │  reply(2, "Hello")                     │
//!     │───────────────────────────────────────▶│  resumes; tells Write, returns
//!     │  [Write]               COMPLETE        │
//!     │◀───────────────────────────────────────│
//!     │                                        ┴
//! ```
//!
//! # The pieces
//!
//! - [`run::Run`] is the shape of a routine: a [`turn`](run::Run::turn) that
//!   is one iteration of its loop, and a [`run`](run::Run::run) that repeats
//!   it until it breaks.
//! - [`post::Post`] is what a routine writes into: [`tell`](post::Post::tell)
//!   an effect and move on, or [`ask`](post::Post::ask) one and await the
//!   reply. Every wait goes through it.
//! - [`reply::ReplyHandle`] is the typed, single-use capability to answer one
//!   `ask`. It travels inside the effect to whoever performs it.
//! - [`driver::Driver`] turns a routine into something a host can step:
//!   [`start`](driver::Driver::start), then [`reply`](driver::Driver::reply)
//!   with each handle the effects hand back, until it is finished.
//! - [`wire`] is what crosses a boundary: the reply menu, and the traits an
//!   effect type implements to be shown to a host that cannot hold a Rust
//!   value.
//!
//! # Writing one
//!
//! The routine owns a closed effect type and asks for each wait by name.
//! Variants that carry a [`ReplyHandle`](reply::ReplyHandle) await a reply of
//! that type; the rest are fire-and-forget.
//!
//! ```
//! use core::ops::ControlFlow;
//! use effect_routine::{post::Post, reply::ReplyHandle, run::Run};
//!
//! enum Effect {
//!     ReadLine(ReplyHandle<String>),
//!     Write(String),
//! }
//!
//! struct Greeter<O: Post<Effect>> {
//!     outbox: O,
//! }
//!
//! impl<O: Post<Effect>> Run for Greeter<O> {
//!     async fn turn(&mut self) -> ControlFlow<()> {
//!         self.outbox.tell(Effect::Write("Who are you?".into()));
//!         let name = self.outbox.ask(Effect::ReadLine).await;
//!         self.outbox.tell(Effect::Write(format!("Hello, {name}!")));
//!         ControlFlow::Break(())
//!     }
//! }
//! ```
//!
//! # Driving one
//!
//! A Rust host matches on the effects and replies through the handle. This
//! is the whole host loop for the routine above:
//!
//! ```
//! # use core::ops::ControlFlow;
//! # use effect_routine::{post::Post, reply::ReplyHandle, run::Run};
//! # enum Effect { ReadLine(ReplyHandle<String>), Write(String) }
//! # struct Greeter<O: Post<Effect>> { outbox: O }
//! # impl<O: Post<Effect>> Run for Greeter<O> {
//! #     async fn turn(&mut self) -> ControlFlow<()> {
//! #         self.outbox.tell(Effect::Write("Who are you?".into()));
//! #         let name = self.outbox.ask(Effect::ReadLine).await;
//! #         self.outbox.tell(Effect::Write(format!("Hello, {name}!")));
//! #         ControlFlow::Break(())
//! #     }
//! # }
//! use effect_routine::driver::{Driver, Status};
//! use std::collections::VecDeque;
//!
//! let mut driver = Driver::new(|outbox| Greeter { outbox }.run());
//! let mut queue: VecDeque<Effect> = driver.start().into();
//! let mut written = Vec::new();
//!
//! // Each reply may return more effects; queue them so a batch with two
//! // requests outstanding is handled the same as a batch with one.
//! while let Some(effect) = queue.pop_front() {
//!     match effect {
//!         Effect::Write(text) => written.push(text),
//!         Effect::ReadLine(reply) => queue.extend(driver.reply(reply, "bob".into())),
//!     }
//! }
//!
//! assert!(driver.is_finished());
//! assert_eq!(driver.status(), Status::Complete);
//! assert_eq!(written, ["Who are you?", "Hello, bob!"]);
//! ```
//!
//! A host in another language cannot hold a `ReplyHandle`; see [`wire`] and
//! the `effect_routine_host` crate for that path.
//!
//! # Three styles, one mechanism
//!
//! The example above is the _effects_ style: the routine owns the enum, and
//! the enum is also the wire. Two others use exactly the same mechanism:
//!
//! - _coeffects_: each wait is a [`request::Request`] struct, and the routine
//!   states what its host must carry as `E: From<Asked<Lookup>>` bounds, so
//!   several routines can share one host and a host can attenuate.
//! - _capabilities_: the routine asks for traits (`C: Clock + Directory`) and
//!   never sees an effect; a _context_ implements them either natively or by
//!   posting effects. The routine can then also run as a plain `async fn`.
//!
//! Pick effects unless you need one of those two properties.
//!
//! # `no_std`
//!
//! This crate is `no_std` + `alloc`. The `std` feature swaps the driver's one
//! lock from `spin` to `std::sync::Mutex`. Targets without native CAS or
//! 64-bit atomics enable `portable-atomic` (and usually `critical-section`).

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

pub mod driver;
pub mod join;
pub mod post;
pub mod reply;
pub mod request;
pub mod run;
pub mod wire;
