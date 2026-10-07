#![doc = include_str!("../README.md")]
//!
//! ## Your Own Effect Traits
//!
//! [`guide`] writes one from start to finish. An application implements its
//! own effect traits the same way,
//! through [`Ctx::ask`](ctx::Ctx::ask) and
//! [`Ctx::tell`](ctx::Ctx::tell). Every impl in this crate is
//! written over [`AsCtx`](ctx::AsCtx), which `Ctx<E>` implements: `Ctx<E>`
//! has every effect trait, and `AsCtx` exists so a newtype over it can too —
//! one method, and a crate can then reify an effect trait it does not
//! own on its own newtype. Nothing else needs it.
//!
//! ## Fallible Where the World Can Fail
//!
//! A trait returns `Result` exactly when its effect can fail for reasons
//! outside the routine: input can end, so [`read_line`](console::ReadLine::read_line)
//! is fallible; nothing a routine could act on makes a sleep fail, so
//! [`sleep`](time::Sleep::sleep) is not. A fallible effect's request names
//! its real answer — `type Reply = Result<String, ReadLineError>` — which
//! crosses as `bytes` in `sans-effort-core`'s encoding; bytes that do not
//! decode as it are refused before the routine sees them.
//!
//! ## Spelling
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
//! Nothing happens until the future is polled. The reifying context records
//! a request only when it is awaited, so a native context must not start a
//! timer, draw bytes, or write a file when the method is called either —
//! otherwise a routine that makes a future and awaits it later behaves
//! differently on each. Writing the method as `async fn` gets this for free.
//!
//! ## Cancellation
//!
//! The table's last column is each trait's _Cancellation_ section in brief:
//! what abandoning a request — the losing side of a
//! [`select`](sans_effort_core::select::select) — can cost. Retractable
//! effects lose nothing; consuming ones may lose their result; committing ones
//! may happen anyway. To wait on a read without risking its line, race it by
//! reference, as [`ReadLine`](console::ReadLine)'s section shows.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ask;
pub mod console;
pub mod ctx;
pub mod env;
pub mod fs;
pub mod guide;
pub mod random;
pub mod spawn;
pub mod time;
