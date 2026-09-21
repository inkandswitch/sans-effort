//! Effect routines: host-driven async coroutines.
//!
//! An _effect routine_ is an isolated coroutine written in direct style as an
//! ordinary `async fn`, whose every wait is a typed effect answered by whoever
//! drives it. The program has no waker, no executor, and no `Pin`; the host
//! pulls it forward one step at a time and supplies the answers.
//!
//! ```text
//! host                                  routine
//! step(0)             ──────▶           emits Write, awaits ReadLine·1
//! ◀── [Write, ReadLine·1], AWAITING
//! answer 1 "bob"      ──────▶           resumes; emits Lookup·2
//! ```
//!
//! This crate is `no_std` by default; enable the `std` feature for
//! `std`-backed drivers.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "std")]
extern crate std;
