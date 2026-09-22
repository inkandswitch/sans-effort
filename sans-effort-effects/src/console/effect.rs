//! What [`ReadLine`](super::ReadLine) and [`WriteLine`](super::WriteLine)
//! record under a reifying context.

use super::ReadLineError;
use alloc::{string::String, vec::Vec};
use sans_effort::{
    boundary::codec::{Decode, DecodeError, Encode, Reader, Writer},
    request::Request,
};

/// The next line of input. Awaits `bytes`: an encoded
/// `Result<String, ReadLineError>`, which [`ReadLine::reply`] builds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadLine;

impl ReadLine {
    /// The reply for a line read, or for why none was: what a host sends
    /// back through this request's handle.
    #[must_use]
    pub fn reply(line: Result<&str, ReadLineError>) -> Vec<u8> {
        line.to_bytes()
    }
}

impl Request for ReadLine {
    type Reply = Vec<u8>;
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

/// Show a line. Fire-and-forget; not a [`Request`].
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
