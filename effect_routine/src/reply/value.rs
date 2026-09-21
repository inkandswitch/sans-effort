//! A reply value as the mailbox and the ABI carry it.

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

/// Which entry of the menu a slot expects. Recorded when a request opens,
/// checked when a value is delivered, so a mismatch is caught at the mailbox
/// and never reaches the routine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// [`Value::Bytes`].
    Bytes,
    /// [`Value::Str`].
    Str,
    /// [`Value::U64`].
    U64,
    /// [`Value::Unit`].
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
