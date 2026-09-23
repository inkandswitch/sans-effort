//! The console: lines in from an async reader, lines out to a writer.
//!
//! Two components, one per capability, so a context can grant output
//! without input or the other way round.

use sans_effort_effects::console::{ReadLine, ReadLineError, WriteLine};
use std::{
    io,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// `ReadLine` from an async buffered reader: stdin, a byte slice in a test,
/// a socket.
pub struct TokioInput<R> {
    /// `read_line` needs `&mut R`; the trait takes `&self` so that a routine
    /// can `join` two waits on one context. The mutex bridges the two. It
    /// must be tokio's, not `std`'s: the guard is held across `.await`, and a
    /// `std` guard there would make the future `!Send`.
    reader: tokio::sync::Mutex<R>,
}

impl<R> TokioInput<R> {
    /// Input read line by line from `reader`.
    #[must_use]
    pub const fn new(reader: R) -> Self {
        Self {
            reader: tokio::sync::Mutex::const_new(reader),
        }
    }

    /// The reader back.
    #[must_use]
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }
}

impl<R: AsyncBufRead + Unpin> ReadLine for TokioInput<R> {
    /// The next line without its line ending (`\n` or `\r\n`); `Closed` at
    /// the end of input; `Failed` on a read error.
    async fn read_line(&self) -> Result<String, ReadLineError> {
        let mut line = String::new();
        let read = self.reader.lock().await.read_line(&mut line).await;

        match read {
            Ok(0) => Err(ReadLineError::Closed),
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Ok(line)
            }
            Err(_) => Err(ReadLineError::Failed),
        }
    }
}

impl<R> core::fmt::Debug for TokioInput<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TokioInput").finish_non_exhaustive()
    }
}

/// `WriteLine` to a writer: stdout, a `Vec<u8>` in a test, a socket.
///
/// `WriteLine` is fire-and-forget, so a failed write has nowhere to go. A
/// context must not end the process on the routine's behalf: the first
/// failure is logged with `tracing`, and later lines are dropped.
pub struct TokioOutput<W> {
    /// `write_line` is synchronous, so `std`'s mutex: the guard is never held
    /// across an `.await`.
    writer: Mutex<W>,
    /// Set once a write has failed, so the failure is logged once.
    failed: AtomicBool,
}

impl<W> TokioOutput<W> {
    /// Output written line by line to `writer`.
    #[must_use]
    pub const fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
            failed: AtomicBool::new(false),
        }
    }

    /// The writer back, for a test to inspect what was written.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.writer
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<W: io::Write> WriteLine for TokioOutput<W> {
    fn write_line(&self, line: String) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }

        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(error) = writeln!(writer, "{line}").and_then(|()| writer.flush())
            && !self.failed.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(%error, "write failed; later lines are dropped");
        }
    }
}

impl<W> core::fmt::Debug for TokioOutput<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TokioOutput")
            .field("failed", &self.failed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn lines_lose_their_endings_and_the_end_is_closed() {
        let input = TokioInput::new(&b"a\r\nb \nc"[..]);
        assert_eq!(input.read_line().await, Ok(String::from("a")));
        assert_eq!(
            input.read_line().await,
            Ok(String::from("b ")),
            "only the ending goes"
        );
        assert_eq!(
            input.read_line().await,
            Ok(String::from("c")),
            "a last line with no ending"
        );
        assert_eq!(input.read_line().await, Err(ReadLineError::Closed));
    }

    #[test]
    fn written_lines_end_in_a_newline() {
        let output = TokioOutput::new(Vec::new());
        output.write_line(String::from("one"));
        output.write_line(String::from("two"));
        assert_eq!(output.into_inner(), b"one\ntwo\n");
    }

    /// Fails every write, and counts the attempts.
    struct Broken<'a>(&'a AtomicUsize);

    impl io::Write for Broken<'_> {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::other("broken"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn after_a_failed_write_later_lines_are_dropped() {
        let attempts = AtomicUsize::new(0);
        let output = TokioOutput::new(Broken(&attempts));
        output.write_line(String::from("lost"));
        let after_first = attempts.load(Ordering::Relaxed);
        output.write_line(String::from("dropped"));
        assert!(after_first > 0, "the first line was attempted");
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            after_first,
            "the second was not"
        );
    }
}
