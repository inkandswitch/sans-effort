//! The ready-made context: every effect trait, built from the components.

use crate::{
    clock::TokioClock,
    console::{TokioInput, TokioOutput},
    env::TokioEnv,
    fs::TokioFs,
    random::TokioRandom,
    spawn::TokioSpawner,
};
use core::{future::Future, time::Duration};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    env::Var,
    fs::{FsError, ReadFile, WriteFile},
    random::Random,
    spawn::{Spawn, SpawnPinned},
    time::{Now, Sleep, UnixTime},
};
use std::{io, sync::Arc};
use tokio::io::{AsyncBufRead, BufReader, Stdin};
use tokio_util::task::LocalPoolHandle;

/// Every effect trait in `sans-effort-effects` as a tokio future: a
/// [`TokioClock`], a [`TokioInput`], a [`TokioOutput`], a [`TokioFs`], a
/// [`TokioEnv`], a [`TokioRandom`], and a [`TokioSpawner`] in one value.
///
/// A routine that names only stdlib effect traits runs on it directly:
/// `tokio::spawn(Ticker::new(TokioCtx::stdio(pool), 3).run())`. Cloning shares
/// the input, the output, and the spawner: a spawned child's context is a
/// clone of its parent's.
#[derive(Debug)]
pub struct TokioCtx<R, W> {
    clock: TokioClock,
    /// Behind one `Arc`: clones share input and output together, so one
    /// count says whether a clone still holds them.
    console: Arc<Console<R, W>>,
    fs: TokioFs,
    env: TokioEnv,
    random: TokioRandom,
    spawner: TokioSpawner,
}

impl<R, W> TokioCtx<R, W> {
    /// A context reading lines from `reader`, writing them to `writer`, and
    /// running pinned children on `pool`.
    #[must_use]
    pub fn new(reader: R, writer: W, pool: LocalPoolHandle) -> Self {
        Self {
            clock: TokioClock,
            console: Arc::new(Console {
                input: TokioInput::new(reader),
                output: TokioOutput::new(writer),
            }),
            fs: TokioFs,
            env: TokioEnv,
            random: TokioRandom,
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
        &self.console.input
    }

    /// The output.
    #[must_use]
    pub fn output(&self) -> &TokioOutput<W> {
        &self.console.output
    }

    /// The file system.
    #[must_use]
    pub const fn fs(&self) -> &TokioFs {
        &self.fs
    }

    /// The environment.
    #[must_use]
    pub const fn env(&self) -> &TokioEnv {
        &self.env
    }

    /// The randomness.
    #[must_use]
    pub const fn random(&self) -> &TokioRandom {
        &self.random
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
        let Self {
            clock,
            console,
            fs,
            env,
            random,
            spawner,
        } = self;
        match Arc::try_unwrap(console) {
            Ok(Console { input, output }) => Ok((input.into_inner(), output.into_inner())),
            Err(console) => Err(Self {
                clock,
                console,
                fs,
                env,
                random,
                spawner,
            }),
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
            console: Arc::clone(&self.console),
            fs: self.fs,
            env: self.env,
            random: self.random,
            spawner: self.spawner.clone(),
        }
    }
}

impl<R, W> Sleep for TokioCtx<R, W> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.clock.sleep(duration)
    }
}

impl<R, W> Now for TokioCtx<R, W> {
    fn now(&self) -> impl Future<Output = UnixTime> + Send {
        self.clock.now()
    }
}

impl<R: Sync, W: Sync> ReadFile for TokioCtx<R, W> {
    fn read_file(&self, path: String) -> impl Future<Output = Result<Vec<u8>, FsError>> + Send {
        self.fs.read_file(path)
    }
}

impl<R: Sync, W: Sync> WriteFile for TokioCtx<R, W> {
    fn write_file(
        &self,
        path: String,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), FsError>> + Send {
        self.fs.write_file(path, bytes)
    }
}

impl<R, W> Var for TokioCtx<R, W> {
    fn var(&self, name: String) -> impl Future<Output = Option<String>> + Send {
        self.env.var(name)
    }
}

impl<R, W> Random for TokioCtx<R, W> {
    fn random_bytes(&self, len: u32) -> impl Future<Output = Vec<u8>> + Send {
        self.random.random_bytes(len)
    }
}

impl<R: AsyncBufRead + Unpin + Send, W> ReadLine for TokioCtx<R, W> {
    fn read_line(&self) -> impl Future<Output = Result<String, ReadLineError>> + Send {
        self.console.input.read_line()
    }
}

impl<R, W: io::Write> WriteLine for TokioCtx<R, W> {
    fn write_line(&self, line: String) {
        self.console.output.write_line(line);
    }
}

/// The input and the output a context's clones share.
#[derive(Debug)]
struct Console<R, W> {
    input: TokioInput<R>,
    output: TokioOutput<W>,
}

/// A child's context is a clone of its parent's.
impl<R: Send + 'static, W: Send + 'static> Spawn for TokioCtx<R, W> {
    type Child = Self;

    fn spawn<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static>(
        &self,
        f: F,
    ) {
        self.spawner.spawn(f(self.clone()));
    }
}

/// As for [`Spawn`]: a clone, run on the pool's thread.
impl<R: Send + 'static, W: Send + 'static> SpawnPinned for TokioCtx<R, W> {
    type Child = Self;

    fn spawn_pinned<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + 'static>(
        &self,
        f: F,
    ) {
        let child = self.clone();
        self.spawner.spawn_pinned(move || f(child));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testresult::TestResult;

    use std::time::SystemTime;
    use tokio::sync::oneshot;

    /// Each trait forwards to its component, and each kind of child gets a
    /// clone of the context: its lines land in the parent's output, and once
    /// every child has let go, the parts come back.
    #[tokio::test(start_paused = true)]
    async fn every_effect_trait_through_one_value() -> TestResult {
        let ctx = TokioCtx::new(&b"hi\n"[..], Vec::new(), LocalPoolHandle::new(1));
        let start = tokio::time::Instant::now();

        let line = ctx.read_line().await;
        ctx.sleep(Duration::from_millis(10)).await;
        ctx.write_line(format!("{line:?}"));
        assert_eq!(start.elapsed(), Duration::from_millis(10));

        let wall = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
        assert!(ctx.now().await.since_epoch() >= wall);

        assert_eq!(
            ctx.var("CARGO_PKG_NAME".into()).await.as_deref(),
            Some(env!("CARGO_PKG_NAME"))
        );
        assert_eq!(ctx.random_bytes(8).await.len(), 8);

        let dir = std::env::temp_dir().join(format!("sans-effort-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("file").to_string_lossy().into_owned();
        ctx.write_file(path.clone(), b"kept".to_vec()).await?;
        assert_eq!(ctx.read_file(path).await, Ok(b"kept".to_vec()));
        std::fs::remove_dir_all(dir)?;

        let (done, finished) = oneshot::channel();
        ctx.spawn(|child| async move {
            child.write_line("from a child".into());
            drop(child);
            assert!(done.send(()).is_ok(), "the parent waits");
        });
        finished.await?;

        let (done, finished) = oneshot::channel();
        ctx.spawn_pinned(|child| async move {
            child.write_line("from a pinned child".into());
            drop(child);
            assert!(done.send(()).is_ok(), "the parent waits");
        });
        finished.await?;

        let (unread, written) = ctx.into_parts().map_err(|_| "every child let go")?;
        assert!(unread.is_empty());
        assert_eq!(
            String::from_utf8(written)?,
            "Ok(\"hi\")\nfrom a child\nfrom a pinned child\n"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_context_shared_with_a_child_gives_its_parts_back_only_after() -> TestResult {
        let ctx = TokioCtx::new(&b"unread\n"[..], Vec::<u8>::new(), LocalPoolHandle::new(1));
        let child = ctx.clone();
        let ctx = ctx
            .into_parts()
            .err()
            .ok_or("the child still shares them")?;
        child.write_line("from the child".into());
        drop(child);

        let (unread, written) = ctx.into_parts().map_err(|_| "the child let go")?;
        assert_eq!(
            (unread, written.as_slice()),
            (&b"unread\n"[..], &b"from the child\n"[..])
        );
        Ok(())
    }
}
