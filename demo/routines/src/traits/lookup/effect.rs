//! What [`Lookup`](super::Lookup) records under a reifying context.

use alloc::string::String;
use sans_effort::ask::Ask;

/// The greeting word for a name. Awaits a `str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup(pub String);

impl Ask for Lookup {
    type Reply = String;
}
