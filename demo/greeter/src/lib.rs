//! The greeter, written against capability traits.
//!
//! Prompt, read a name, look up a greeting, pause, greet, count, repeat;
//! `quit` ends it. Four kinds of wait per step, and one fire-and-forget
//! effect. Small enough to read in one sitting, and every host in the
//! exploration this library came from drove exactly this program.
//!
//! [`Greeter`] owns a context `C` and asks nothing of it beyond five traits.
//! It does not know whether `read_line` awaits a tokio channel, pops a line
//! off a test script, or records an effect for a Python host and suspends;
//! each of those is a different `C`, and `Greeter` is the same code under all
//! of them. It imports [`Run`] and [`join`] from the mechanism and nothing
//! else — no effect, no handle, no driver.
//!
//! ```text
//!   Greeter<C: Clock + Counter + Directory + Input + Output>: Run
//!        │
//!        ├── C = TokioCtx          (../tokio)   waits are tokio futures; tokio polls; no driver
//!        ├── C = wire::Ctx<E, O>   (../wire)    waits record effects; a Driver polls; any host
//!        └── C = Recording         (tests)      waits are ready at once; one poll runs it all
//! ```
//!
//! No `Send` appears in the traits. Whether `Greeter<C>::run()` is `Send` is
//! decided by `C`, and the compiler works it out where `C` is concrete —
//! `tokio::spawn` accepts a `Greeter<TokioCtx>` because tokio's handles are
//! `Send`; `Driver::new` accepts a `Greeter<Ctx<E, Outbox<E>>>` for the same
//! reason.

#![no_std]

extern crate alloc;

pub mod traits;

use alloc::{format, string::String};
use core::{ops::ControlFlow, time::Duration};
use effect_routine::{join::join, run::Run};
use traits::{Clock, Counter, Directory, Input, Output};

/// How long the greeter pauses between the greeting and the count.
pub const PAUSE: Duration = Duration::from_millis(50);

/// The greeter. One [`step`](Run::step) is one conversation round.
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

/// The same capabilities, two waits at a time.
///
/// Reads a name, then looks up the greeting and counts _concurrently_; then
/// waits out the pause while reading the farewell, likewise. On tokio each
/// [`join`] is two native futures polled together. Behind a reifying context
/// it is two requests in one batch, replied to in either order — which is
/// what the request ids on the wire are for. The routine cannot tell.
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

/// A routine that needs less: it ticks `n` times, pausing between, and never
/// reads a line, looks anything up, or counts.
///
/// The bound _is_ the permission. A `Ticker` cannot ask for input whatever
/// its context could offer, and a context that offers only `Clock + Output`
/// can run a `Ticker` but not a [`Greeter`] — checked where it is built, and,
/// under a reifying context with a host-chosen vocabulary, visible on the
/// wire as tags that can never appear. See `greeter_wire::Quiet`.
#[derive(Debug)]
pub struct Ticker<C: Clock + Output> {
    ctx: C,
    remaining: u32,
}

impl<C: Clock + Output> Ticker<C> {
    /// Tick `n` times through `ctx`.
    pub const fn new(ctx: C, n: u32) -> Self {
        Self { ctx, remaining: n }
    }
}

impl<C: Clock + Output> Run for Ticker<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.remaining == 0 {
            return ControlFlow::Break(());
        }

        self.remaining -= 1;
        self.ctx.write(format!("tick ({} left)", self.remaining));
        self.ctx.sleep(PAUSE).await;
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    //! The capabilities test story: a mock implements the five traits, every
    //! future is ready at once, and one poll runs the routine to completion.
    //! No driver, no runtime, no vocabulary — and what the mock recorded is
    //! the assertion.

    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use alloc::{collections::VecDeque, rc::Rc, string::ToString, vec, vec::Vec};
    use core::cell::{Cell, RefCell};
    use effect_routine::testing::run_now;

    /// What a routine did to its context, in order.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Count,
        Lookup(String),
        ReadLine,
        Sleep(Duration),
        Write(String),
    }

    fn greeting_for(name: &str) -> String {
        format!("Hello to {name}")
    }

    /// A mock of all five capabilities: scripted input, a fixed directory, an
    /// instant clock, a running counter, and a log of every call. `Clone`
    /// shares the log, so the test keeps a handle after moving one into the
    /// routine.
    #[derive(Clone)]
    struct Recording(Rc<Inner>);

    struct Inner {
        calls: RefCell<Vec<Call>>,
        count: Cell<u64>,
        lines: RefCell<VecDeque<String>>,
    }

    impl Recording {
        fn new(script: &[String]) -> Self {
            Self(Rc::new(Inner {
                calls: RefCell::new(Vec::new()),
                count: Cell::new(0),
                lines: RefCell::new(script.iter().cloned().collect()),
            }))
        }

        fn log(&self, call: Call) {
            self.0.calls.borrow_mut().push(call);
        }

        fn calls(&self) -> Vec<Call> {
            self.0.calls.borrow().clone()
        }
    }

    impl Clock for Recording {
        async fn sleep(&self, duration: Duration) {
            self.log(Call::Sleep(duration));
        }
    }

    impl Counter for Recording {
        async fn count(&self) -> u64 {
            self.log(Call::Count);
            self.0.count.set(self.0.count.get() + 1);
            self.0.count.get()
        }
    }

    impl Directory for Recording {
        async fn lookup(&self, name: String) -> String {
            self.log(Call::Lookup(name.clone()));
            greeting_for(&name)
        }
    }

    impl Input for Recording {
        async fn read_line(&self) -> String {
            self.log(Call::ReadLine);
            self.0
                .lines
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| "quit".to_string())
        }
    }

    impl Output for Recording {
        fn write(&self, line: String) {
            self.log(Call::Write(line));
        }
    }

    /// Run a routine against a recording of the script; return what it did.
    fn transcript<P: Run>(routine: impl FnOnce(Recording) -> P, script: &[String]) -> Vec<Call> {
        let recording = Recording::new(script);
        run_now(routine(recording.clone()).run());
        recording.calls()
    }

    /// What the greeter should do for `script`, stated without running it.
    fn expected_greeter(script: &[String]) -> Vec<Call> {
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

    #[test]
    fn greeter_on_any_script() {
        bolero::check!()
            .with_type::<Vec<String>>()
            .for_each(|script| {
                assert_eq!(transcript(Greeter::new, script), expected_greeter(script));
            });
    }

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

    #[test]
    fn ticker_ticks_n_times_and_never_reads() {
        bolero::check!().with_type::<u8>().for_each(|n| {
            let calls = transcript(|ctx| Ticker::new(ctx, u32::from(*n)), &[]);
            assert_eq!(calls.len(), 2 * usize::from(*n));
            assert!(
                calls
                    .iter()
                    .all(|c| matches!(c, Call::Write(_) | Call::Sleep(_)))
            );
        });
    }
}
