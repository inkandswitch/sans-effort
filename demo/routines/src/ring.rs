//! A token passed around a ring of routines: what one hop costs on each host.

use alloc::{format, vec::Vec};
use async_channel::{Receiver, Sender};
use core::ops::ControlFlow;
use sans_effort::step::Step;
use sans_effort_effects::{console::WriteLine, spawn::Spawn};

/// Spawns `size - 1` [`Node`]s in a ring with itself, then sends a counter
/// around it `laps` times; each hop adds one. Only this routine writes — one
/// line, at the end — so the transcript is the same on every host, and a
/// host's timing of the run divided by `size × laps` is its cost per hop.
#[derive(Debug)]
pub struct Ring<C> {
    ctx: C,
    size: u32,
    laps: u32,
}

impl<C> Ring<C> {
    /// A ring of `size` routines, counting `laps` times around.
    pub const fn new(ctx: C, size: u32, laps: u32) -> Self {
        Self { ctx, size, laps }
    }
}

impl<C: Spawn + WriteLine> Step for Ring<C>
where
    C::Child: Send + 'static,
{
    async fn step(&mut self) -> ControlFlow<()> {
        // Node `i` receives on channel `i` and sends on channel `i + 1`,
        // wrapping; this routine is node 0.
        let (senders, receivers): (Vec<Sender<u64>>, Vec<Receiver<u64>>) =
            (0..self.size).map(|_| async_channel::unbounded()).unzip();
        let mut nodes = receivers
            .into_iter()
            .zip(senders.iter().cycle().skip(1).cloned());
        let Some((inbox, onward)) = nodes.next() else {
            return ControlFlow::Break(());
        };

        for (inbox, onward) in nodes {
            self.ctx.spawn(move |child_ctx| {
                Node {
                    _ctx: child_ctx,
                    inbox,
                    onward,
                }
                .run()
            });
        }
        // Only the nodes hold senders now: when this routine drops its own,
        // the ring closes one node at a time.
        drop(senders);

        let mut hops = 0;
        for _ in 0..self.laps {
            if onward.send(hops).await.is_err() {
                break;
            }

            let Ok(token) = inbox.recv().await else {
                break;
            };

            hops = token + 1;
        }

        self.ctx.write_line(format!(
            "ring of {}, {} laps: {hops} hops",
            self.size, self.laps
        ));
        ControlFlow::Break(())
    }
}

/// Passes the counter on, one more, until the ring closes.
#[derive(Debug)]
pub struct Node<C> {
    _ctx: C,
    inbox: Receiver<u64>,
    onward: Sender<u64>,
}

impl<C> Step for Node<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        let Ok(token) = self.inbox.recv().await else {
            return ControlFlow::Break(());
        };

        if self.onward.send(token + 1).await.is_err() {
            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    }
}
