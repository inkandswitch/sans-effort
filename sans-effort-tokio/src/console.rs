//! The console: lines in from an async reader, lines out to a writer.
//!
//! Two components, one per effect trait, so a context can grant output
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
///
/// # Cancellation
///
/// A read dropped part-way through a line — the losing side of a `select` —
/// keeps the bytes it had read, and the next read continues the line. tokio's
/// own `read_line` throws them away, so a timeout that fires while a user is
/// typing would garble the next line; this reads with `read_until` into a
/// buffer kept here instead. A routine meant for any host must not rely on
/// it: [`ReadLine`] promises no cancel safety in general.
pub struct TokioInput<R> {
    /// `read_line` needs `&mut R`; the trait takes `&self` so that a routine
    /// can `join` two waits on one context. The mutex bridges the two, and
    /// serves readers in the order they asked. It must be tokio's, not
    /// `std`'s: the guard is held across `.await`, and a `std` guard there
    /// would make the future `!Send`.
    input: tokio::sync::Mutex<Input<R>>,
}

/// The reader, and the line in progress: bytes a dropped read had already
/// taken from it.
struct Input<R> {
    reader: R,
    partial: Vec<u8>,
}

impl<R> TokioInput<R> {
    /// Input read line by line from `reader`.
    #[must_use]
    pub const fn new(reader: R) -> Self {
        Self {
            input: tokio::sync::Mutex::const_new(Input {
                reader,
                partial: Vec::new(),
            }),
        }
    }

    /// The reader back. A line a dropped read had begun is discarded.
    #[must_use]
    pub fn into_inner(self) -> R {
        self.input.into_inner().reader
    }
}

impl<R: AsyncBufRead + Unpin + Send> ReadLine for TokioInput<R> {
    /// The next line without its line ending (`\n` or `\r\n`); `Closed` at
    /// the end of input; `Failed` on a read error.
    /// A line that is not UTF-8 is consumed and reads as `Failed`, so the
    /// next read starts on the line after it.
    async fn read_line(&self) -> Result<String, ReadLineError> {
        let mut input = self.input.lock().await;
        let Input { reader, partial } = &mut *input;
        // `read_until` counts only this call's bytes, so the end of input is
        // `Ok(0)` with nothing held: a held line ended by the end of input is
        // still a line.
        match reader.read_until(b'\n', partial).await {
            Ok(0) if partial.is_empty() => Err(ReadLineError::Closed),
            Ok(_) => line(core::mem::take(partial)),
            Err(_) => {
                partial.clear();
                Err(ReadLineError::Failed)
            }
        }
    }
}

/// A line without its ending (`\n` or `\r\n`), if it is UTF-8.
fn line(mut bytes: Vec<u8>) -> Result<String, ReadLineError> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    String::from_utf8(bytes).map_err(|_| ReadLineError::Failed)
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
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use core::time::Duration;
    use std::sync::atomic::AtomicUsize;
    use tokio::{io::AsyncWriteExt, time::timeout};

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

    #[tokio::test]
    async fn a_line_that_is_not_utf8_is_consumed() {
        let input = TokioInput::new(&b"\xff\xfe\nok\n"[..]);
        assert_eq!(input.read_line().await, Err(ReadLineError::Failed));
        assert_eq!(
            input.read_line().await,
            Ok(String::from("ok")),
            "the next read is not stuck on the bad line"
        );
    }

    /// For any line, split anywhere — mid-character included — with a read
    /// dropped in between: the next read returns the whole line, its ending
    /// stripped.
    #[test]
    fn a_line_split_by_a_dropped_read_arrives_whole() {
        bolero::check!()
            .with_type::<(String, u16, bool)>()
            .for_each(|(text, at, crlf)| {
                let text: String = text
                    .chars()
                    .filter(|c| !matches!(c, '\n' | '\r'))
                    .take(256)
                    .collect();
                let bytes = text.as_bytes();
                let at = usize::from(*at) % (bytes.len() + 1);
                let (head, tail) = bytes.split_at(at);
                let ending: &[u8] = if *crlf { b"\r\n" } else { b"\n" };

                paused().block_on(async {
                    let (reader, mut writer) = tokio::io::duplex(4096);
                    let input = TokioInput::new(tokio::io::BufReader::new(reader));

                    writer.write_all(head).await.expect("written");
                    let dropped = timeout(Duration::from_millis(1), input.read_line()).await;
                    assert!(dropped.is_err(), "no line yet: the read was dropped");

                    writer.write_all(tail).await.expect("written");
                    writer.write_all(ending).await.expect("written");
                    assert_eq!(input.read_line().await, Ok(text.clone()));
                });
            });
    }

    /// The line in progress is ended by the end of input, not a newline: it
    /// is still a line, and only then is the input closed.
    #[tokio::test(start_paused = true)]
    async fn a_held_line_ended_by_the_end_of_input_is_a_line() {
        let (reader, mut writer) = tokio::io::duplex(64);
        let input = TokioInput::new(tokio::io::BufReader::new(reader));
        writer.write_all(b"bye").await.expect("written");
        let dropped = timeout(Duration::from_millis(1), input.read_line()).await;
        assert!(dropped.is_err());
        drop(writer);
        assert_eq!(input.read_line().await, Ok(String::from("bye")));
        assert_eq!(input.read_line().await, Err(ReadLineError::Closed));
    }

    /// A current-thread runtime on a paused clock, for a property test.
    fn paused() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("a runtime")
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
