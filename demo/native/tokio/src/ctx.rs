//! The demo's native context: `sans-effort-tokio`'s `TokioCtx`, plus the
//! demo's own effect traits.
//!
//! `TokioCtx` already serves `Sleep`, `ReadLine`, `WriteLine`, and `Spawn`
//! with tokio futures and tasks. `Count` and `Lookup` are the demo's, and the
//! orphan rule allows `impl Count for TokioCtx` in neither this crate nor any
//! other that owns only one side — so they go on a type this crate owns,
//! which forwards the stdlib effect traits to the `TokioCtx` inside it, one
//! line each.
//!
//! For the journal it also keeps a small world of its own — a variable,
//! files in memory, a clock that starts at 1 700 000 000 000 ms and moves a
//! second per reading, and counting random bytes — the same one every demo
//! host keeps, so the transcripts agree. `TokioCtx`'s real clock, files,
//! environment, and entropy are `sans-effort-tokio`'s, tested there.
//!
//! Compare `greeter_boundary`'s vocabularies: there each effect trait records
//! an effect and suspends until a host replies. Here each one is a real
//! future, and the routine is the task.

use core::{future::Future, time::Duration};
use routines::effects::{count::Count, lookup::Lookup};
use sans_effort::tokio::ctx::TokioCtx;
use sans_effort::{
    console::{ReadLine, ReadLineError, WriteLine},
    env::Var,
    fs::{FsError, ReadFile, WriteFile},
    random::Random,
    spawn::{Spawn, SpawnPinned},
    time::{Now, Sleep, UnixTime},
};
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};
use tokio::io::AsyncBufRead;

/// `TokioCtx` with a greeting counter, a fixed directory, and the journal's
/// world. Cloning shares all of it, as a spawned child's context does.
#[derive(Debug)]
pub(crate) struct DemoCtx<R, W> {
    tokio: TokioCtx<R, W>,
    greeted: Arc<AtomicU64>,
    world: Arc<World>,
}

/// What the journal reads and writes: files in memory, a clock that moves a
/// second per reading, and the next random byte.
#[derive(Debug, Default)]
struct World {
    files: Mutex<HashMap<String, Vec<u8>>>,
    readings: AtomicU64,
    next_byte: AtomicU8,
}

/// The one variable the journal asks for.
const JOURNAL: (&str, &str) = ("JOURNAL", "notes.txt");

/// Where the demo clock starts.
const EPOCH_MILLIS: u64 = 1_700_000_000_000;

impl<R, W> DemoCtx<R, W> {
    /// The demo's effect traits on top of `tokio`.
    pub(crate) fn new(tokio: TokioCtx<R, W>) -> Self {
        Self {
            tokio,
            greeted: Arc::new(AtomicU64::new(0)),
            world: Arc::default(),
        }
    }
}

impl<R, W> Clone for DemoCtx<R, W> {
    fn clone(&self) -> Self {
        Self {
            tokio: self.tokio.clone(),
            greeted: Arc::clone(&self.greeted),
            world: Arc::clone(&self.world),
        }
    }
}

impl<R, W> Count for DemoCtx<R, W> {
    fn count(&self) -> impl Future<Output = u64> + Send {
        let greeted = Arc::clone(&self.greeted);
        async move { greeted.fetch_add(1, Ordering::Relaxed) + 1 }
    }
}

impl<R, W> Lookup for DemoCtx<R, W> {
    /// A fixed directory, so the answer is ready at once.
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send {
        let greeting = match name.as_str() {
            "alice" => "Hello",
            "bob" => "Hi",
            "carol" => "Hey",
            _ => "Greetings",
        };
        core::future::ready(greeting.to_owned())
    }
}

impl<R, W> Var for DemoCtx<R, W> {
    fn var(&self, name: String) -> impl Future<Output = Option<String>> + Send {
        core::future::ready((name == JOURNAL.0).then(|| JOURNAL.1.to_owned()))
    }
}

impl<R: Sync, W: Sync> ReadFile for DemoCtx<R, W> {
    fn read_file(&self, path: String) -> impl Future<Output = Result<Vec<u8>, FsError>> + Send {
        let world = Arc::clone(&self.world);
        async move {
            let files = world.files.lock().unwrap_or_else(PoisonError::into_inner);
            files.get(&path).cloned().ok_or(FsError::NotFound)
        }
    }
}

impl<R: Sync, W: Sync> WriteFile for DemoCtx<R, W> {
    fn write_file(
        &self,
        path: String,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<(), FsError>> + Send {
        let world = Arc::clone(&self.world);
        async move {
            world
                .files
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path, bytes);
            Ok(())
        }
    }
}

impl<R, W> Now for DemoCtx<R, W> {
    fn now(&self) -> impl Future<Output = UnixTime> + Send {
        let world = Arc::clone(&self.world);
        async move {
            let reading = world.readings.fetch_add(1, Ordering::Relaxed);
            let millis = EPOCH_MILLIS + 1_000 * reading;
            UnixTime::from_since_epoch(Duration::from_millis(millis))
        }
    }
}

impl<R, W> Random for DemoCtx<R, W> {
    fn random_bytes(&self, len: u32) -> impl Future<Output = Vec<u8>> + Send {
        let world = Arc::clone(&self.world);
        async move {
            (0..len)
                .map(|_| world.next_byte.fetch_add(1, Ordering::Relaxed))
                .collect()
        }
    }
}

impl<R, W> Sleep for DemoCtx<R, W> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.tokio.sleep(duration)
    }
}

impl<R: AsyncBufRead + Unpin + Send, W> ReadLine for DemoCtx<R, W> {
    fn read_line(&self) -> impl Future<Output = Result<String, ReadLineError>> + Send {
        self.tokio.read_line()
    }
}

impl<R, W: io::Write> WriteLine for DemoCtx<R, W> {
    fn write_line(&self, line: String) {
        self.tokio.write_line(line);
    }
}

/// A child's context is a clone: it shares the counter, the directory, and
/// the console.
impl<R: Send + 'static, W: Send + 'static> Spawn for DemoCtx<R, W> {
    type Child = Self;

    fn spawn<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static>(
        &self,
        f: F,
    ) {
        self.tokio.spawner().spawn(f(self.clone()));
    }
}

/// As for [`Spawn`]: a clone, run on the pool's thread.
impl<R: Send + 'static, W: Send + 'static> SpawnPinned for DemoCtx<R, W> {
    type Child = Self;

    fn spawn_pinned<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + 'static>(
        &self,
        f: F,
    ) {
        let child = self.clone();
        self.tokio.spawner().spawn_pinned(move || f(child));
    }
}
