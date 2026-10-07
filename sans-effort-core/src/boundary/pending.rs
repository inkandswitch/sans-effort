//! An outstanding request, by the kind of reply it takes.

use crate::reply::handle::ReplyHandle;
use crate::reply::kind::Kind;
use alloc::{string::String, vec::Vec};

/// One variant per entry of the menu; a host replies with the matching kind,
/// and the host layer type-checks it here.
#[derive(Debug, PartialEq, Eq)]
pub enum Pending {
    /// Awaits [`Value::Bytes`](crate::reply::value::Value::Bytes).
    Bytes(ReplyHandle<Vec<u8>>),
    /// Awaits [`Value::Str`](crate::reply::value::Value::Str).
    Str(ReplyHandle<String>),
    /// Awaits [`Value::U64`](crate::reply::value::Value::U64).
    U64(ReplyHandle<u64>),
    /// Awaits [`Value::Unit`](crate::reply::value::Value::Unit).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_reply_keeps_its_request_id_and_kind() {
        bolero::check!().with_type::<u64>().for_each(|id| {
            let pending = [
                Pending::Bytes(ReplyHandle::mint(1, *id)),
                Pending::Str(ReplyHandle::mint(1, *id)),
                Pending::U64(ReplyHandle::mint(1, *id)),
                Pending::Unit(ReplyHandle::mint(1, *id)),
            ];
            assert_eq!(pending.each_ref().map(Pending::id), [*id; 4]);
            assert_eq!(
                pending.each_ref().map(Pending::kind),
                [Kind::Bytes, Kind::Str, Kind::U64, Kind::Unit]
            );
        });
    }
}
