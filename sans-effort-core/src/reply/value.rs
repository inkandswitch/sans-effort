//! A reply value as the mailbox and the ABI carry it.

use super::kind::Kind;
use alloc::{string::String, vec::Vec};

/// A reply value as the mailbox and the ABI carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Anything else, encoded by the host and decoded on the routine's side.
    Bytes(Vec<u8>),
    /// Text.
    Str(String),
    /// Counts, ids, timestamps, lengths.
    U64(u64),
    /// An acknowledgement: a sleep, an ack.
    Unit,
}

impl Value {
    /// Which entry of the menu this is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Value::Bytes(_) => Kind::Bytes,
            Value::Str(_) => Kind::Str,
            Value::U64(_) => Kind::U64,
            Value::Unit => Kind::Unit,
        }
    }
}
