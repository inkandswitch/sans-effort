//! One decoded reply record: what the host is telling the routine.

use crate::error::Error;
use effect_routine::boundary::codec::Reader;

pub(super) enum Input {
    Bytes(u64, Vec<u8>),
    Str(u64, String),
    U64(u64, u64),
    Unit(u64),
}

impl Input {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let bad = Error::BadInput;

        let input = match r.u8().ok_or(bad)? {
            1 => Input::Str(r.u64().ok_or(bad)?, r.str().ok_or(bad)?),
            2 => Input::U64(r.u64().ok_or(bad)?, r.u64().ok_or(bad)?),
            3 => Input::Unit(r.u64().ok_or(bad)?),
            4 => Input::Bytes(r.u64().ok_or(bad)?, r.bytes().ok_or(bad)?.to_vec()),
            _ => return Err(bad),
        };

        if r.is_empty() { Ok(input) } else { Err(bad) }
    }
}
