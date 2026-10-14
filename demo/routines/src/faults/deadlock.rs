//! Two routines each waiting for the other: a deadlock, on purpose.

use alloc::string::String;
use core::ops::ControlFlow;
use sans_effort::{console::WriteLine, spawn::Spawn, step::Step};

/// Spawns a child, then each waits to hear from the other first. Each holds
/// the sender the other waits on, so neither channel can close, and nothing
/// will ever wake either: a deadlock.
///
/// It is for hosts to report, not hang on. A driven host knows when nothing
/// can happen — no call to make, no request outstanding, nothing from
/// `wakes` — and the demo's say so and exit. Native code has no such
/// detector: under tokio this simply hangs, so `demo:faults` does not run it
/// there; Node's event loop runs dry, and Node exits 13 for an unsettled
/// top-level `await`.
///
/// Kill either machine and the other hears its channel close, and finishes:
/// a crash breaks the deadlock.
#[derive(Debug)]
pub struct Deadlock<C> {
    ctx: C,
}

impl<C> Deadlock<C> {
    /// A deadlock through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: Spawn + WriteLine> Step for Deadlock<C>
where
    C::Child: Send + 'static,
{
    async fn step(&mut self) -> ControlFlow<()> {
        let (to_child, from_parent) = async_channel::bounded::<()>(1);
        let (to_parent, from_child) = async_channel::bounded::<()>(1);
        self.ctx.spawn(move |_ctx| async move {
            if from_parent.recv().await.is_ok() {
                to_parent.send(()).await.unwrap_or_default();
            }
        });

        self.ctx
            .write_line(String::from("each waits for the other to go first"));
        let heard = from_child.recv().await;
        drop(to_child);
        self.ctx.write_line(String::from(if heard.is_ok() {
            "heard from the other"
        } else {
            "the other is gone"
        }));

        ControlFlow::Break(())
    }
}
