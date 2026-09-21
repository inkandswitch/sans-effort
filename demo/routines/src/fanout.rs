//! The greeter's conversation, two waits at a time.

use crate::{
    PAUSE,
    traits::{Clock, Counter, Directory, Input, Output},
};
use alloc::{format, string::String};
use core::ops::ControlFlow;
use effect_routine::{join::join, run::Run};

/// Reads a name, then looks up the greeting and counts _concurrently_; then
/// waits out the pause while reading the farewell, likewise.
///
/// On tokio each [`join`] is two native futures polled together. Behind a
/// reifying context it is two requests in one batch, replied to in either
/// order — which is what the request ids on the wire are for. The routine
/// cannot tell.
#[derive(Debug)]
pub struct Fanout<C: Clock + Counter + Directory + Input + Output> {
    ctx: C,
}

impl<C: Clock + Counter + Directory + Input + Output> Fanout<C> {
    /// A fan-out greeter that does everything through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: Clock + Counter + Directory + Input + Output> Run for Fanout<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write(String::from("Who are you?"));
        let name = self.ctx.read_line().await;

        let (greeting, n) = join(self.ctx.lookup(name.clone()), self.ctx.count()).await;
        self.ctx.write(format!("{greeting}, {name}! (#{n})"));

        let ((), farewell) = join(self.ctx.sleep(PAUSE), self.ctx.read_line()).await;
        self.ctx.write(format!("Bye, {farewell}."));

        ControlFlow::Break(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::recording::{Call, greeting_for, transcript};
    use alloc::vec;

    /// With every future ready at once, `join` polls left then right, so the
    /// call order is fixed; what the test pins is that both capabilities are
    /// used and the result assembled before the next write.
    #[test]
    fn fanout_on_any_script() {
        bolero::check!()
            .with_type::<(String, String)>()
            .for_each(|(name, farewell)| {
                let script = [name.clone(), farewell.clone()];
                assert_eq!(
                    transcript(Fanout::new, &script),
                    vec![
                        Call::Write("Who are you?".into()),
                        Call::ReadLine,
                        Call::Lookup(name.clone()),
                        Call::Count,
                        Call::Write(format!("{}, {name}! (#1)", greeting_for(name))),
                        Call::Sleep(PAUSE),
                        Call::ReadLine,
                        Call::Write(format!("Bye, {farewell}.")),
                    ]
                );
            });
    }
}
