//! The ready-made context: every capability, built from the components.

use crate::{
    clock::TokioClock,
    console::{TokioInput, TokioOutput},
};
use core::time::Duration;
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    time::Sleep,
};
use std::io;
use tokio::io::{AsyncBufRead, BufReader, Stdin};

/// Every capability in `sans-effort-effects` as a tokio future: a
/// [`TokioClock`], a [`TokioInput`], and a [`TokioOutput`] in one value.
///
/// A routine that names only stdlib capabilities runs on it directly:
/// `tokio::spawn(Ticker::new(TokioCtx::stdio(), 3).run())`. The future is
/// `Send` whenever `R` and `W` are, and the compiler works that out at the
/// spawn site.
#[derive(Debug)]
pub struct TokioCtx<R, W> {
    clock: TokioClock,
    input: TokioInput<R>,
    output: TokioOutput<W>,
}

impl<R, W> TokioCtx<R, W> {
    /// A context reading lines from `reader` and writing them to `writer`.
    #[must_use]
    pub const fn new(reader: R, writer: W) -> Self {
        Self {
            clock: TokioClock,
            input: TokioInput::new(reader),
            output: TokioOutput::new(writer),
        }
    }

    /// The clock.
    #[must_use]
    pub const fn clock(&self) -> &TokioClock {
        &self.clock
    }

    /// The input.
    #[must_use]
    pub const fn input(&self) -> &TokioInput<R> {
        &self.input
    }

    /// The output.
    #[must_use]
    pub const fn output(&self) -> &TokioOutput<W> {
        &self.output
    }

    /// The reader and the writer back: for a test to see what was written.
    #[must_use]
    pub fn into_parts(self) -> (R, W) {
        (self.input.into_inner(), self.output.into_inner())
    }
}

impl TokioCtx<BufReader<Stdin>, io::Stdout> {
    /// A context over the process's standard input and output.
    #[must_use]
    pub fn stdio() -> Self {
        Self::new(BufReader::new(tokio::io::stdin()), io::stdout())
    }
}

impl<R, W> Sleep for TokioCtx<R, W> {
    async fn sleep(&self, duration: Duration) {
        self.clock.sleep(duration).await;
    }
}

impl<R: AsyncBufRead + Unpin, W> ReadLine for TokioCtx<R, W> {
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.input.read_line().await
    }
}

impl<R, W: io::Write> WriteLine for TokioCtx<R, W> {
    fn write_line(&self, line: String) {
        self.output.write_line(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn every_capability_through_one_value() {
        let ctx = TokioCtx::new(&b"hi\n"[..], Vec::new());
        let start = tokio::time::Instant::now();

        let line = ctx.read_line().await;
        ctx.sleep(Duration::from_millis(10)).await;
        ctx.write_line(format!("{line:?}"));

        assert_eq!(start.elapsed(), Duration::from_millis(10));
        assert_eq!(ctx.into_parts().1, b"Ok(\"hi\")\n");
    }
}
