//! Files, through `tokio::fs`.

use sans_effort_effects::fs::{FsError, ReadFile, WriteFile};
use std::io;

/// `ReadFile` and `WriteFile` as `tokio::fs::read` and `tokio::fs::write`,
/// with paths as the process sees them.
///
/// Two traits on one component: a context that should read but not write
/// forwards only [`ReadFile`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioFs;

impl ReadFile for TokioFs {
    async fn read_file(&self, path: String) -> Result<Vec<u8>, FsError> {
        tokio::fs::read(path).await.map_err(|e| fs_error(&e))
    }
}

impl WriteFile for TokioFs {
    async fn write_file(&self, path: String, bytes: Vec<u8>) -> Result<(), FsError> {
        tokio::fs::write(path, bytes)
            .await
            .map_err(|e| fs_error(&e))
    }
}

/// What a routine can act on, of everything the OS might say.
fn fs_error(error: &io::Error) -> FsError {
    let kind = error.kind();
    if kind == io::ErrorKind::NotFound {
        FsError::NotFound
    } else if kind == io::ErrorKind::PermissionDenied {
        FsError::PermissionDenied
    } else {
        FsError::Other
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;

    #[test]
    fn what_the_os_says_becomes_what_a_routine_can_act_on() {
        for (kind, error) in [
            (io::ErrorKind::NotFound, FsError::NotFound),
            (io::ErrorKind::PermissionDenied, FsError::PermissionDenied),
            (io::ErrorKind::AlreadyExists, FsError::Other),
            (io::ErrorKind::InvalidData, FsError::Other),
        ] {
            assert_eq!(fs_error(&io::Error::from(kind)), error, "{kind:?}");
        }
    }

    #[tokio::test]
    async fn written_then_read_back_and_a_missing_file_is_not_found() {
        let dir = std::env::temp_dir().join(format!("sans-effort-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("file").to_string_lossy().into_owned();

        assert_eq!(
            TokioFs.read_file(path.clone()).await,
            Err(FsError::NotFound)
        );
        TokioFs
            .write_file(path.clone(), b"hello".to_vec())
            .await
            .expect("written");
        assert_eq!(TokioFs.read_file(path).await, Ok(b"hello".to_vec()));

        std::fs::remove_dir_all(dir).expect("cleaned up");
    }
}
