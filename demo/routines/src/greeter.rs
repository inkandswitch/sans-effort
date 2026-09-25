//! The greeter: one conversation round per step.

use crate::{
    PAUSE,
    traits::{Count, Lookup},
};
use alloc::{format, string::String};
use core::ops::ControlFlow;
use sans_effort::step::Step;
use sans_effort::{
    console::{ReadLine, WriteLine},
    time::Sleep,
};

/// Prompt, read a name, look up a greeting, pause, greet, count; repeat until
/// the name is `quit` or the input ends.
#[derive(Debug)]
pub struct Greeter<C: Count + Lookup + ReadLine + Sleep + WriteLine> {
    ctx: C,
}

impl<C: Count + Lookup + ReadLine + Sleep + WriteLine> Greeter<C> {
    /// A greeter that does everything through `ctx`.
    pub const fn new(ctx: C) -> Self {
        Self { ctx }
    }
}

impl<C: Count + Lookup + ReadLine + Sleep + WriteLine> Step for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write_line(String::from("Who are you?"));
        let name = match self.ctx.read_line().await {
            Ok(name) if name != "quit" => name,
            Ok(_) | Err(_) => {
                self.ctx.write_line(String::from("Bye."));
                return ControlFlow::Break(());
            }
        };

        let greeting = self.ctx.lookup(name.clone()).await;
        self.ctx.sleep(PAUSE).await;
        self.ctx.write_line(format!("{greeting}, {name}!"));

        let n = self.ctx.count().await;
        self.ctx.write_line(format!("(greeted {n} so far)"));

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
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
            out.extend([
                Call::Write("Who are you?".into()),
                Call::ReadLine,
                Call::Lookup(name.clone()),
                Call::Sleep(PAUSE),
                Call::Write(format!("{}, {name}!", greeting_for(name))),
                Call::Count,
                Call::Write(format!("(greeted {} so far)", i + 1)),
            ]);
        }

        out.extend([
            Call::Write("Who are you?".into()),
            Call::ReadLine,
            Call::Write("Bye.".into()),
        ]);
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
