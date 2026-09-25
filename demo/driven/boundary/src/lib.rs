//! The greeter's host vocabularies: its side of the boundary.
//!
//! `Greeter<Ctx<E>>` is the same routine as under `greeter_tokio`; only
//! the context differs. The routines are the `routines` crate; the context,
//! [`Ctx`](sans_effort_effects::ctx::Ctx), and the `Sleep`, `ReadLine`, and
//! `WriteLine` effects are `sans-effort-effects`, the standard library.
//! `Ctx` serves every wait by recording a request that carries a
//! [`ReplyHandle`](sans_effort_core::reply::handle::ReplyHandle) and
//! suspending; a host replies by id. The demo's own capabilities, `Count` and `Lookup`,
//! carry their effects and `Ctx` impls with their traits in `routines`. This
//! crate is the rest of what only the routine's author can write — the
//! vocabularies a host may offer, and how a host sees them — and nothing
//! else.
//! Stepping, the handle table, and the type check on replies are
//! `sans-effort-host`; the `extern "C"` surface is the _binding_ over both,
//! in `../cdylib`. (`../../native/wasm` needs neither: JS is a runtime host.)
//!
//! ```text
//!   routines               Greeter<C>: Step; Count, Lookup
//!                          (with their effects and Ctx impls)
//!                                     │
//!   sans-effort-effects    Ctx<E>; Sleep, ReadLine, WriteLine
//!                          (with their effects)
//!                                     │
//!                                     ▼
//!   this crate             Full · Quiet · View · Encode
//!                                     │
//!                                     ▼
//!   sans-effort-host  ──▶  cdylib (C ABI)
//! ```
//!
//! # The Host Chooses the Vocabulary
//!
//! Each wait is a request struct (`Lookup`, `ReadLine`, …) naming its
//! reply type through `Request`. `Ctx` implements each of the greeter's
//! traits for _any_ `E` that can carry the corresponding request —
//! `E: From<Asked<Lookup>>` — so a host defines `E` and meets each bound
//! with a `From` impl. [`Full`] carries all five; [`Quiet`] carries only
//! `Sleep` and `WriteLine`.
//!
//! That is attenuation, checked where the routine is built and visible on
//! the wire: `Ctx<Quiet>` does not implement `traits::Lookup`, so a
//! `Greeter` cannot be spawned under it, while a `Ticker` can — and a host
//! driving a `Quiet` machine knows from the type alone that tags 1–3 can
//! never appear.
//!
//! ```compile_fail,E0277
//! use sans_effort_core::{driver::Driver, step::Step};
//! use routines::greeter::Greeter;
//! use greeter_boundary::Quiet;
//! use sans_effort_effects::ctx::Ctx;
//!
//! // error[E0277]: the trait bound `Ctx<Quiet>: Lookup` is not satisfied
//! let _ = Driver::<Quiet>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());
//! ```
//!
//! # Encoding
//!
//! Little-endian; `str` is `u32 len` + UTF-8. One record per effect, which
//! the host layer wraps in a frame (`ABI.md` §Frames); an awaiting effect's
//! record ends with its `u64` request id. This table is the payload only.
//! `ReadLine` is fallible, so its reply is `bytes` holding an encoded
//! `Result<String, ReadLineError>`: `00 · str` for a line, `01 · 00` at the
//! end of input, `01 · 01` if the input failed.
//!
//! | tag | effect                   | reply with |
//! |-----|--------------------------|------------|
//! | 1   | `Count · id`             | `2 id u64` |
//! | 2   | `Lookup(str) · id`       | `1 id str` |
//! | 3   | `ReadLine · id`          | `4 id bytes` |
//! | 4   | `Sleep(u64 millis) · id` | `3 id`     |
//! | 5   | `WriteLine(str)`         | —          |
//! | 6   | `Spawned(u64 handle)`    | —          |
//! | 7   | `SpawnedPinned(u64 handle)` | —       |
//!
//! Tags 6 and 7 name a child the routine spawned, already registered in
//! `sans-effort-host`'s table: the host starts it. A `SpawnedPinned` child
//! stays on the thread that starts it; a `Spawned` one may migrate. They exist
//! with the `table` feature (the default), because registering a child needs
//! the table, and the table needs `std`; without it, `Full` offers no
//! spawning and the crate is `no_std`.

#![no_std]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use routines::traits::effect::{Count, Lookup};
use sans_effort_core::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    reply::Reply,
};
use sans_effort_effects::{
    console::effect::{ReadLine, WriteLine},
    request::Asked,
    time::effect::Sleep,
};

#[cfg(feature = "table")]
use sans_effort_effects::spawn::effect::{Spawn, SpawnPinned};

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
    WriteLine(WriteLine),
    /// Tag 6.
    #[cfg(feature = "table")]
    Spawn(Spawn<Full>),
    /// Tag 7.
    #[cfg(feature = "table")]
    SpawnPinned(SpawnPinned<Full>),
}

#[cfg(feature = "table")]
impl From<Spawn<Full>> for Full {
    fn from(spawn: Spawn<Full>) -> Self {
        Full::Spawn(spawn)
    }
}

#[cfg(feature = "table")]
impl From<SpawnPinned<Full>> for Full {
    fn from(spawn: SpawnPinned<Full>) -> Self {
        Full::SpawnPinned(spawn)
    }
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
        Full::WriteLine(write)
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
    WriteLine {
        /// What to show.
        text: String,
    },
    /// Tag 6: a child that may migrate between threads. Resume it to begin it.
    Spawned {
        /// The child's machine.
        handle: u64,
    },
    /// Tag 7: a child pinned to the thread that first resumes it. Resume it
    /// there.
    SpawnedPinned {
        /// The child's machine.
        handle: u64,
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
                Some(Vec::<u8>::pending(reply)),
            ),
            Full::Sleep(Asked {
                request: sleep,
                reply,
            }) => (
                View::Sleep {
                    millis: sleep.millis(),
                    id: reply.id(),
                },
                Some(<()>::pending(reply)),
            ),
            Full::WriteLine(WriteLine(text)) => (View::WriteLine { text }, None),
            #[cfg(feature = "table")]
            Full::Spawn(Spawn(child)) => (
                View::Spawned {
                    handle: sans_effort_host::table::new_boxed(move |outbox| child.start(outbox)),
                },
                None,
            ),
            #[cfg(feature = "table")]
            Full::SpawnPinned(SpawnPinned(child)) => (
                View::SpawnedPinned {
                    handle: sans_effort_host::table::park_pinned(move |outbox| child.start(outbox)),
                },
                None,
            ),
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
            View::WriteLine { text } => {
                w.u8(5);
                w.str(text);
            }
            View::Spawned { handle } => {
                w.u8(6);
                w.u64(*handle);
            }
            View::SpawnedPinned { handle } => {
                w.u8(7);
                w.u64(*handle);
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
    WriteLine(WriteLine),
}

impl From<Asked<Sleep>> for Quiet {
    fn from(asked: Asked<Sleep>) -> Self {
        Quiet::Sleep(asked)
    }
}

impl From<WriteLine> for Quiet {
    fn from(write: WriteLine) -> Self {
        Quiet::WriteLine(write)
    }
}

impl HostEffect for Quiet {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Quiet::Sleep(asked) => Full::Sleep(asked).split(),
            Quiet::WriteLine(write) => Full::WriteLine(write).split(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The boundary crate's test story: the same routine, through a
    //! `Driver`, read off as data. Where `greeter`'s tests assert on what a
    //! mock recorded, these assert on the effects a host would see — the
    //! thing the wire exists to carry.

    #![expect(
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert their preconditions; let-else arms name the batch they expected"
    )]

    extern crate std;

    use super::*;
    use alloc::{collections::VecDeque, format, vec, vec::Vec};
    use core::future::Future;
    use routines::{PAUSE, fanout::Fanout, greeter::Greeter, ticker::Ticker};
    use sans_effort_core::{
        driver::{Driver, outbox::Outbox, status::Status},
        step::Step,
    };
    use sans_effort_effects::{console::ReadLineError, ctx::Ctx};

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
        let mut lines = script.iter().copied();
        let mut seen = Vec::new();
        let mut written = Vec::new();
        let mut greeted = 0;
        let mut queue: VecDeque<Full> = driver.resume().into();

        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                Full::WriteLine(WriteLine(text)) => {
                    seen.push(View::WriteLine { text: text.clone() });
                    written.push(text);
                    continue;
                }
                Full::ReadLine(Asked { reply, .. }) => {
                    seen.push(View::ReadLine { id: reply.id() });
                    let line = lines.next().ok_or(ReadLineError::Closed);
                    driver.reply(reply, ReadLine::reply(line))
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
                #[cfg(feature = "table")]
                Full::Spawn(_) | Full::SpawnPinned(_) => panic!("the greeter never spawns"),
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
                View::WriteLine { .. } | View::Spawned { .. } | View::SpawnedPinned { .. } => None,
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
                    Full::WriteLine(WriteLine(prompt)),
                    Full::ReadLine(Asked { reply: read, .. }),
                ] = exactly(driver.resume())
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
                ] = exactly(driver.reply(read, ReadLine::reply(Ok("bob"))))
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
                    Full::WriteLine(WriteLine(greeting)),
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
                    fourth.extend(driver.reply(read, ReadLine::reply(Ok("carol"))));
                } else {
                    fourth.extend(driver.reply(read, ReadLine::reply(Ok("carol"))));
                    assert!(fourth.is_empty());
                    fourth.extend(driver.reply(sleep, ()));
                }
                let [Full::WriteLine(WriteLine(bye))] = exactly(fourth) else {
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
        let mut queue: VecDeque<Quiet> = driver.resume().into();

        while let Some(effect) = queue.pop_front() {
            match effect {
                Quiet::WriteLine(WriteLine(text)) => written.push(text),
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
    fn exactly<const N: usize, S: Into<Vec<Full>>>(effects: S) -> [Full; N] {
        let effects = effects.into();
        let len = effects.len();
        effects
            .try_into()
            .ok()
            .unwrap_or_else(|| panic!("expected a batch of {N}, got {len}"))
    }

    /// A deterministic Rust host for machines that spawn and message each
    /// other: every request answered at once, spawned children started in
    /// the order they appear, and machines resumed in the order their drivers
    /// report them woken — the same hooks the host table installs.
    #[cfg(feature = "table")]
    mod router {
        use super::*;
        use routines::{front_desk::FrontDesk, ping_pong::PingPong};
        use sans_effort_core::{
            driver::{LocalDriver, Yield},
            reply::{Reply, handle::ReplyHandle},
        };

        /// A machine the router drives: migrating or pinned. All run on the
        /// test's one thread.
        enum Machine {
            Migrating(Driver<Full>),
            Pinned(LocalDriver<Full>),
        }

        impl Machine {
            fn resume(&mut self) -> Yield<Full> {
                match self {
                    Machine::Migrating(d) => d.resume(),
                    Machine::Pinned(d) => d.resume(),
                }
            }

            fn reply<T: Reply>(&mut self, handle: ReplyHandle<T>, value: T) -> Yield<Full> {
                match self {
                    Machine::Migrating(d) => d.reply(handle, value),
                    Machine::Pinned(d) => d.reply(handle, value),
                }
            }

            fn status(&self) -> Status {
                match self {
                    Machine::Migrating(d) => d.status(),
                    Machine::Pinned(d) => d.status(),
                }
            }

            /// Report wakes as this machine's index in `woken`.
            fn report_wakes(&self, at: usize, woken: &Woken) {
                let woken = Woken::clone(woken);
                let hook = move || woken.lock().expect("woken").push_back(at);
                match self {
                    Machine::Migrating(d) => d.on_wake(hook),
                    Machine::Pinned(d) => d.on_wake(hook),
                }
            }
        }

        /// Which machines woke, in order, by index.
        type Woken = std::sync::Arc<std::sync::Mutex<VecDeque<usize>>>;

        /// Run `root` and everything it spawns until nothing can progress:
        /// what each wrote, in order, and whether every machine completed.
        fn route(root: Driver<Full>, script: &[&str]) -> (Vec<String>, bool) {
            let mut lines = script.iter().copied();
            let woken = Woken::default();
            let mut machines = Vec::new();
            let (at, first) = adopt(&mut machines, Machine::Migrating(root), &woken);
            let mut queue: VecDeque<(usize, Full)> = first.into_iter().map(|e| (at, e)).collect();
            let mut written = Vec::new();

            loop {
                while let Some((at, effect)) = queue.pop_front() {
                    let (from, more) = match effect {
                        Full::WriteLine(WriteLine(text)) => {
                            written.push(text);
                            continue;
                        }
                        Full::Spawn(Spawn(child)) => adopt(
                            &mut machines,
                            Machine::Migrating(Driver::from_boxed(|outbox| child.start(outbox))),
                            &woken,
                        ),
                        Full::SpawnPinned(SpawnPinned(child)) => adopt(
                            &mut machines,
                            Machine::Pinned(LocalDriver::from_boxed(|outbox| child.start(outbox))),
                            &woken,
                        ),
                        Full::Sleep(Asked { reply, .. }) => {
                            (at, nth(&mut machines, at).reply(reply, ()))
                        }
                        Full::ReadLine(Asked { reply, .. }) => {
                            let line = lines.next().ok_or(ReadLineError::Closed);
                            (
                                at,
                                nth(&mut machines, at).reply(reply, ReadLine::reply(line)),
                            )
                        }
                        Full::Lookup(Asked {
                            request: Lookup(name),
                            reply,
                        }) => {
                            let greeting = match name.as_str() {
                                "alice" => "Hello",
                                "bob" => "Hi",
                                _ => "Greetings",
                            };
                            (
                                at,
                                nth(&mut machines, at).reply(reply, String::from(greeting)),
                            )
                        }
                        Full::Count(Asked { reply, .. }) => {
                            (at, nth(&mut machines, at).reply(reply, 0))
                        }
                    };
                    queue.extend(more.into_iter().map(|e| (from, e)));
                }

                // Nothing queued: resume the next machine that woke. None
                // woke, and nothing is queued: nothing can happen again.
                let next = woken.lock().expect("woken").pop_front();
                let Some(at) = next else {
                    let done = machines.iter().all(|m| m.status() == Status::Complete);
                    return (written, done);
                };
                let step = nth(&mut machines, at).resume();
                queue.extend(step.into_iter().map(|e| (at, e)));
            }
        }

        /// Begin a spawned machine and keep it: its index, and what its first
        /// resume yielded.
        fn adopt(
            machines: &mut Vec<Machine>,
            mut machine: Machine,
            woken: &Woken,
        ) -> (usize, Yield<Full>) {
            let at = machines.len();
            machine.report_wakes(at, woken);
            let step = machine.resume();
            machines.push(machine);
            (at, step)
        }

        fn nth(machines: &mut [Machine], at: usize) -> &mut Machine {
            machines.get_mut(at).expect("an index the router issued")
        }

        #[test]
        fn ping_pong_across_two_machines() {
            let root = Driver::new(|outbox| PingPong::new(Ctx::<Full>::new(outbox), 3).run());
            let (written, done) = route(root, &[]);
            assert_eq!(
                written,
                ["ping 1, pong 1", "ping 2, pong 2", "ping 3, pong 3"]
            );
            assert!(done, "the parent returned, so the child's pings ended too");
        }

        #[test]
        fn front_desk_greets_in_arrival_order() {
            let root = Driver::new(|outbox| FrontDesk::new(Ctx::<Full>::new(outbox)).run());
            let (written, done) = route(root, &["alice", "bob", "zed"]);
            assert_eq!(
                written,
                ["Hello, alice!", "Hi, bob!", "Greetings, zed!", "Closed."]
            );
            assert!(done, "every clerk replied and finished");
        }
    }
}
