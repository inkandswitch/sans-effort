//! What [`ReadLine`](super::ReadLine) and [`WriteLine`](super::WriteLine)
//! record under a reifying context.

use super::ReadLineError;
use crate::ask::Ask;
use alloc::string::String;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// The next line of input. Awaits a `Result<String, ReadLineError>`, which
/// crosses as `bytes`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadLine;

impl Ask for ReadLine {
    type Reply = Result<String, ReadLineError>;
}

/// No fields.
impl Encode for ReadLine {
    fn encode(&self, _: &mut Writer) {}
}

impl Decode for ReadLine {
    fn decode(_: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ReadLine)
    }
}

/// Show a line. Fire-and-forget; not a [`Ask`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteLine(pub String);

/// The line, as a `str`.
impl Encode for WriteLine {
    fn encode(&self, w: &mut Writer) {
        w.str(&self.0);
    }
}

impl Decode for WriteLine {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.str().map(WriteLine)
    }
}
