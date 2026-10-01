#![doc = include_str!("../README.md")]
#![no_std]

#[doc(inline)]
pub use sans_effort_core::{boundary, driver, join, reply, select, step};

#[cfg(feature = "testing")]
#[doc(inline)]
pub use sans_effort_core::testing;

#[doc(inline)]
pub use sans_effort_effects::{ask, console, ctx, env, fs, random, spawn, time};

#[cfg(feature = "host")]
#[doc(inline)]
pub use sans_effort_host as host;

#[cfg(feature = "tokio")]
#[doc(inline)]
pub use sans_effort_tokio as tokio;
