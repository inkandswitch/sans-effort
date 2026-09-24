//! A receptionist and a clerk per visitor: spawning, and replies over
//! one-shot channels.

use crate::traits::Lookup;
use alloc::{format, string::String, vec::Vec};
use async_channel::{Receiver, Sender};
use core::ops::ControlFlow;
use sans_effort::step::Step;
use sans_effort_effects::{
    console::{ReadLine, WriteLine},
    spawn::Spawn,
};

/// Reads names until the input closes (or someone types `quit`), spawning a
/// [`Clerk`] for each as it arrives, then greets everyone in the order they
/// came.
///
/// The clerks look their names up concurrently and may finish in any order;
/// the receptionist awaits their replies in arrival order, so the transcript
/// is the same whatever order the lookups finish in. Each reply comes back on
/// a one-slot channel. Clerks are spawned with `spawn_pinned`, so each stays
/// on the thread that starts it. They need not — they hold nothing tied to a
/// thread — but [`PingPong`](crate::ping_pong::PingPong) shows the other
/// kind, and between them the demo hosts see both.
#[derive(Debug)]
pub struct FrontDesk<C> {
    ctx: C,
}

impl<C> FrontDesk<C> {
    /// A front desk working through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: ReadLine + WriteLine + Spawn> Step for FrontDesk<C>
where
    C::Child: Lookup + 'static,
{
    async fn step(&mut self) -> ControlFlow<()> {
        let mut visitors: Vec<(String, Receiver<String>)> = Vec::new();

        while let Ok(name) = self.ctx.read_line().await {
            if name == "quit" {
                break;
            }

            let (reply, greeting) = async_channel::bounded(1);
            let clerk_name = name.clone();
            self.ctx.spawn_pinned(move |child_ctx| {
                Clerk {
                    ctx: child_ctx,
                    name: clerk_name,
                    reply,
                }
                .run()
            });
            visitors.push((name, greeting));
        }

        for (name, greeting) in visitors {
            if let Ok(greeting) = greeting.recv().await {
                self.ctx.write_line(format!("{greeting}, {name}!"));
            }
        }

        self.ctx.write_line(String::from("Closed."));
        ControlFlow::Break(())
    }
}

/// Looks one name up and sends the greeting back, once.
#[derive(Debug)]
pub struct Clerk<C> {
    ctx: C,
    name: String,
    reply: Sender<String>,
}

impl<C: Lookup> Step for Clerk<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        let greeting = self.ctx.lookup(core::mem::take(&mut self.name)).await;
        // The receptionist waits for every clerk, so this finds it listening.
        drop(self.reply.send(greeting).await);
        ControlFlow::Break(())
    }
}
