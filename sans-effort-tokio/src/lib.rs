#![doc = include_str!("../README.md")]
//!
//! ## Your Own Effect Traits
//!
//! A routine takes one context, which must provide every effect trait it
//! names. An application with effect traits of its own wraps a `TokioCtx` in
//! a type of its own, implements its traits there, and forwards the stdlib
//! ones — one line each:
//!
//! ```
//! use core::{future::Future, time::Duration};
//! use sans_effort_effects::time::Sleep;
//! use sans_effort_tokio::ctx::TokioCtx;
//!
//! struct AppCtx<R, W> {
//!     tokio: TokioCtx<R, W>,
//! }
//!
//! impl<R, W> Sleep for AppCtx<R, W> {
//!     fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
//!         self.tokio.sleep(duration)
//!     }
//! }
//! ```
//!
//! The orphan rule is why: `impl Count for TokioCtx` would be a foreign
//! trait on a foreign type unless the application defines one of them, so
//! the impls go on a type it owns. A reifying context avoids the forwarding
//! through `sans_effort_effects::ctx::AsCtx`; there is no native equivalent,
//! because blanket impls of the stdlib's traits can only live in the stdlib.

pub mod clock;
pub mod console;
pub mod ctx;
pub mod env;
pub mod fs;
pub mod random;
pub mod spawn;
