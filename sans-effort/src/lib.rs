//! sans-io, without all the effort: write a routine once as a direct-style
//! `async fn`, then run it natively on an executor or drive it from a foreign
//! host, one typed effect at a time.
//!
//! This crate is the one dependency a routine author needs. It re-exports the
//! crates underneath, which runtime and binding authors can depend on
//! directly:
//!
//! ```text
//!   sans_effort::              from
//!     boundary  driver  join
//!     reply  select  step
//!     testing                  sans-effort-core
//!     ask  console  ctx
//!     spawn  time              sans-effort-effects
//!     host     (feature)       sans-effort-host
//!     tokio    (feature)       sans-effort-tokio
//! ```
//!
//! The core and the standard effect traits share one flat namespace: together
//! they are the vocabulary a routine is written in. A runtime or a binding kit
//! is opted into, and keeps its own module.
//!
//! # Example
//!
//! A routine names only the effect traits it uses, as trait bounds:
//!
//! ```
//! use core::{ops::ControlFlow, time::Duration};
//! use sans_effort::{
//!     console::WriteLine,
//!     step::Step,
//!     time::Sleep,
//! };
//!
//! struct Countdown<C> {
//!     ctx: C,
//!     left: u32,
//! }
//!
//! impl<C: Sleep + WriteLine> Step for Countdown<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         if self.left == 0 {
//!             return ControlFlow::Break(());
//!         }
//!         self.ctx.write_line(format!("{}", self.left));
//!         self.ctx.sleep(Duration::from_millis(10)).await;
//!         self.left -= 1;
//!         ControlFlow::Continue(())
//!     }
//! }
//! ```
//!
//! # Features
//!
//! - `std` (default): the driver's lock is `std::sync::Mutex`.
//! - `spin`: the driver's lock is `spin::Mutex`, for `no_std`.
//! - `portable-atomic`: atomics for targets without native CAS; implies
//!   `spin`.
//! - `critical-section`: `portable-atomic` over the application's
//!   `critical-section`.
//! - `host`: `host`, the machines and the handle table for a binding.
//! - `tokio`: `tokio`, every standard effect trait as a tokio future; implies
//!   `std`.
//!
//! At least one of `std` and `spin` must be enabled somewhere in the build.

#![no_std]

#[doc(inline)]
pub use sans_effort_core::{boundary, driver, join, reply, select, step, testing};

#[doc(inline)]
pub use sans_effort_effects::{ask, console, ctx, spawn, time};

#[cfg(feature = "host")]
#[doc(inline)]
pub use sans_effort_host as host;

#[cfg(feature = "tokio")]
#[doc(inline)]
pub use sans_effort_tokio as tokio;
