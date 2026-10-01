//! Waiting for an answer, but not forever: a receive raced against a sleep.

use crate::PAUSE;
use alloc::{format, string::String};
use async_channel::Receiver;
use core::{ops::ControlFlow, time::Duration};
use sans_effort::{
    console::WriteLine,
    select::{Either, select},
    spawn::Spawn,
    step::Step,
    time::Sleep,
};

/// How long the routine waits for the quick worker. Far longer than it takes
/// to answer, so the sleep always loses; long enough that a host which fails
/// to cancel the abandoned sleep is caught by the time it takes.
pub const PATIENCE: Duration = Duration::from_secs(30);

/// Asks two workers for an answer, each with a deadline.
///
/// The quick worker answers at once, so the receive wins and the sleep — its
/// ask already made — is abandoned: every host sees that request close, and
/// should stop the timer behind it. The slow worker cannot answer until told
/// to go on, so its deadline passes first; then the routine tells it, and
/// takes the late answer. Neither race can go the other way on any host, so
/// the transcript is the same everywhere.
///
/// The answers come over plain channels, which lose nothing when a receive
/// is abandoned, so racing one is safe.
#[derive(Debug)]
pub struct Deadline<C> {
    ctx: C,
}

impl<C> Deadline<C> {
    /// Ask both workers through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: Sleep> Deadline<C> {
    /// An answer from `answers` within `patience`, described.
    async fn wait(&self, answers: &Receiver<u32>, patience: Duration) -> String {
        match select(answers.recv(), self.ctx.sleep(patience)).await {
            Either::Left(Ok(n)) => format!("answered {n} in time"),
            Either::Left(Err(_)) => "gone".into(),
            Either::Right(()) => format!("no answer within {} ms", patience.as_millis()),
        }
    }
}

impl<C: Sleep + Spawn + WriteLine> Step for Deadline<C>
where
    C::Child: Send + 'static,
{
    async fn step(&mut self) -> ControlFlow<()> {
        let (answer, answers) = async_channel::bounded(1);
        self.ctx.spawn(move |_ctx| async move {
            // The routine may have stopped waiting; then nobody listens.
            answer.send(1_u32).await.unwrap_or_default();
        });
        let quick = self.wait(&answers, PATIENCE).await;
        self.ctx.write_line(format!("quick worker: {quick}"));

        let (go, gate) = async_channel::bounded::<()>(1);
        let (answer, answers) = async_channel::bounded(1);
        self.ctx.spawn(move |_ctx| async move {
            if gate.recv().await.is_ok() {
                answer.send(2_u32).await.unwrap_or_default();
            }
        });
        let slow = self.wait(&answers, PAUSE).await;
        self.ctx.write_line(format!("slow worker: {slow}"));

        go.send(()).await.unwrap_or_default();
        let late = match answers.recv().await {
            Ok(n) => format!("answered {n} late"),
            Err(_) => "gone".into(),
        };
        self.ctx.write_line(format!("slow worker: {late}"));

        ControlFlow::Break(())
    }
}
