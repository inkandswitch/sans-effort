//! The greeter's reifying context and host vocabularies: its side of the boundary.
//!
//! `Greeter<Ctx<E>>` is the same routine as under `greeter_tokio`; only
//! the context differs. The routines are the `routines` crate. [`Ctx`] serves every wait by recording a request that
//! carries a [`ReplyHandle`](effect_routine::reply::handle::ReplyHandle) and suspending; a host replies by id. This crate
//! is what only the routine's author can write — which requests exist, how
//! the five traits map onto them, how a host sees them — and nothing else.
//! Stepping, the handle table, and the type check on replies are
//! `effect_routine_host`; the `extern "C"` or wasm-bindgen surface is a
//! _skin_ over both, one per binding, in `../cdylib` and `../wasm`.
//!
//! ```text
//!   routines   Greeter<C>: Run   ──▶   this crate   Ctx<E> · Full · View · Encode   ──▶   effect_routine_host
//!                                                                                                  │
//!                                                                     ┌────────────────────────────┴──────────┐
//!                                                                  cdylib  greeter_* (C ABI)          wasm  Greeter class
//! ```
//!
//! # The host chooses the vocabulary
//!
//! Each wait is a request struct ([`Lookup`], [`ReadLine`], …) naming its
//! reply type through [`Request`]. [`Ctx`] implements each of the greeter's
//! traits for _any_ `E` that can carry the corresponding request —
//! `E: From<Asked<Lookup>>` — so a host defines `E` and meets each bound
//! with a `From` impl. [`Full`] carries all five; [`Quiet`] carries only
//! `Sleep` and `Write`.
//!
//! That is attenuation, checked where the routine is built and visible on the
//! wire: `Ctx<Quiet>` does not implement `traits::Lookup`, so a `Greeter` cannot
//! be spawned under it, while a `Ticker` can — and a host driving a `Quiet`
//! machine knows from the type alone that tags 1–3 can never appear.
//!
//! ```compile_fail,E0277
//! use effect_routine::{driver::Driver, run::Run};
//! use routines::greeter::Greeter;
//! use greeter_boundary::{Ctx, Quiet};
//!
//! // error[E0277]: the trait bound `Ctx<Quiet>: Lookup` is not satisfied
//! let _ = Driver::<Quiet>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());
//! ```
//!
//! # Encoding
//!
//! Little-endian; `str` is `u32 len` + UTF-8. One record per effect; an
//! awaiting effect's record ends with its `u64` request id:
//!
//! | tag | effect                   | reply with |
//! |-----|--------------------------|------------|
//! | 1   | `Count · id`             | `2 id u64` |
//! | 2   | `Lookup(str) · id`       | `1 id str` |
//! | 3   | `ReadLine · id`          | `1 id str` |
//! | 4   | `Sleep(u64 millis) · id` | `3 id`     |
//! | 5   | `Write(str)`             | —          |

#![no_std]

extern crate alloc;

use alloc::string::String;
use core::time::Duration;
use effect_routine::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::outbox::Outbox,
    reply::Reply,
    request::{Asked, Request},
};
use routines::traits;

// ---- requests: one per awaited capability, plus the one message ----------

/// How many greetings so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count;

/// The greeting word for a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup(pub String);

/// The next line of input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLine;

/// Wake after a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sleep(pub Duration);

/// Show a line. Fire-and-forget; not a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteLine(pub String);

impl Request for Count {
    type Reply = u64;
}

impl Request for Lookup {
    type Reply = String;
}

impl Request for ReadLine {
    type Reply = String;
}

impl Request for Sleep {
    type Reply = ();
}

// ---- the reifying context -------------------------------------------------

/// A context that serves every wait by asking the host.
///
/// Generic over the host's vocabulary `E`. Each trait impl below holds
/// exactly when `E` can carry that trait's request, so the set of traits
/// `Ctx<E>` implements _is_ the set of capabilities the host has agreed to
/// provide.
#[derive(Debug)]
pub struct Ctx<E> {
    outbox: Outbox<E>,
}

impl<E> Ctx<E> {
    /// A context writing into `outbox`.
    #[must_use]
    pub const fn new(outbox: Outbox<E>) -> Self {
        Self { outbox }
    }
}

impl<E: From<Asked<Sleep>>> traits::Sleep for Ctx<E> {
    async fn sleep(&self, duration: Duration) {
        self.outbox.request(Sleep(duration)).await;
    }
}

impl<E: From<Asked<Count>>> traits::Count for Ctx<E> {
    async fn count(&self) -> u64 {
        self.outbox.request(Count).await
    }
}

impl<E: From<Asked<Lookup>>> traits::Lookup for Ctx<E> {
    async fn lookup(&self, name: String) -> String {
        self.outbox.request(Lookup(name)).await
    }
}

impl<E: From<Asked<ReadLine>>> traits::ReadLine for Ctx<E> {
    async fn read_line(&self) -> String {
        self.outbox.request(ReadLine).await
    }
}

impl<E: From<WriteLine>> traits::WriteLine for Ctx<E> {
    fn write(&self, line: String) {
        self.outbox.notify(WriteLine(line));
    }
}

// ---- Full: a host that offers everything ----------------------------------

/// The vocabulary of a host that offers all five capabilities.
#[derive(Debug)]
pub enum Full {
    /// Tag 1.
    Count(Asked<Count>),
    /// Tag 2.
    Lookup(Asked<Lookup>),
    /// Tag 3.
    ReadLine(Asked<ReadLine>),
    /// Tag 4.
    Sleep(Asked<Sleep>),
    /// Tag 5.
    Write(WriteLine),
}

impl From<Asked<Count>> for Full {
    fn from(asked: Asked<Count>) -> Self {
        Full::Count(asked)
    }
}

impl From<Asked<Lookup>> for Full {
    fn from(asked: Asked<Lookup>) -> Self {
        Full::Lookup(asked)
    }
}

impl From<Asked<ReadLine>> for Full {
    fn from(asked: Asked<ReadLine>) -> Self {
        Full::ReadLine(asked)
    }
}

impl From<Asked<Sleep>> for Full {
    fn from(asked: Asked<Sleep>) -> Self {
        Full::Sleep(asked)
    }
}

impl From<WriteLine> for Full {
    fn from(write: WriteLine) -> Self {
        Full::Write(write)
    }
}

/// A [`Full`] effect as a host sees it: handles replaced by request ids.
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

impl HostEffect for Full {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Full::Count(Asked { reply, .. }) => {
                (View::Count { id: reply.id() }, Some(u64::pending(reply)))
            }
            Full::Lookup(Asked {
                request: Lookup(name),
                reply,
            }) => (
                View::Lookup {
                    name,
                    id: reply.id(),
                },
                Some(String::pending(reply)),
            ),
            Full::ReadLine(Asked { reply, .. }) => (
                View::ReadLine { id: reply.id() },
                Some(String::pending(reply)),
            ),
            Full::Sleep(Asked {
                request: Sleep(after),
                reply,
            }) => (
                View::Sleep {
                    millis: u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
                    id: reply.id(),
                },
                Some(<()>::pending(reply)),
            ),
            Full::Write(WriteLine(text)) => (View::Write { text }, None),
        }
    }
}

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

// ---- Quiet: a host that offers only a clock and an output -----------------

/// The vocabulary of a host that offers only `Sleep` and `WriteLine`. A
/// [`Ticker`](routines::ticker::Ticker) runs under it; a
/// [`Greeter`](routines::greeter::Greeter) does not compile against it.
#[derive(Debug)]
pub enum Quiet {
    /// Tag 4.
    Sleep(Asked<Sleep>),
    /// Tag 5.
    Write(WriteLine),
}

impl From<Asked<Sleep>> for Quiet {
    fn from(asked: Asked<Sleep>) -> Self {
        Quiet::Sleep(asked)
    }
}

impl From<WriteLine> for Quiet {
    fn from(write: WriteLine) -> Self {
        Quiet::Write(write)
    }
}

impl HostEffect for Quiet {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Quiet::Sleep(asked) => Full::Sleep(asked).split(),
            Quiet::Write(write) => Full::Write(write).split(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The wire's test story: the same routine, through a `Driver`, read off
    //! as data. Where `greeter`'s tests assert on what a mock recorded, these
    //! assert on the effects a host would see — the thing the wire exists to
    //! carry.

    #![expect(
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert their preconditions; let-else arms name the batch they expected"
    )]

    extern crate std;

    use super::*;
    use alloc::{collections::VecDeque, format, vec, vec::Vec};
    use core::future::Future;
    use effect_routine::{
        driver::{Driver, status::Status},
        run::Run,
    };
    use routines::{PAUSE, fanout::Fanout, greeter::Greeter, ticker::Ticker};

    fn greeter(outbox: Outbox<Full>) -> impl Future<Output = ()> {
        Greeter::new(Ctx::new(outbox)).run()
    }

    fn fanout(outbox: Outbox<Full>) -> impl Future<Output = ()> {
        Fanout::new(Ctx::new(outbox)).run()
    }

    fn ticker(outbox: Outbox<Quiet>) -> impl Future<Output = ()> {
        Ticker::new(Ctx::new(outbox), 3).run()
    }

    /// A scripted host: answers every request at once, records what it was
    /// shown, and returns what the routine wrote.
    fn transcript(mut driver: Driver<Full>, script: &[&str]) -> (Vec<View>, Vec<String>) {
        let mut lines = script.iter().map(|s| String::from(*s));
        let mut seen = Vec::new();
        let mut written = Vec::new();
        let mut greeted = 0;
        let mut queue: VecDeque<Full> = driver.start().into();

        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                Full::Write(WriteLine(text)) => {
                    seen.push(View::Write { text: text.clone() });
                    written.push(text);
                    continue;
                }
                Full::ReadLine(Asked { reply, .. }) => {
                    seen.push(View::ReadLine { id: reply.id() });
                    let line = lines.next().unwrap_or_else(|| String::from("quit"));
                    driver.reply(reply, line)
                }
                Full::Lookup(Asked {
                    request: Lookup(name),
                    reply,
                }) => {
                    seen.push(View::Lookup {
                        name: name.clone(),
                        id: reply.id(),
                    });
                    driver.reply(reply, format!("Hello to {name}"))
                }
                Full::Sleep(Asked {
                    request: Sleep(after),
                    reply,
                }) => {
                    seen.push(View::Sleep {
                        millis: u64::try_from(after.as_millis()).expect("a demo pause fits in u64"),
                        id: reply.id(),
                    });
                    driver.reply(reply, ())
                }
                Full::Count(Asked { reply, .. }) => {
                    seen.push(View::Count { id: reply.id() });
                    greeted += 1;
                    driver.reply(reply, greeted)
                }
            };
            queue.extend(more);
        }

        assert_eq!(driver.status(), Status::Complete);
        (seen, written)
    }

    /// What the greeter should write for `script`, stated without running it.
    fn expected(script: &[&str]) -> Vec<String> {
        let mut out = Vec::new();

        for (i, name) in script.iter().take_while(|n| **n != "quit").enumerate() {
            out.extend([
                String::from("Who are you?"),
                format!("Hello to {name}, {name}!"),
                format!("(greeted {} so far)", i + 1),
            ]);
        }

        out.extend([String::from("Who are you?"), String::from("Bye.")]);
        out
    }

    #[test]
    fn greeter_through_the_wire_on_any_script() {
        bolero::check!()
            .with_type::<Vec<String>>()
            .for_each(|names| {
                let script: Vec<&str> = names.iter().map(String::as_str).collect();
                let (_, written) = transcript(Driver::new(greeter), &script);
                assert_eq!(written, expected(&script));
            });
    }

    /// Request ids are minted in order from 1, and every awaiting effect the
    /// host sees carries one.
    #[test]
    fn ids_count_up_from_one() {
        let (seen, _) = transcript(Driver::new(greeter), &["alice"]);
        let ids: Vec<u64> = seen
            .iter()
            .filter_map(|v| match v {
                View::Count { id }
                | View::Lookup { id, .. }
                | View::ReadLine { id }
                | View::Sleep { id, .. } => Some(*id),
                View::Write { .. } => None,
            })
            .collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
    }

    /// Fan-out puts two requests in one batch, and the transcript is the
    /// same whichever the host replies to first.
    #[test]
    fn fanout_is_order_independent() {
        bolero::check!()
            .with_type::<(bool, bool)>()
            .for_each(|(lookup_first, sleep_first)| {
                let mut driver = Driver::new(fanout);
                let mut written = Vec::new();

                let [
                    Full::Write(WriteLine(prompt)),
                    Full::ReadLine(Asked { reply: read, .. }),
                ] = exactly(driver.start())
                else {
                    panic!("first batch: prompt + read");
                };
                written.push(prompt);

                let [
                    Full::Lookup(Asked {
                        request: Lookup(name),
                        reply: lookup,
                    }),
                    Full::Count(Asked { reply: count, .. }),
                ] = exactly(driver.reply(read, String::from("bob")))
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
                    Full::Write(WriteLine(greeting)),
                    Full::Sleep(Asked { reply: sleep, .. }),
                    Full::ReadLine(Asked { reply: read, .. }),
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
                let [Full::Write(WriteLine(bye))] = exactly(fourth) else {
                    panic!("last batch: bye");
                };
                written.push(bye);

                assert_eq!(written, ["Who are you?", "Hi, bob! (#1)", "Bye, carol."]);
                assert_eq!(driver.status(), Status::Complete);
            });
    }

    /// A `Quiet` host runs the ticker, and only ever sees tags 4 and 5.
    #[test]
    fn ticker_under_quiet() {
        let mut driver = Driver::new(ticker);
        let mut written = Vec::new();
        let mut queue: VecDeque<Quiet> = driver.start().into();

        while let Some(effect) = queue.pop_front() {
            match effect {
                Quiet::Write(WriteLine(text)) => written.push(text),
                Quiet::Sleep(Asked { request, reply }) => {
                    assert_eq!(request, Sleep(PAUSE));
                    queue.extend(driver.reply(reply, ()));
                }
            }
        }

        assert_eq!(
            written,
            vec!["tick (2 left)", "tick (1 left)", "tick (0 left)"]
        );
        assert_eq!(driver.status(), Status::Complete);
    }

    /// A batch of exactly `N` effects, or a test failure.
    fn exactly<const N: usize>(effects: Vec<Full>) -> [Full; N] {
        let len = effects.len();
        effects
            .try_into()
            .ok()
            .unwrap_or_else(|| panic!("expected a batch of {N}, got {len}"))
    }
}
