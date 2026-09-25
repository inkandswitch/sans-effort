//! What [`Count`](super::Count) and [`Lookup`](super::Lookup) record under a
//! reifying context.

use alloc::string::String;
use sans_effort::ask::Ask;

/// How many greetings so far. Awaits a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count;

/// The greeting word for a name. Awaits a `str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup(pub String);

impl Ask for Count {
    type Reply = u64;
}

impl Ask for Lookup {
    type Reply = String;
}
