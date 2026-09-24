//! The ready-made context: every capability, built from the components.

use crate::{
    clock::TokioClock,
    console::{TokioInput, TokioOutput},
    spawn::TokioSpawner,
};
use core::{future::Future, time::Duration};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    spawn::Spawn,
    time::Sleep,
};
use std::{io, sync::Arc};
use tokio::io::{AsyncBufRead, BufReader, Stdin};
use tokio_util::task::LocalPoolHandle;

/// Every capability in `sans-effort-effects` as a tokio future: a
/// [`TokioClock`], a [`TokioInput`], a [`TokioOutput`], and a
/// [`TokioSpawner`] in one value.
///
/// A routine that names only stdlib capabilities runs on it directly:
/// `tokio::spawn(Ticker::new(TokioCtx::stdio(pool), 3).run())`. Cloning shares
/// the input, the output, and the spawner: a spawned child's context is a
/// clone of its parent's.
#[derive(Debug)]
pub struct TokioCtx<R, W> {
    clock: TokioClock,
    input: Arc<TokioInput<R>>,
    output: Arc<TokioOutput<W>>,
    spawner: TokioSpawner,
}

impl<R, W> TokioCtx<R, W> {
    /// A context reading lines from `reader`, writing them to `writer`, and
    /// running pinned children on `pool`.
    #[must_use]
    pub fn new(reader: R, writer: W, pool: LocalPoolHandle) -> Self {
        Self {
            clock: TokioClock,
            input: Arc::new(TokioInput::new(reader)),
            output: Arc::new(TokioOutput::new(writer)),
            spawner: TokioSpawner::new(pool),
        }
    }

    /// The clock.
    #[must_use]
    pub const fn clock(&self) -> &TokioClock {
        &self.clock
    }

    /// The input.
    #[must_use]
    pub fn input(&self) -> &TokioInput<R> {
        &self.input
    }

    /// The output.
    #[must_use]
    pub fn output(&self) -> &TokioOutput<W> {
        &self.output
    }

    /// The spawner.
    #[must_use]
    pub const fn spawner(&self) -> &TokioSpawner {
        &self.spawner
    }

    /// The reader and the writer back: for a test to see what was written.
    ///
    /// # Errors
    ///
    /// The context itself, unchanged, while a clone — a child's context —
    /// still shares them.
    pub fn into_parts(self) -> Result<(R, W), Self> {
        if Arc::strong_count(&self.input) > 1 || Arc::strong_count(&self.output) > 1 {
            return Err(self);
        }

        let Self { input, output, .. } = self;
        match (Arc::into_inner(input), Arc::into_inner(output)) {
            (Some(input), Some(output)) => Ok((input.into_inner(), output.into_inner())),
            _ => unreachable!("no clone shares them: counted just now, and only a clone could"),
        }
    }
}

impl TokioCtx<BufReader<Stdin>, io::Stdout> {
    /// A context over the process's standard input and output, running
    /// pinned children on `pool`.
    #[must_use]
    pub fn stdio(pool: LocalPoolHandle) -> Self {
        Self::new(BufReader::new(tokio::io::stdin()), io::stdout(), pool)
    }
}

impl<R, W> Clone for TokioCtx<R, W> {
    fn clone(&self) -> Self {
        Self {
            clock: self.clock,
            input: Arc::clone(&self.input),
            output: Arc::clone(&self.output),
            spawner: self.spawner.clone(),
        }
    }
}

impl<R, W> Sleep for TokioCtx<R, W> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.clock.sleep(duration)
    }
}

impl<R: AsyncBufRead + Unpin + Send, W> ReadLine for TokioCtx<R, W> {
    fn read_line(&self) -> impl Future<Output = Result<String, ReadLineError>> + Send {
        self.input.read_line()
    }
}

impl<R, W: io::Write> WriteLine for TokioCtx<R, W> {
    fn write_line(&self, line: String) {
        self.output.write_line(line);
    }
}

/// A child's context is a clone of its parent's.
impl<R: Send + 'static, W: Send + 'static> Spawn for TokioCtx<R, W> {
    type Child = Self;

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawner.spawn(f(self.clone()));
    }

    fn spawn_pinned<F, Fut>(&self, f: F)
    where
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let child = self.clone();
        self.spawner.spawn_pinned(move || f(child));
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn every_capability_through_one_value() {
        let ctx = TokioCtx::new(&b"hi\n"[..], Vec::new(), LocalPoolHandle::new(1));
        let start = tokio::time::Instant::now();

        let line = ctx.read_line().await;
        ctx.sleep(Duration::from_millis(10)).await;
        ctx.write_line(format!("{line:?}"));

        assert_eq!(start.elapsed(), Duration::from_millis(10));
        assert_eq!(ctx.into_parts().expect("no child").1, b"Ok(\"hi\")\n");
    }

    #[tokio::test]
    async fn a_context_shared_with_a_child_gives_its_parts_back_only_after() {
        let ctx = TokioCtx::new(&b""[..], Vec::<u8>::new(), LocalPoolHandle::new(1));
        let child = ctx.clone();
        let ctx = ctx.into_parts().expect_err("the child still shares them");
        drop(child);
        assert!(ctx.into_parts().is_ok());
    }
}
