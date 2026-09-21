//! The greeter: one effect routine, written in the effects style.
//!
//! Prompt, read a name, look up a greeting, pause, greet, count, repeat;
//! `quit` ends it. Four kinds of wait per turn, and one fire-and-forget
//! effect. Small enough to read in one sitting, and every host in the
//! exploration this library came from drove exactly this program.
//!
//! The routine owns a closed [`Effect`] enum and asks for each wait by name.
//! The enum is _the_ vocabulary — one statement, written once — and it is
//! also the wire: [`View`], [`HostEffect`], and [`Encode`] are implemented
//! here because the author already wrote the enum and there is nothing left
//! for a boundary to restate. In exchange the routine knows it is an effect
//! routine (it imports the driver's types), and to run it anywhere someone
//! writes a host loop that performs the effects — `main.rs` is one in Rust,
//! `../python/main.py` is one in Python over the C ABI in `../cdylib`.

use core::{future::Future, ops::ControlFlow, time::Duration};
use effect_routine::{
    join::join,
    post::Post,
    reply::ReplyHandle,
    run::Run,
    wire::{Encode, HostEffect, Pending, Writer},
};

/// How long the greeter pauses between the greeting and the count.
pub const PAUSE: Duration = Duration::from_millis(50);

/// Something the greeter wants the host to do. Variants that carry a
/// [`ReplyHandle`] await a reply of that type; the rest are fire-and-forget.
#[derive(Debug)]
pub enum Effect {
    /// How many greetings so far, including this one.
    Count(ReplyHandle<u64>),
    /// The greeting word for this name.
    Lookup(String, ReplyHandle<String>),
    /// A line of input.
    ReadLine(ReplyHandle<String>),
    /// Wait this long.
    Sleep(Duration, ReplyHandle<()>),
    /// Show this.
    Write(String),
}

/// An [`Effect`] as a host sees it: handles replaced by request ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// Tag 1.
    Count {
        /// Request id.
        id: u64,
    },
    /// Tag 2.
    Lookup {
        /// The name to look up.
        name: String,
        /// Request id.
        id: u64,
    },
    /// Tag 3.
    ReadLine {
        /// Request id.
        id: u64,
    },
    /// Tag 4.
    Sleep {
        /// How long, in milliseconds.
        millis: u64,
        /// Request id.
        id: u64,
    },
    /// Tag 5.
    Write {
        /// What to show.
        text: String,
    },
}

impl HostEffect for Effect {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Effect::Count(r) => (View::Count { id: r.id() }, Some(Pending::U64(r))),
            Effect::Lookup(name, r) => (View::Lookup { name, id: r.id() }, Some(Pending::Str(r))),
            Effect::ReadLine(r) => (View::ReadLine { id: r.id() }, Some(Pending::Str(r))),
            Effect::Sleep(after, r) => (
                View::Sleep {
                    millis: u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
                    id: r.id(),
                },
                Some(Pending::Unit(r)),
            ),
            Effect::Write(text) => (View::Write { text }, None),
        }
    }
}

/// Little-endian; `str` is `u32 len` + UTF-8; an awaiting effect's record ends
/// with its `u64` id.
impl Encode for View {
    fn encode(&self, w: &mut Writer) {
        match self {
            View::Count { id } => {
                w.u8(1);
                w.u64(*id);
            }
            View::Lookup { name, id } => {
                w.u8(2);
                w.str(name);
                w.u64(*id);
            }
            View::ReadLine { id } => {
                w.u8(3);
                w.u64(*id);
            }
            View::Sleep { millis, id } => {
                w.u8(4);
                w.u64(*millis);
                w.u64(*id);
            }
            View::Write { text } => {
                w.u8(5);
                w.str(text);
            }
        }
    }
}

/// The greeter. Generic over the outbox so that a test can supply its own.
#[derive(Debug)]
pub struct Greeter<O: Post<Effect>> {
    outbox: O,
}

impl<O: Post<Effect>> Greeter<O> {
    /// A greeter writing into `outbox`.
    pub const fn new(outbox: O) -> Self {
        Self { outbox }
    }
}

impl<O: Post<Effect>> Run for Greeter<O> {
    async fn turn(&mut self) -> ControlFlow<()> {
        let out = &self.outbox;

        out.tell(Effect::Write(String::from("Who are you?")));
        let name = out.ask(Effect::ReadLine).await;

        if name == "quit" {
            out.tell(Effect::Write(String::from("Bye.")));
            return ControlFlow::Break(());
        }

        let greeting = out.ask(|reply| Effect::Lookup(name.clone(), reply)).await;
        out.ask(|reply| Effect::Sleep(PAUSE, reply)).await;
        out.tell(Effect::Write(format!("{greeting}, {name}!")));

        let n = out.ask(Effect::Count).await;
        out.tell(Effect::Write(format!("(greeted {n} so far)")));

        ControlFlow::Continue(())
    }
}

/// The greeter, running against an outbox: what a skin hands to the host
/// layer's `new`.
///
/// No `Send` bound here, on purpose. Whether the future can cross threads
/// depends on `O`, and `Driver::new` decides it at the concrete call site by
/// auto-trait leakage; a bound here would have to be repeated on every
/// `Awaiting<T>` and would forbid a `!Send` outbox that never migrates.
pub fn greeter<O: Post<Effect> + 'static>(outbox: O) -> impl Future<Output = ()> {
    Greeter::new(outbox).run()
}

/// The same vocabulary, two waits at a time.
///
/// Reads a name, then looks up the greeting and counts _in one batch_; then
/// waits out the pause while reading the farewell, likewise. Each `join` puts
/// two requests on the wire before either is replied to, and the host may
/// reply in either order — which is what the request ids are for.
#[derive(Debug)]
pub struct Fanout<O: Post<Effect>> {
    outbox: O,
}

impl<O: Post<Effect>> Fanout<O> {
    /// A fan-out greeter writing into `outbox`.
    pub const fn new(outbox: O) -> Self {
        Self { outbox }
    }
}

impl<O: Post<Effect>> Run for Fanout<O> {
    async fn turn(&mut self) -> ControlFlow<()> {
        let out = &self.outbox;

        out.tell(Effect::Write(String::from("Who are you?")));
        let name = out.ask(Effect::ReadLine).await;

        let (greeting, n) = join(
            out.ask(|reply| Effect::Lookup(name.clone(), reply)),
            out.ask(Effect::Count),
        )
        .await;
        out.tell(Effect::Write(format!("{greeting}, {name}! (#{n})")));

        let ((), farewell) = join(
            out.ask(|reply| Effect::Sleep(PAUSE, reply)),
            out.ask(Effect::ReadLine),
        )
        .await;
        out.tell(Effect::Write(format!("Bye, {farewell}.")));

        ControlFlow::Break(())
    }
}

/// The fan-out greeter, running against an outbox.
pub fn fanout<O: Post<Effect> + 'static>(outbox: O) -> impl Future<Output = ()> {
    Fanout::new(outbox).run()
}

/// A scripted world for tests and demos: a fixed list of lines to read, a
/// greeting table, and a counter. Answers every effect at once.
pub mod world {
    use super::{Effect, View};
    use effect_routine::driver::Driver;
    use std::collections::VecDeque;

    /// Run a routine to completion against a script, collecting what it
    /// wrote. `Sleep` is answered instantly; `Lookup` says `Hello` to
    /// everyone; `Count` counts.
    #[must_use]
    pub fn transcript(mut driver: Driver<Effect>, script: &[&str]) -> Vec<String> {
        let mut lines = script.iter().map(|s| String::from(*s));
        let mut written = Vec::new();
        let mut greeted = 0;
        let mut queue: VecDeque<Effect> = driver.start().into();

        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                Effect::Write(text) => {
                    written.push(text);
                    continue;
                }
                Effect::ReadLine(reply) => {
                    let line = lines.next().unwrap_or_else(|| String::from("quit"));
                    driver.reply(reply, line)
                }
                Effect::Lookup(_, reply) => driver.reply(reply, String::from("Hello")),
                Effect::Sleep(_, reply) => driver.reply(reply, ()),
                Effect::Count(reply) => {
                    greeted += 1;
                    driver.reply(reply, greeted)
                }
            };
            queue.extend(more);
        }

        written
    }

    /// What the greeter should write for `script`, stated without running
    /// it: the specification the transcript is checked against.
    #[must_use]
    pub fn expected(script: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut greeted = 0;

        for name in script.iter().chain(core::iter::once(&"quit")) {
            out.push(String::from("Who are you?"));
            if *name == "quit" {
                out.push(String::from("Bye."));
                break;
            }
            greeted += 1;
            out.push(format!("Hello, {name}!"));
            out.push(format!("(greeted {greeted} so far)"));
        }

        out
    }

    /// Whether `view` awaits a reply, and of which id.
    #[must_use]
    pub const fn awaiting(view: &View) -> Option<u64> {
        match view {
            View::Count { id }
            | View::Lookup { id, .. }
            | View::ReadLine { id }
            | View::Sleep { id, .. } => Some(*id),
            View::Write { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{world::*, *};
    use effect_routine::driver::{Driver, Status};

    /// A batch of exactly `N` effects, or a test failure.
    fn exactly<const N: usize>(effects: Vec<Effect>) -> [Effect; N] {
        let len = effects.len();
        effects
            .try_into()
            .ok()
            .unwrap_or_else(|| panic!("expected a batch of {N}, got {len}"))
    }

    /// For any script, the greeter writes exactly what the specification
    /// says. `quit` anywhere in the script ends the conversation there.
    #[test]
    fn transcript_matches_specification() {
        bolero::check!()
            .with_type::<Vec<String>>()
            .for_each(|names| {
                let script: Vec<&str> = names.iter().map(String::as_str).collect();
                assert_eq!(transcript(Driver::new(greeter), &script), expected(&script));
            });
    }

    #[test]
    fn greeter_finishes_on_quit() {
        let written = transcript(Driver::new(greeter), &["alice", "quit"]);
        assert_eq!(
            written,
            [
                "Who are you?",
                "Hello, alice!",
                "(greeted 1 so far)",
                "Who are you?",
                "Bye."
            ]
        );
    }

    /// Fan-out puts two requests in one batch, and the transcript is the same
    /// whichever the host replies to first.
    #[test]
    fn fanout_is_order_independent() {
        bolero::check!()
            .with_type::<(bool, bool)>()
            .for_each(|(lookup_first, sleep_first)| {
                let mut driver = Driver::new(fanout);
                let mut written = Vec::new();

                let [Effect::Write(prompt), Effect::ReadLine(read)] = exactly(driver.start())
                else {
                    panic!("first batch: prompt + read");
                };
                written.push(prompt);

                let [Effect::Lookup(name, lookup), Effect::Count(count)] =
                    exactly(driver.reply(read, String::from("bob")))
                else {
                    panic!("second batch: lookup + count");
                };
                assert_eq!(name, "bob");

                let mut third = Vec::new();
                if *lookup_first {
                    third.extend(driver.reply(lookup, String::from("Hi")));
                    assert!(third.is_empty(), "one of two replied: nothing yet");
                    third.extend(driver.reply(count, 1));
                } else {
                    third.extend(driver.reply(count, 1));
                    assert!(third.is_empty(), "one of two replied: nothing yet");
                    third.extend(driver.reply(lookup, String::from("Hi")));
                }
                let [
                    Effect::Write(greeting),
                    Effect::Sleep(_, sleep),
                    Effect::ReadLine(read),
                ] = exactly(third)
                else {
                    panic!("third batch: greeting + sleep + read");
                };
                written.push(greeting);

                let mut fourth = Vec::new();
                if *sleep_first {
                    fourth.extend(driver.reply(sleep, ()));
                    assert!(fourth.is_empty());
                    fourth.extend(driver.reply(read, String::from("carol")));
                } else {
                    fourth.extend(driver.reply(read, String::from("carol")));
                    assert!(fourth.is_empty());
                    fourth.extend(driver.reply(sleep, ()));
                }
                let [Effect::Write(bye)] = exactly(fourth) else {
                    panic!("last batch: bye");
                };
                written.push(bye);

                assert_eq!(written, ["Who are you?", "Hi, bob! (#1)", "Bye, carol."]);
                assert_eq!(driver.status(), Status::Complete);
            });
    }
}
