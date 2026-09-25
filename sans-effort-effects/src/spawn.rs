//! Spawning: starting a child routine.
//!
//! A child runs as a machine of its own, beside its parent rather than
//! inside it. Parent and child talk through whatever the parent hands the
//! child when it spawns it — typically the ends of a channel. Channels are
//! plain Rust; they are not a capability.
//!
//! Spawning is fire-and-forget: it records the child, unstarted, and returns.
//! Whoever runs the parent decides when and where the child runs. The closure
//! receives the child's context — `child_ctx` — because that context does not
//! exist until the child's machine does.
//!
//! ```
//! use core::ops::ControlFlow;
//! use sans_effort_core::step::Step;
//! use sans_effort_effects::{console::WriteLine, spawn::Spawn};
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
//!     C: Spawn,
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
//! # Two Methods
//!
//! [`spawn`](Spawn::spawn) starts a child that may move between threads
//! after it starts — work-stealing under tokio — so its future must be
//! `Send`. Every capability in this crate declares its futures `Send`, so a
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
//! [`spawn_pinned`](Spawn::spawn_pinned) starts a child that stays on the
//! thread that starts it: only the closure must be `Send`, so the child's
//! future may hold an `Rc` or a value tied to its thread.

pub mod effect;

use crate::ctx::{AsCtx, Ctx};
use alloc::boxed::Box;
use core::future::Future;
use sans_effort_core::driver::{BoxedRoutine, LocalBoxedRoutine};

/// Start child routines.
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

    /// Start a child that stays on the thread that starts it. Its future need
    /// not be `Send`.
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
/// parent's context type, so capabilities an application reifies on a
/// newtype of its own are the parent's alone.
impl<C: AsCtx> Spawn for C
where
    C::Vocabulary:
        From<effect::Spawn<C::Vocabulary>> + From<effect::SpawnPinned<C::Vocabulary>> + 'static,
{
    type Child = Ctx<C::Vocabulary>;

    fn spawn<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    >(
        &self,
        f: F,
    ) {
        let child =
            effect::Child::new(move |outbox| -> BoxedRoutine { Box::pin(f(Ctx::new(outbox))) });
        self.ctx().notify(effect::Spawn(child));
    }

    fn spawn_pinned<
        F: FnOnce(Self::Child) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    >(
        &self,
        f: F,
    ) {
        let child = effect::PinnedChild::new(move |outbox| -> LocalBoxedRoutine {
            Box::pin(f(Ctx::new(outbox)))
        });
        self.ctx().notify(effect::SpawnPinned(child));
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic,
        reason = "let-else arms name the effect the test expected"
    )]

    use super::*;
    use crate::console::{WriteLine, effect::WriteLine as Written};
    use alloc::{rc::Rc, string::String, vec::Vec};
    use core::ops::ControlFlow;
    use sans_effort_core::{
        driver::{Driver, LocalDriver, Yield, status::Status},
        step::Step,
    };

    enum Effect {
        Spawn(effect::Spawn<Effect>),
        SpawnPinned(effect::SpawnPinned<Effect>),
        WriteLine(Written),
    }

    impl From<effect::Spawn<Effect>> for Effect {
        fn from(spawn: effect::Spawn<Effect>) -> Self {
            Effect::Spawn(spawn)
        }
    }

    impl From<effect::SpawnPinned<Effect>> for Effect {
        fn from(spawn: effect::SpawnPinned<Effect>) -> Self {
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

    fn written(step: Yield<Effect>) -> Vec<String> {
        step.into_iter()
            .map(|e| match e {
                Effect::WriteLine(Written(line)) => line,
                Effect::Spawn(_) | Effect::SpawnPinned(_) => panic!("not a write"),
            })
            .collect()
    }

    #[test]
    fn children_run_as_machines_of_their_own() {
        let mut parent = Driver::<Effect>::new(|outbox| Parent(Ctx::new(outbox)).run());
        let mut effects = parent.resume().into_iter();
        assert_eq!(parent.status(), Status::Complete);

        let Some(Effect::Spawn(effect::Spawn(child))) = effects.next() else {
            panic!("the migrating child first");
        };
        let Some(Effect::SpawnPinned(effect::SpawnPinned(pinned))) = effects.next() else {
            panic!("then the pinned one");
        };
        let Some(Effect::WriteLine(Written(line))) = effects.next() else {
            panic!("then the parent's own write");
        };
        assert_eq!(line, "spawned", "nothing ran yet: spawning only records");
        assert!(effects.next().is_none());

        let mut child = Driver::from_boxed(|outbox| child.start(outbox));
        assert_eq!(written(child.resume()), ["migrating"]);
        assert!(child.is_finished());

        let mut pinned = LocalDriver::from_boxed(|outbox| pinned.start(outbox));
        assert_eq!(written(pinned.resume()), ["pinned"]);
        assert!(pinned.is_finished());
    }
}
