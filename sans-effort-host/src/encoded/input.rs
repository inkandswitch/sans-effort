//! One decoded reply record: what the host is telling the routine.

use alloc::{string::String, vec::Vec};
use sans_effort_core::boundary::codec::{DecodeError, Reader};

pub(super) enum Input {
    Bytes(u64, Vec<u8>),
    Str(u64, String),
    U64(u64, u64),
    Unit(u64),
}

impl Input {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);

        let input = match r.u8()? {
            1 => Input::Str(r.u64()?, r.str()?),
            2 => Input::U64(r.u64()?, r.u64()?),
            3 => Input::Unit(r.u64()?),
            4 => Input::Bytes(r.u64()?, r.bytes()?.to_vec()),
            tag => return Err(DecodeError::UnknownTag { tag }),
        };

        r.finish()?;
        Ok(input)
    }
}
