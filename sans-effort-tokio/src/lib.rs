//! Native tokio contexts for `sans-effort` routines.
//!
//! A routine names the capabilities it needs as traits; a context decides
//! what each call does. This crate is the native side: every capability in
//! `sans-effort-effects` as a real tokio future, so a routine runs as an
//! ordinary task — `tokio::spawn(routine.run())` — with no driver and no
//! host.
//!
//! | Type                                 | Implements         |
//! |--------------------------------------|--------------------|
//! | [`TokioClock`](clock::TokioClock)    | `Sleep`            |
//! | [`TokioInput`](console::TokioInput)  | `ReadLine`         |
//! | [`TokioOutput`](console::TokioOutput) | `WriteLine`       |
//! | [`TokioSpawner`](spawn::TokioSpawner) | `Spawn`, for the contexts built on it |
//! | [`TokioCtx`](ctx::TokioCtx)          | all of the above   |
//!
//! [`TokioCtx`](ctx::TokioCtx) is the ready-made context: one value with
//! every capability, built from the components — one component per
//! capability. The components are there to compose differently, and to
//! grant no more than a routine needs: a clock and output for a routine that
//! never reads, or a paused clock with an in-memory output in a test.
//!
//! ```
//! use sans_effort_effects::console::WriteLine;
//! use sans_effort_tokio::ctx::TokioCtx;
//! use tokio_util::task::LocalPoolHandle;
//!
//! let ctx = TokioCtx::new(&b""[..], Vec::new(), LocalPoolHandle::new(1));
//! ctx.write_line("hello".into());
//! let Ok((_, written)) = ctx.into_parts() else { unreachable!("no child shares it") };
//! assert_eq!(written, b"hello\n");
//! ```
//!
//! # Your Own Capabilities
//!
//! A routine takes one context, which must provide every capability it
//! names. An application with capabilities of its own wraps a `TokioCtx` in
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
pub mod spawn;
