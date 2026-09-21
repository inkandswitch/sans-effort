//! The greeter: one conversation round per step.

use crate::{
    PAUSE,
    traits::{Clock, Counter, Directory, Input, Output},
};
use alloc::{format, string::String};
use core::ops::ControlFlow;
use effect_routine::run::Run;

/// Prompt, read a name, look up a greeting, pause, greet, count; repeat until
/// the name is `quit`.
#[derive(Debug)]
pub struct Greeter<C: Clock + Counter + Directory + Input + Output> {
    ctx: C,
}

impl<C: Clock + Counter + Directory + Input + Output> Greeter<C> {
    /// A greeter that does everything through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: Clock + Counter + Directory + Input + Output> Run for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write(String::from("Who are you?"));
        let name = self.ctx.read_line().await;

        if name == "quit" {
            self.ctx.write(String::from("Bye."));
            return ControlFlow::Break(());
        }

        let greeting = self.ctx.lookup(name.clone()).await;
        self.ctx.sleep(PAUSE).await;
        self.ctx.write(format!("{greeting}, {name}!"));

        let n = self.ctx.count().await;
        self.ctx.write(format!("(greeted {n} so far)"));

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::recording::{Call, greeting_for, transcript};
    use alloc::vec::Vec;

    /// What the greeter should do for `script`, stated without running it.
    fn expected(script: &[String]) -> Vec<Call> {
        let mut out = Vec::new();

        for (i, name) in script
            .iter()
            .take_while(|n| n.as_str() != "quit")
            .enumerate()
        {
            out.push(Call::Write("Who are you?".into()));
            out.push(Call::ReadLine);
            out.push(Call::Lookup(name.clone()));
            out.push(Call::Sleep(PAUSE));
            out.push(Call::Write(format!("{}, {name}!", greeting_for(name))));
            out.push(Call::Count);
            out.push(Call::Write(format!("(greeted {} so far)", i + 1)));
        }

        out.push(Call::Write("Who are you?".into()));
        out.push(Call::ReadLine);
        out.push(Call::Write("Bye.".into()));
        out
    }

    /// For any script, the greeter does exactly what the specification says.
    /// `quit` anywhere in the script ends the conversation there.
    #[test]
    fn greeter_on_any_script() {
        bolero::check!()
            .with_type::<Vec<String>>()
            .for_each(|script| {
                assert_eq!(transcript(Greeter::new, script), expected(script));
            });
    }
}
