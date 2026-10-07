#![doc = include_str!("../README.md")]
//!
//! ## Where the Vocabulary Lives
//!
//! The example above keeps the routine free of any effect type and puts the
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

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

pub mod boundary;
pub mod driver;
pub mod join;
pub mod reply;
pub mod select;
pub mod step;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
