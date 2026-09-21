//! An outstanding request, by the kind of reply it takes.

use super::menu::Kind;
use crate::reply::ReplyHandle;
use alloc::{string::String, vec::Vec};

/// One variant per entry of the menu; a host replies with the matching kind,
/// and the host layer type-checks it here.
#[derive(Debug, PartialEq, Eq)]
pub enum Pending {
    /// Awaits [`Value::Bytes`](super::menu::Value::Bytes).
    Bytes(ReplyHandle<Vec<u8>>),
    /// Awaits [`Value::Str`](super::menu::Value::Str).
    Str(ReplyHandle<String>),
    /// Awaits [`Value::U64`](super::menu::Value::U64).
    U64(ReplyHandle<u64>),
    /// Awaits [`Value::Unit`](super::menu::Value::Unit).
    Unit(ReplyHandle<()>),
}

impl Pending {
    /// The request id of the handle inside.
    #[must_use]
    pub const fn id(&self) -> u64 {
        match self {
            Pending::Bytes(r) => r.id(),
            Pending::Str(r) => r.id(),
            Pending::U64(r) => r.id(),
            Pending::Unit(r) => r.id(),
        }
    }

    /// Which entry of the menu the request awaits.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Pending::Bytes(_) => Kind::Bytes,
            Pending::Str(_) => Kind::Str,
            Pending::U64(_) => Kind::U64,
            Pending::Unit(_) => Kind::Unit,
        }
    }
}
