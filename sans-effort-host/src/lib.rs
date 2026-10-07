#![doc = include_str!("../README.md")]
//!
//! ## Where the Boundary Is Split
//!
//! The [`HostEffect`](sans_effort_core::boundary::host_effect::HostEffect) and
//! [`Encode`](sans_effort_core::boundary::codec::Encode) traits,
//! [`Pending`](sans_effort_core::boundary::pending::Pending), and the codec
//! live in [`sans_effort_core::boundary`]; the reply menu
//! ([`Value`](sans_effort_core::reply::value::Value),
//! [`Kind`](sans_effort_core::reply::kind::Kind)) in
//! [`sans_effort_core::reply`]. Both are `no_std`, so a routine's boundary
//! crate may implement them. This crate decodes reply records (`1 id str`;
//! `2 id u64`; `3 id`; `4 id bytes`), checks the kind, and keeps the table.
//! `ABI.md` at the repository root is the contract in full.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod contract;
pub mod encoded;
pub mod error;
pub mod machine;

#[cfg(feature = "std")]
pub mod record;
#[cfg(feature = "std")]
pub mod table;

#[cfg(test)]
mod fixtures;
