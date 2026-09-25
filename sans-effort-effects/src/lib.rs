//! A standard library of effect traits for `sans-effort` routines.
//!
//! A routine names what it needs as traits; a context decides what each call
//! does. Most routines want the same few things — to sleep, to read and write
//! lines — and without this crate every application writes each of them
//! three times: the trait, the effect and its reifying impl, and a native
//! impl. Here the first two are written once.
//!
//! | Module      | Traits                    |
//! |-------------|---------------------------|
//! | [`time`]    | [`Sleep`](time::Sleep)    |
//! | [`console`] | [`ReadLine`](console::ReadLine), [`WriteLine`](console::WriteLine) |
//! | [`spawn`]   | [`Spawn`](spawn::Spawn)   |
//!
//! Underneath them is [`ask`]: [`Ask`](ask::Ask) and
//! [`Asked`](ask::Asked), the pattern that lets one context serve any
//! host's vocabulary. The mechanism itself only has `ask` and `tell`, where
//! the caller names a variant of one concrete enum; this crate is the layer
//! that makes contexts generic over the enum.
//!
//! Each module holds the trait, its effect structs (in `effect`), and the impl
//! of the trait for [`Ctx`](ctx::Ctx) — the reifying context, which records
//! each call as an effect for a host to perform. `Ctx` is generic over the
//! host's vocabulary `E`, so one impl serves every application: an
//! application writes its vocabulary enum, with a `From` impl per effect it
//! offers, and [`Ctx<E>`](ctx::Ctx) implements exactly the traits that
//! vocabulary can carry.
//!
//! ```
//! use sans_effort_core::{
//!     driver::{Driver, status::Status},
//!     step::Step,
//! };
//! use sans_effort_effects::{
//!     console::{ReadLine, ReadLineError, WriteLine, effect},
//!     ctx::Ctx,
//!     ask::Asked,
//! };
//! use core::ops::ControlFlow;
//!
//! struct Echo<C>(C);
//!
//! impl<C: ReadLine + WriteLine> Step for Echo<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         match self.0.read_line().await {
//!             Ok(line) => self.0.write_line(line),
//!             Err(ReadLineError::Closed) => return ControlFlow::Break(()),
//!             Err(_) => self.0.write_line("?".into()),
//!         }
//!         ControlFlow::Continue(())
//!     }
//! }
//!
//! // The host's vocabulary: which effects it offers.
//! enum Effect {
//!     ReadLine(Asked<effect::ReadLine>),
//!     WriteLine(effect::WriteLine),
//! }
//!
//! impl From<Asked<effect::ReadLine>> for Effect {
//!     fn from(asked: Asked<effect::ReadLine>) -> Self { Effect::ReadLine(asked) }
//! }
//!
//! impl From<effect::WriteLine> for Effect {
//!     fn from(write: effect::WriteLine) -> Self { Effect::WriteLine(write) }
//! }
//!
//! let mut driver = Driver::<Effect>::new(|outbox| Echo(Ctx::new(outbox)).run());
//! let mut input = vec![Ok("hi")].into_iter();
//! let mut written = Vec::new();
//! let mut queue: std::collections::VecDeque<Effect> = driver.resume().into();
//!
//! while let Some(e) = queue.pop_front() {
//!     match e {
//!         Effect::WriteLine(effect::WriteLine(line)) => written.push(line),
//!         Effect::ReadLine(Asked { reply, .. }) => {
//!             let line = input.next().unwrap_or(Err(ReadLineError::Closed));
//!             queue.extend(driver.reply(reply, line.map(String::from)));
//!         }
//!     }
//! }
//!
//! assert_eq!(written, ["hi"]);
//! assert_eq!(driver.status(), Status::Complete);
//! ```
//!
//! An application implements its own effect traits the same way,
//! through [`Ctx::ask`](ctx::Ctx::ask) and
//! [`Ctx::tell`](ctx::Ctx::tell). Every impl in this crate is
//! written over [`AsCtx`](ctx::AsCtx), which `Ctx<E>` implements: `Ctx<E>`
//! has every effect trait, and `AsCtx` exists so a newtype over it can too —
//! one method, and a crate can then reify an effect trait it does not
//! own on its own newtype. Nothing else needs it.
//!
//! # Fallible Where the World Can Fail
//!
//! A trait returns `Result` exactly when its effect can fail for reasons
//! outside the routine: input can end, so [`read_line`](console::ReadLine::read_line)
//! is fallible; nothing a routine could act on makes a sleep fail, so
//! [`sleep`](time::Sleep::sleep) is not. A fallible effect's request names
//! its real answer — `type Reply = Result<String, ReadLineError>` — which
//! crosses as `bytes` in `sans-effort-core`'s encoding; bytes that do not
//! decode as it are refused before the routine sees them.
//!
//! # Spelling
//!
//! Trait methods are `fn … -> impl Future<Output = T> + Send`, as
//! `sans_effort_core::step::Step` spells `step` but with `Send`; implementors
//! write `async fn`. Declaring `Send` is what lets a routine generic over its
//! context prove a child it spawns is `Send`: otherwise each effect trait's
//! future is opaque, and nothing in generic code could say it may cross
//! threads. The price is that a context's futures must be `Send`, so a
//! context is `Sync`: one that holds a value tied to its thread — a
//! `JsValue`, an `Rc` — keeps that value behind a task of its own and talks
//! to it over a channel.
//!
//! # `no_std`
//!
//! This crate is `no_std` + `alloc`. Its `std` and `spin` features only
//! forward `sans-effort-core`'s choice of lock; a crate that names the traits
//! and nothing else should depend with `default-features = false`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ask;
pub mod console;
pub mod ctx;
pub mod spawn;
pub mod time;
