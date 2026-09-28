//! Files, read and written whole.
//!
//! Paths are UTF-8 strings, interpreted by the host: `no_std` has no `Path`,
//! and a routine should not assume one host's path syntax anyway.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
use alloc::{string::String, vec::Vec};
use core::future::Future;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Read a file whole.
pub trait ReadFile {
    /// The contents of the file at `path`.
    ///
    /// # Errors
    ///
    /// [`FsError`] if the file cannot be read.
    fn read_file(&self, path: String) -> impl Future<Output = Result<Vec<u8>, FsError>> + Send;
}

/// Write a file whole.
pub trait WriteFile {
    /// Replace the contents of the file at `path` with `bytes`, creating it
    /// if it does not exist. Its directory must.
    ///
    /// # Errors
    ///
    /// [`FsError`] if the file cannot be written.
    fn write_file(
        &self,
        path: String,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), FsError>> + Send;
}

impl<C: AsCtx + Sync> ReadFile for C
where
    C::Vocabulary: From<Asked<ReadFileEffect>> + Send,
{
    async fn read_file(&self, path: String) -> Result<Vec<u8>, FsError> {
        self.ctx().ask(ReadFileEffect(path)).await
    }
}

impl<C: AsCtx + Sync> WriteFile for C
where
    C::Vocabulary: From<Asked<WriteFileEffect>> + Send,
{
    async fn write_file(&self, path: String, bytes: Vec<u8>) -> Result<(), FsError> {
        self.ctx().ask(WriteFileEffect { path, bytes }).await
    }
}

/// A file's contents. Awaits a `Result<Vec<u8>, FsError>`, which crosses as
/// `bytes`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadFileEffect(pub String);

impl Ask for ReadFileEffect {
    type Reply = Result<Vec<u8>, FsError>;
}

/// The path, as a `str`.
impl Encode for ReadFileEffect {
    fn encode(&self, w: &mut Writer) {
        w.str(&self.0);
    }
}

impl Decode for ReadFileEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.str().map(ReadFileEffect)
    }
}

/// New contents for a file. Awaits a `Result<(), FsError>`, which crosses as
/// `bytes`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteFileEffect {
    /// Where.
    pub path: String,
    /// What.
    pub bytes: Vec<u8>,
}

impl Ask for WriteFileEffect {
    type Reply = Result<(), FsError>;
}

/// The path as a `str`, then the contents as `bytes`.
impl Encode for WriteFileEffect {
    fn encode(&self, w: &mut Writer) {
        w.str(&self.path);
        w.bytes(&self.bytes);
    }
}

impl Decode for WriteFileEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let path = r.str()?;
        let bytes = r.bytes()?.to_vec();
        Ok(WriteFileEffect { path, bytes })
    }
}

/// Why a file could not be read or written.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum FsError {
    /// Nothing is at the path, or its directory does not exist.
    #[error("not found")]
    NotFound,

    /// The host may not read or write the path.
    #[error("permission denied")]
    PermissionDenied,

    /// Anything else the host could not do.
    #[error("file operation failed")]
    Other,
}

/// One tag: `0` not found, `1` permission denied, `2` anything else.
impl Encode for FsError {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            FsError::NotFound => 0,
            FsError::PermissionDenied => 1,
            FsError::Other => 2,
        });
    }
}

impl Decode for FsError {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(FsError::NotFound),
            1 => Ok(FsError::PermissionDenied),
            2 => Ok(FsError::Other),
            tag => Err(DecodeError::UnknownTag { tag }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        bolero::check!()
            .with_type::<(String, Vec<u8>)>()
            .for_each(|(path, bytes)| {
                let read = ReadFileEffect(path.clone());
                assert_eq!(ReadFileEffect::from_bytes(&read.to_bytes()), Ok(read));
                let write = WriteFileEffect {
                    path: path.clone(),
                    bytes: bytes.clone(),
                };
                assert_eq!(WriteFileEffect::from_bytes(&write.to_bytes()), Ok(write));
            });
    }

    #[test]
    fn errors_round_trip() {
        for e in [FsError::NotFound, FsError::PermissionDenied, FsError::Other] {
            assert_eq!(FsError::from_bytes(&e.to_bytes()), Ok(e));
        }
    }
}
