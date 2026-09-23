//! A standard library of capabilities for `sans-effort` routines.
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
//!
//! Underneath them is [`request`]: [`Request`](request::Request) and
//! [`Asked`](request::Asked), the pattern that lets one context serve any
//! host's vocabulary. The mechanism itself only has `ask` and `tell`, where
//! the caller names a variant of one concrete enum; this crate is the layer
//! that makes contexts generic over the enum.
//!
//! Each module holds the trait, its effect structs (in `effect`), and the impl
//! of the trait for [`Ctx`] — the reifying context, which records each call
//! as an effect for a host to perform. [`Ctx`] is generic over the host's
//! vocabulary `E`, so one impl serves every application: an application
//! writes its vocabulary enum, with a `From` impl per effect it offers, and
//! [`Ctx<E>`](Ctx) implements exactly the traits that vocabulary can carry.
//!
//! ```
//! use sans_effort::{
//!     driver::{Driver, status::Status},
//!     run::Run,
//! };
//! use sans_effort_effects::{
//!     Ctx,
//!     console::{ReadLine, ReadLineError, WriteLine, effect},
//!     request::Asked,
//! };
//! use core::ops::ControlFlow;
//!
//! struct Echo<C>(C);
//!
//! impl<C: ReadLine + WriteLine> Run for Echo<C> {
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
//! let mut queue: std::collections::VecDeque<Effect> = driver.start().into();
//!
//! while let Some(e) = queue.pop_front() {
//!     match e {
//!         Effect::WriteLine(effect::WriteLine(line)) => written.push(line),
//!         Effect::ReadLine(Asked { reply, .. }) => {
//!             let line = input.next().unwrap_or(Err(ReadLineError::Closed));
//!             queue.extend(driver.reply(reply, effect::ReadLine::reply(line)));
//!         }
//!     }
//! }
//!
//! assert_eq!(written, ["hi"]);
//! assert_eq!(driver.status(), Status::Complete);
//! ```
//!
//! An application's own capabilities implement their traits for the same
//! [`Ctx`], through [`Ctx::request`] and [`Ctx::notify`].
//!
//! # Fallible where the world can fail
//!
//! A trait returns `Result` exactly when its effect can fail for reasons
//! outside the routine: input can end, so [`read_line`](console::ReadLine::read_line)
//! is fallible; nothing a routine could act on makes a sleep fail, so
//! [`sleep`](time::Sleep::sleep) is not. A fallible effect's reply crosses as
//! `bytes`, encoded with `sans-effort`'s convention for `Result`.
//!
//! # Spelling
//!
//! Trait methods are `fn … -> impl Future`, as `sans_effort::run::Run`
//! spells `step`; implementors write `async fn`. No trait says anything about
//! `Send`: whether a routine can cross threads is decided where its context
//! is concrete (`Driver::new`, `tokio::spawn`), so a `!Send` context — an
//! `Rc`-based test mock — works wherever nothing asks.
//!
//! # `no_std`
//!
//! This crate is `no_std` + `alloc`. Its `std` and `spin` features only
//! forward `sans-effort`'s choice of lock; a crate that names the traits and
//! nothing else should depend with `default-features = false`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod console;
pub mod ctx;
pub mod request;
pub mod time;

pub use ctx::Ctx;
