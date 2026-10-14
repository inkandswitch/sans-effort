//! Routines that misbehave on purpose, for testing hosts — not examples to
//! follow.
//!
//! Every other routine here shows a way to write one. These exist so that
//! `demo:faults` can check what each host does when a routine goes wrong:
//! that it reports the problem instead of hanging, and that every host
//! reports it the same way.

pub mod deadlock;
