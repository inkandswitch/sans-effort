//! Spawning: starting a child routine, as [`Spawn`] (a child that may move
//! between threads) or [`SpawnPinned`] (one that stays on its thread).
//!
//! A child runs as a machine of its own, beside its parent rather than
//! inside it. Parent and child talk through whatever the parent hands the
//! child when it spawns it — typically the ends of a channel. Channels are
//! plain Rust; they are not an effect trait.
//!
//! Spawning is fire-and-forget: it records the child, unstarted, and returns.
//! Whoever runs the parent decides when and where the child runs. The closure
//! receives the child's context — `child_ctx` — because that context does not
//! exist until the child's machine does.
//!
//! ```
//! use core::ops::ControlFlow;
//! use sans_effort_core::step::Step;
//! use sans_effort_effects::{console::WriteLine, spawn::SpawnPinned};
//!
//! struct Parent<C>(C);
//!
//! struct Child<C>(C, String);
//!
//! impl<C: WriteLine> Step for Child<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         self.0.write_line(core::mem::take(&mut self.1));
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! impl<C> Step for Parent<C>
//! where
//!     C: SpawnPinned,
//!     C::Child: WriteLine + 'static,
//! {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         let greeting = String::from("hello from a child");
//!         self.0.spawn_pinned(move |child_ctx| Child(child_ctx, greeting).run());
//!         ControlFlow::Break(())
//!     }
//! }
//! ```
//!
//! # Two Traits
//!
//! A routine names only the kind of spawning it uses, and a host grants each
//! separately: a vocabulary without [`SpawnPinnedEffect`] lets a routine
//! start migrating children but not pinned ones. A routine that uses both
//! names its child context through one of them: `<C as Spawn>::Child`.
//!
//! [`spawn`](Spawn::spawn) starts a child that may move between threads
//! after it starts — work-stealing under tokio — so its future must be
//! `Send`. Every effect trait in this crate declares its futures `Send`, so a
//! routine generic over its context proves that with `C::Child: Send + Sync`
//! (`Sync` because a routine's methods borrow it across `.await`s):
//!
//! ```
//! use core::{ops::ControlFlow, time::Duration};
//! use sans_effort_core::step::Step;
//! use sans_effort_effects::{spawn::Spawn, time::Sleep};
//!
//! struct Napper<C>(C);
//!
//! impl<C: Sleep> Step for Napper<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         self.0.sleep(Duration::from_millis(1)).await;
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! struct Parent<C>(C);
//!
//! impl<C> Step for Parent<C>
//! where
//!     C: Spawn,
//!     C::Child: Sleep + Send + Sync + 'static,
//! {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         self.0.spawn(|child_ctx| Napper(child_ctx).run());
//!         ControlFlow::Break(())
//!     }
//! }
//! ```
//!
//! [`spawn_pinned`](SpawnPinned::spawn_pinned) starts a child that stays on the
//! thread that starts it: only the closure must be `Send`, so the child's
//! future may hold an `Rc` or a value tied to its thread.

use crate::ctx::{AsCtx, Ctx};
use alloc::boxed::Box;
use core::{fmt, future::Future};
use sans_effort_core::driver::{BoxedRoutine, LocalBoxedRoutine, outbox::Outbox};

/// Start child routines that may move between threads.
pub trait Spawn {
    /// The child's context.
    type Child;

    /// Start a child that may move between threads after it starts.
    fn spawn<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    >(
        &self,
        f: F,
    );
}

/// Start child routines that stay on one thread.
pub trait SpawnPinned {
    /// The child's context.
    type Child;

    /// Start a child that stays on the thread that first resumes it. Its
    /// future need not be `Send`.
    fn spawn_pinned<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    >(
        &self,
        f: F,
    );
}

/// A reifying context records the child for the host. The child's context is
/// a [`Ctx`] of the same vocabulary, so a child can do no more than its
/// parent: attenuation passes down by type. It is a plain `Ctx`, not the
/// parent's context type, so effect traits an application reifies on a
/// newtype of its own are the parent's alone.
impl<C: AsCtx> Spawn for C
where
    C::Vocabulary: From<SpawnEffect<C::Vocabulary>> + 'static,
{
    type Child = Ctx<C::Vocabulary>;

    fn spawn<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    >(
        &self,
        f: F,
    ) {
        let child = Child::new(move |outbox| -> BoxedRoutine { Box::pin(f(Ctx::new(outbox))) });
        self.ctx().tell(SpawnEffect(child));
    }
}

/// As for [`Spawn`]: the child's context is a [`Ctx`] of the parent's
/// vocabulary.
impl<C: AsCtx> SpawnPinned for C
where
    C::Vocabulary: From<SpawnPinnedEffect<C::Vocabulary>> + 'static,
{
    type Child = Ctx<C::Vocabulary>;

    fn spawn_pinned<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    >(
        &self,
        f: F,
    ) {
        let child =
            PinnedChild::new(move |outbox| -> LocalBoxedRoutine { Box::pin(f(Ctx::new(outbox))) });
        self.ctx().tell(SpawnPinnedEffect(child));
    }
}

/// Start a child that may move between threads: a tell.
///
/// Each spawn effect carries the child itself, unstarted: a closure from the
/// child's own outbox to its boxed future. The host's vocabulary decides what
/// spawning means — typically registering the child as a machine of its own
/// and telling the host its handle. None of it crosses an FFI boundary; only
/// that handle does.
#[derive(Debug)]
pub struct SpawnEffect<E>(pub Child<E>);

/// Start a child that stays on the thread that starts it: a tell.
#[derive(Debug)]
pub struct SpawnPinnedEffect<E>(pub PinnedChild<E>);

/// A child routine, unstarted; its future is `Send`.
///
/// Only [`Spawn`] builds one, so a child always gets a context
/// built around its own outbox.
pub struct Child<E> {
    make: Box<dyn FnOnce(Outbox<E>) -> BoxedRoutine + Send>,
}

impl<E> Child<E> {
    fn new<M: FnOnce(Outbox<E>) -> BoxedRoutine + Send + 'static>(make: M) -> Self {
        Self {
            make: Box::new(make),
        }
    }

    /// Build the child's future around `outbox`, the outbox of the machine
    /// that will run it — `Driver::from_boxed(|outbox| child.start(outbox))`.
    #[must_use]
    pub fn start(self, outbox: Outbox<E>) -> BoxedRoutine {
        (self.make)(outbox)
    }
}

impl<E> fmt::Debug for Child<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Child").finish_non_exhaustive()
    }
}

/// A child routine, unstarted; its future need not be `Send`, so it is built
/// and run on one thread. The closure that builds it is `Send`, so the child
/// can be handed to that thread first.
pub struct PinnedChild<E> {
    make: Box<dyn FnOnce(Outbox<E>) -> LocalBoxedRoutine + Send>,
}

impl<E> PinnedChild<E> {
    fn new<M: FnOnce(Outbox<E>) -> LocalBoxedRoutine + Send + 'static>(make: M) -> Self {
        Self {
            make: Box::new(make),
        }
    }

    /// Build the child's future around `outbox`, on the thread that will run
    /// it — `Driver::local_boxed(|outbox| child.start(outbox))`.
    #[must_use]
    pub fn start(self, outbox: Outbox<E>) -> LocalBoxedRoutine {
        (self.make)(outbox)
    }
}

impl<E> fmt::Debug for PinnedChild<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedChild").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::{WriteLine, WriteLineEffect as Written};
    use alloc::{rc::Rc, string::String, vec::Vec};
    use core::ops::ControlFlow;
    use sans_effort_core::{
        driver::{Driver, Yield, status::Status},
        step::Step,
    };
    use testresult::TestResult;

    enum Effect {
        Spawn(SpawnEffect<Effect>),
        SpawnPinned(SpawnPinnedEffect<Effect>),
        WriteLine(Written),
    }

    impl From<SpawnEffect<Effect>> for Effect {
        fn from(spawn: SpawnEffect<Effect>) -> Self {
            Effect::Spawn(spawn)
        }
    }

    impl From<SpawnPinnedEffect<Effect>> for Effect {
        fn from(spawn: SpawnPinnedEffect<Effect>) -> Self {
            Effect::SpawnPinned(spawn)
        }
    }

    impl From<Written> for Effect {
        fn from(write: Written) -> Self {
            Effect::WriteLine(write)
        }
    }

    /// Writes its line and finishes.
    struct Writer<C>(C, String);

    impl<C: WriteLine> Step for Writer<C> {
        async fn step(&mut self) -> ControlFlow<()> {
            self.0.write_line(core::mem::take(&mut self.1));
            ControlFlow::Break(())
        }
    }

    /// Holds an `Rc` across an `.await` before writing: its future is not
    /// `Send`, which a pinned child may be.
    struct Local<C>(C, Rc<String>);

    impl<C: WriteLine> Step for Local<C> {
        async fn step(&mut self) -> ControlFlow<()> {
            let text = Rc::clone(&self.1);
            core::future::ready(()).await;
            self.0.write_line(String::clone(&text));
            ControlFlow::Break(())
        }
    }

    /// Spawns one child of each kind, then says it did. Concrete in its
    /// context, so the migrating child's future is provably `Send`.
    struct Parent(Ctx<Effect>);

    impl Step for Parent {
        async fn step(&mut self) -> ControlFlow<()> {
            self.0
                .spawn(|child_ctx| Writer(child_ctx, String::from("migrating")).run());
            self.0
                .spawn_pinned(|child_ctx| Local(child_ctx, Rc::new(String::from("pinned"))).run());
            self.0.write_line(String::from("spawned"));
            ControlFlow::Break(())
        }
    }

    fn written(step: Yield<Effect>) -> TestResult<Vec<String>> {
        let mut lines = Vec::new();
        for effect in step {
            match effect {
                Effect::WriteLine(Written(line)) => lines.push(line),
                Effect::Spawn(_) | Effect::SpawnPinned(_) => return Err("not a write")?,
            }
        }
        Ok(lines)
    }

    #[test]
    fn children_run_as_machines_of_their_own() -> TestResult {
        let mut parent = Driver::<Effect>::new(|outbox| Parent(Ctx::new(outbox)).run());
        let mut effects = parent.resume().into_iter();
        assert_eq!(parent.status(), Status::Complete);

        let Some(Effect::Spawn(SpawnEffect(child))) = effects.next() else {
            return Err("the migrating child first")?;
        };
        let Some(Effect::SpawnPinned(SpawnPinnedEffect(pinned))) = effects.next() else {
            return Err("then the pinned one")?;
        };
        let Some(Effect::WriteLine(Written(line))) = effects.next() else {
            return Err("then the parent's own write")?;
        };
        assert_eq!(line, "spawned", "nothing ran yet: spawning only records");
        assert!(effects.next().is_none());

        let mut child = Driver::from_boxed(|outbox| child.start(outbox));
        assert_eq!(written(child.resume())?, ["migrating"]);
        assert!(child.is_finished());

        let mut pinned = Driver::local_boxed(|outbox| pinned.start(outbox));
        assert_eq!(written(pinned.resume())?, ["pinned"]);
        assert!(pinned.is_finished());
        Ok(())
    }
}
