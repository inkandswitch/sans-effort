//! Two routines on two machines, talking over plain channels.

use alloc::format;
use async_channel::{Receiver, Sender};
use core::ops::ControlFlow;
use sans_effort::run::Run;
use sans_effort_effects::{console::WriteLine, spawn::Spawn};

/// Spawns a [`Ponger`], then sends it `rounds` pings, waiting for each pong
/// before the next.
///
/// Only the parent writes: a host that polls both machines at once prints each
/// machine's writes after its call returns, so writes from two machines would
/// interleave by timing, and the transcript would stop being comparable.
///
/// The channels are ordinary `async-channel`s, not effects. While the parent
/// waits for a pong it has no request outstanding; under a driver it reports
/// `Idle`, and the host resumes it after the child has run. The child is
/// spawned with `spawn`: it holds nothing tied to a thread, so under tokio it
/// may run on any worker.
#[derive(Debug)]
pub struct PingPong<C> {
    ctx: C,
    rounds: u32,
}

impl<C> PingPong<C> {
    /// Play `rounds` rounds through `ctx`.
    pub const fn new(ctx: C, rounds: u32) -> Self {
        Self { ctx, rounds }
    }
}

impl<C: Spawn + WriteLine> Run for PingPong<C>
where
    C::Child: Send + 'static,
{
    async fn step(&mut self) -> ControlFlow<()> {
        let (serve, serves) = async_channel::unbounded();
        let (reply, replies) = async_channel::unbounded();
        self.ctx.spawn(move |child_ctx| {
            Ponger {
                _ctx: child_ctx,
                pings: serves,
                pongs: reply,
            }
            .run()
        });

        for n in 1..=self.rounds {
            if serve.send(n).await.is_err() {
                break;
            }

            let Ok(pong) = replies.recv().await else {
                break;
            };

            self.ctx.write_line(format!("ping {n}, pong {pong}"));
        }

        ControlFlow::Break(())
    }
}

/// Answers each ping with a pong of the same number, until the pings stop.
/// It keeps a context — a child always gets one — though it asks nothing of it.
#[derive(Debug)]
pub struct Ponger<C> {
    _ctx: C,
    pings: Receiver<u32>,
    pongs: Sender<u32>,
}

impl<C> Run for Ponger<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        let Ok(n) = self.pings.recv().await else {
            return ControlFlow::Break(());
        };

        if self.pongs.send(n).await.is_err() {
            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    }
}
