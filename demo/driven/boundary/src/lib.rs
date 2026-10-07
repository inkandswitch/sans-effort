//! The greeter's host vocabularies: its side of the boundary.
//!
//! `Greeter<Ctx<E>>` is the same routine as under `greeter_tokio`; only
//! the context differs. The routines are the `routines` crate; the context,
//! [`Ctx`](sans_effort_effects::ctx::Ctx), and the `Sleep`, `ReadLine`, and
//! `WriteLine` effects are `sans-effort-effects`, the standard library.
//! `Ctx` serves every wait by recording a request that carries a
//! [`ReplyHandle`](sans_effort_core::reply::handle::ReplyHandle) and
//! suspending; a host replies by id. The demo's own effect traits, `Count`
//! and `Lookup`, carry their effects and `Ctx` impls with their traits in
//! `routines`. This crate is the rest of what only the routine's author can
//! write — the vocabularies a host may offer, and how a host sees them — and
//! nothing else.
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
//! reply type through `Ask`. `Ctx` implements each of the greeter's
//! traits for _any_ `E` that can carry the corresponding request —
//! `E: From<Asked<Lookup>>` — so a host defines `E` and meets each bound
//! with a `From` impl. [`Full`] carries all five; [`Quiet`] carries only
//! `Sleep` and `WriteLine`.
//!
//! That is attenuation, checked where the routine is built and visible on
//! the wire: `Ctx<Quiet>` does not implement `effects::lookup::Lookup`, so a
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
//! | 8   | `Now · id`               | `2 id u64` (nanoseconds since the epoch) |
//! | 9   | `Random(u32 len) · id`   | `4 id bytes` (exactly `len`) |
//! | 10  | `Var(str name) · id`     | `4 id bytes`: `00` unset, `01 · str` |
//! | 11  | `ReadFile(str path) · id` | `4 id bytes`: `00 · bytes`, or `01 · u8` error |
//! | 12  | `WriteFile(str path, bytes) · id` | `4 id bytes`: `00`, or `01 · u8` error |
//!
//! A file error is one byte: `00` not found, `01` permission denied, `02`
//! anything else. `bytes` inside a payload is `u32 len` + the bytes.
//!
//! Tags 6 and 7 name a child the routine spawned, already registered in
//! `sans-effort-host`'s table: the host resumes it to begin it. A
//! `SpawnedPinned` child stays on the thread that first resumes it; a
//! `Spawned` one may migrate. They exist
//! with the `table` feature (the default), because registering a child needs
//! the table, and the table needs `std`; without it, `Full` offers no
//! spawning and the crate is `no_std`.

#![no_std]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use routines::effects::{count::CountEffect, lookup::LookupEffect};
use sans_effort_core::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    reply::Answer,
};
use sans_effort_effects::{
    ask::Asked,
    console::{ReadLineEffect, WriteLineEffect},
    env::VarEffect,
    fs::{ReadFileEffect, WriteFileEffect},
    random::RandomEffect,
    time::{NowEffect, SleepEffect},
};

#[cfg(feature = "table")]
use sans_effort_effects::spawn::{SpawnEffect, SpawnPinnedEffect};

// ---- Full: a host that offers everything ----------------------------------

/// The vocabulary of a host that offers every effect trait the demo's routines
/// use.
#[derive(Debug)]
pub enum Full {
    /// Tag 1.
    Count(Asked<CountEffect>),
    /// Tag 2.
    Lookup(Asked<LookupEffect>),
    /// Tag 3.
    ReadLine(Asked<ReadLineEffect>),
    /// Tag 4.
    Sleep(Asked<SleepEffect>),
    /// Tag 5.
    WriteLine(WriteLineEffect),
    /// Tag 8.
    Now(Asked<NowEffect>),
    /// Tag 9.
    Random(Asked<RandomEffect>),
    /// Tag 10.
    Var(Asked<VarEffect>),
    /// Tag 11.
    ReadFile(Asked<ReadFileEffect>),
    /// Tag 12.
    WriteFile(Asked<WriteFileEffect>),
    /// Tag 6.
    #[cfg(feature = "table")]
    Spawn(SpawnEffect<Full>),
    /// Tag 7.
    #[cfg(feature = "table")]
    SpawnPinned(SpawnPinnedEffect<Full>),
}

#[cfg(feature = "table")]
impl From<SpawnEffect<Full>> for Full {
    fn from(spawn: SpawnEffect<Full>) -> Self {
        Full::Spawn(spawn)
    }
}

#[cfg(feature = "table")]
impl From<SpawnPinnedEffect<Full>> for Full {
    fn from(spawn: SpawnPinnedEffect<Full>) -> Self {
        Full::SpawnPinned(spawn)
    }
}

impl From<Asked<CountEffect>> for Full {
    fn from(asked: Asked<CountEffect>) -> Self {
        Full::Count(asked)
    }
}

impl From<Asked<LookupEffect>> for Full {
    fn from(asked: Asked<LookupEffect>) -> Self {
        Full::Lookup(asked)
    }
}

impl From<Asked<ReadLineEffect>> for Full {
    fn from(asked: Asked<ReadLineEffect>) -> Self {
        Full::ReadLine(asked)
    }
}

impl From<Asked<SleepEffect>> for Full {
    fn from(asked: Asked<SleepEffect>) -> Self {
        Full::Sleep(asked)
    }
}

impl From<Asked<NowEffect>> for Full {
    fn from(asked: Asked<NowEffect>) -> Self {
        Full::Now(asked)
    }
}

impl From<Asked<RandomEffect>> for Full {
    fn from(asked: Asked<RandomEffect>) -> Self {
        Full::Random(asked)
    }
}

impl From<Asked<VarEffect>> for Full {
    fn from(asked: Asked<VarEffect>) -> Self {
        Full::Var(asked)
    }
}

impl From<Asked<ReadFileEffect>> for Full {
    fn from(asked: Asked<ReadFileEffect>) -> Self {
        Full::ReadFile(asked)
    }
}

impl From<Asked<WriteFileEffect>> for Full {
    fn from(asked: Asked<WriteFileEffect>) -> Self {
        Full::WriteFile(asked)
    }
}

impl From<WriteLineEffect> for Full {
    fn from(write: WriteLineEffect) -> Self {
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
    /// Tag 8.
    Now {
        /// Request id.
        id: u64,
    },
    /// Tag 9.
    Random {
        /// How many bytes.
        len: u32,
        /// Request id.
        id: u64,
    },
    /// Tag 10.
    Var {
        /// The variable's name.
        name: String,
        /// Request id.
        id: u64,
    },
    /// Tag 11.
    ReadFile {
        /// The file.
        path: String,
        /// Request id.
        id: u64,
    },
    /// Tag 12.
    WriteFile {
        /// The file.
        path: String,
        /// Its new contents.
        bytes: Vec<u8>,
        /// Request id.
        id: u64,
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
                request: LookupEffect(name),
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
                Some(Answer::pending(reply)),
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
            Full::WriteLine(WriteLineEffect(text)) => (View::WriteLine { text }, None),
            Full::Now(Asked { reply, .. }) => {
                (View::Now { id: reply.id() }, Some(Answer::pending(reply)))
            }
            Full::Random(Asked {
                request: RandomEffect(len),
                reply,
            }) => (
                View::Random {
                    len,
                    id: reply.id(),
                },
                Some(Answer::pending(reply)),
            ),
            Full::Var(Asked {
                request: VarEffect(name),
                reply,
            }) => (
                View::Var {
                    name,
                    id: reply.id(),
                },
                Some(Answer::pending(reply)),
            ),
            Full::ReadFile(Asked {
                request: ReadFileEffect(path),
                reply,
            }) => (
                View::ReadFile {
                    path,
                    id: reply.id(),
                },
                Some(Answer::pending(reply)),
            ),
            Full::WriteFile(Asked {
                request: WriteFileEffect { path, bytes },
                reply,
            }) => (
                View::WriteFile {
                    path,
                    bytes,
                    id: reply.id(),
                },
                Some(Answer::pending(reply)),
            ),
            #[cfg(feature = "table")]
            Full::Spawn(SpawnEffect(child)) => (
                View::Spawned {
                    handle: sans_effort_host::table::new_boxed(move |outbox| child.start(outbox)),
                },
                None,
            ),
            #[cfg(feature = "table")]
            Full::SpawnPinned(SpawnPinnedEffect(child)) => (
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
            View::Now { id } => {
                w.u8(8);
                w.u64(*id);
            }
            View::Random { len, id } => {
                w.u8(9);
                w.u32(*len);
                w.u64(*id);
            }
            View::Var { name, id } => {
                w.u8(10);
                w.str(name);
                w.u64(*id);
            }
            View::ReadFile { path, id } => {
                w.u8(11);
                w.str(path);
                w.u64(*id);
            }
            View::WriteFile { path, bytes, id } => {
                w.u8(12);
                w.str(path);
                w.bytes(bytes);
                w.u64(*id);
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
    Sleep(Asked<SleepEffect>),
    /// Tag 5.
    WriteLine(WriteLineEffect),
}

impl From<Asked<SleepEffect>> for Quiet {
    fn from(asked: Asked<SleepEffect>) -> Self {
        Quiet::Sleep(asked)
    }
}

impl From<WriteLineEffect> for Quiet {
    fn from(write: WriteLineEffect) -> Self {
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

    extern crate std;

    use super::*;
    use alloc::{collections::VecDeque, format, vec, vec::Vec};
    use core::future::Future;
    use routines::{PAUSE, greeter::Greeter, ticker::Ticker};
    use sans_effort_core::{
        driver::{Driver, outbox::Outbox, status::Status},
        step::Step,
    };
    use sans_effort_effects::{console::ReadLineError, ctx::Ctx};
    use testresult::TestResult;

    fn greeter(outbox: Outbox<Full>) -> impl Future<Output = ()> {
        Greeter::new(Ctx::new(outbox)).run()
    }

    fn ticker(outbox: Outbox<Quiet>) -> impl Future<Output = ()> {
        Ticker::new(Ctx::new(outbox), 3).run()
    }

    /// A scripted host: answers every request at once, records what it was
    /// shown, and returns what the routine wrote.
    fn transcript(
        mut driver: Driver<Full>,
        script: &[&str],
    ) -> TestResult<(Vec<View>, Vec<String>)> {
        let mut lines = script.iter().copied();
        let mut seen = Vec::new();
        let mut written = Vec::new();
        let mut greeted = 0;
        let mut queue: VecDeque<Full> = driver.resume().into();

        while let Some(effect) = queue.pop_front() {
            let more = match effect {
                Full::WriteLine(WriteLineEffect(text)) => {
                    seen.push(View::WriteLine { text: text.clone() });
                    written.push(text);
                    continue;
                }
                Full::ReadLine(Asked { reply, .. }) => {
                    seen.push(View::ReadLine { id: reply.id() });
                    let line = lines.next().ok_or(ReadLineError::Closed);
                    driver.reply(reply, line.map(String::from))
                }
                Full::Lookup(Asked {
                    request: LookupEffect(name),
                    reply,
                }) => {
                    seen.push(View::Lookup {
                        name: name.clone(),
                        id: reply.id(),
                    });
                    driver.reply(reply, format!("Hello to {name}"))
                }
                Full::Sleep(Asked {
                    request: SleepEffect(after),
                    reply,
                }) => {
                    seen.push(View::Sleep {
                        millis: u64::try_from(after.as_millis())?,
                        id: reply.id(),
                    });
                    driver.reply(reply, ())
                }
                Full::Count(Asked { reply, .. }) => {
                    seen.push(View::Count { id: reply.id() });
                    greeted += 1;
                    driver.reply(reply, greeted)
                }
                Full::Now(_)
                | Full::Random(_)
                | Full::Var(_)
                | Full::ReadFile(_)
                | Full::WriteFile(_) => return Err("the greeter never keeps a journal")?,
                #[cfg(feature = "table")]
                Full::Spawn(_) | Full::SpawnPinned(_) => return Err("the greeter never spawns")?,
            };
            queue.extend(more);
        }

        assert_eq!(driver.status(), Status::Complete);
        Ok((seen, written))
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

    /// For any script, the routine writes what the greeter should, and the
    /// requests the host sees carry ids minted in order from 1.
    #[test]
    fn greeter_through_the_wire_on_any_script() {
        bolero::check!()
            .with_type::<Vec<String>>()
            .for_each(|names| {
                let script: Vec<&str> = names.iter().map(String::as_str).collect();
                let Ok((seen, written)) = transcript(Driver::new(greeter), &script);
                assert_eq!(written, expected(&script));

                let ids: Vec<u64> = seen
                    .iter()
                    .filter_map(|v| match v {
                        View::Count { id }
                        | View::Lookup { id, .. }
                        | View::ReadLine { id }
                        | View::Sleep { id, .. }
                        | View::Now { id }
                        | View::Random { id, .. }
                        | View::Var { id, .. }
                        | View::ReadFile { id, .. }
                        | View::WriteFile { id, .. } => Some(*id),
                        View::WriteLine { .. }
                        | View::Spawned { .. }
                        | View::SpawnedPinned { .. } => None,
                    })
                    .collect();
                assert_eq!(ids, (1..=ids.len() as u64).collect::<Vec<_>>());
            });
    }

    /// Through the host crate, as a foreign host would reply: bytes that are
    /// not an encoded `Result<String, ReadLineError>` are `MALFORMED`, the
    /// read stays open, and a good reply is accepted after.
    #[cfg(feature = "table")]
    #[test]
    fn a_malformed_read_reply_is_refused_and_can_be_retried() -> TestResult {
        use sans_effort_core::boundary::codec::Encode;
        use sans_effort_host::{error::Error, machine::Machine};

        let mut machine = Machine::<Full>::from_routine(greeter);
        let read = machine.resume()?.into_iter().find_map(|view| {
            if let View::ReadLine { id } = view {
                Some(id)
            } else {
                None
            }
        });
        let read = read.ok_or("the greeter prompts, then reads")?;

        assert!(matches!(
            machine.reply_bytes(read, vec![7]),
            Err(Error::Malformed(_))
        ));

        let good = Ok::<String, ReadLineError>("bob".into()).to_bytes();
        let next = machine.reply_bytes(read, good)?;
        assert!(
            next.into_iter()
                .any(|view| matches!(view, View::Lookup { ref name, .. } if name == "bob")),
            "the greeter looks bob up"
        );
        Ok(())
    }

    /// A `Quiet` host runs the ticker, and only ever sees tags 4 and 5.
    #[test]
    fn ticker_under_quiet() {
        let mut driver = Driver::new(ticker);
        let mut written = Vec::new();
        let mut queue: VecDeque<Quiet> = driver.resume().into();

        while let Some(effect) = queue.pop_front() {
            match effect {
                Quiet::WriteLine(WriteLineEffect(text)) => written.push(text),
                Quiet::Sleep(Asked { request, reply }) => {
                    assert_eq!(request, SleepEffect(PAUSE));
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

    /// Machines that spawn and message each other, run by the core's test
    /// runner: each request answered, each child begun, each woken machine
    /// resumed. Under `Fifo` that happens in the order effects appear; under
    /// `Choices` fed by bolero, replies are delayed, machines interleave, and
    /// machines are resumed spuriously — and the transcript must not change,
    /// nor any machine stall. One writer per routine, so the transcript is
    /// schedule-independent exactly when the routine is.
    #[cfg(feature = "table")]
    mod scheduled {
        use super::*;
        use routines::{
            deadline::Deadline, fanout::Fanout, faults::deadlock::Deadlock, front_desk::FrontDesk,
            ping_pong::PingPong, ring::Ring,
        };
        use sans_effort_core::testing::{
            self, Completed, Stalled,
            schedule::{Choices, Fifo, Schedule},
        };

        /// What the routine and its children wrote, in order, and how the
        /// run ended — unless some machine stalled. Sleeps take their length
        /// on the runner's virtual clock.
        #[expect(
            clippy::panic,
            reason = "the runner's handler cannot return an error; a journal effect from these routines is a bug"
        )]
        fn run<S: Schedule>(
            root: Driver<Full>,
            script: &[&str],
            schedule: S,
        ) -> Result<(Vec<String>, Completed), Stalled> {
            let mut lines = script.iter().copied();
            let mut written = Vec::new();
            let completed = testing::drive_with(root, schedule, |effect, host| match effect {
                Full::WriteLine(WriteLineEffect(text)) => written.push(text),
                Full::Spawn(SpawnEffect(child)) => host.spawn(|outbox| child.start(outbox)),
                Full::SpawnPinned(SpawnPinnedEffect(child)) => {
                    host.spawn_pinned(|outbox| child.start(outbox));
                }
                Full::Sleep(Asked {
                    request: SleepEffect(duration),
                    reply,
                }) => host.reply_after(duration, reply, ()),
                Full::ReadLine(Asked { reply, .. }) => {
                    let line = lines.next().map(String::from);
                    host.reply(reply, line.ok_or(ReadLineError::Closed));
                }
                Full::Lookup(Asked {
                    request: LookupEffect(name),
                    reply,
                }) => host.reply(reply, String::from(greeting(&name))),
                Full::Count(Asked { reply, .. }) => host.reply(reply, 0),
                Full::Now(_)
                | Full::Random(_)
                | Full::Var(_)
                | Full::ReadFile(_)
                | Full::WriteFile(_) => panic!("no routine run here keeps a journal"),
            })?;
            Ok((written, completed))
        }

        /// What was written, if every machine completed and none was killed.
        fn written<S: Schedule>(
            root: Driver<Full>,
            script: &[&str],
            schedule: S,
        ) -> Result<Vec<String>, Stalled> {
            run(root, script, schedule).map(|(written, completed)| {
                assert_eq!(completed.killed(), [], "this schedule kills nothing");
                written
            })
        }

        fn greeting(name: &str) -> &'static str {
            match name {
                "alice" => "Hello",
                "bob" => "Hi",
                _ => "Greetings",
            }
        }

        fn fanout() -> Driver<Full> {
            Driver::new(|outbox| Fanout::new(Ctx::<Full>::new(outbox)).run())
        }

        /// What fan-out writes for `script`, the count always answered 0.
        fn fanned_out(script: &[&str]) -> Vec<String> {
            let mut out = vec![String::from("Who are you?")];
            match script {
                [] => out.push(String::from("Bye.")),
                [name, rest @ ..] => {
                    out.push(format!("{}, {name}! (#0)", greeting(name)));
                    out.push(rest.first().map_or_else(
                        || String::from("Bye."),
                        |farewell| format!("Bye, {farewell}."),
                    ));
                }
            }
            out
        }

        fn ping_pong() -> Driver<Full> {
            Driver::new(|outbox| PingPong::new(Ctx::<Full>::new(outbox), 3).run())
        }

        fn front_desk() -> Driver<Full> {
            Driver::new(|outbox| FrontDesk::new(Ctx::<Full>::new(outbox)).run())
        }

        fn deadlock() -> Driver<Full> {
            Driver::new(|outbox| Deadlock::new(Ctx::<Full>::new(outbox)).run())
        }

        fn deadline() -> Driver<Full> {
            Driver::new(|outbox| Deadline::new(Ctx::<Full>::new(outbox)).run())
        }

        fn ring() -> Driver<Full> {
            Driver::new(|outbox| Ring::new(Ctx::<Full>::new(outbox), 4, 3).run())
        }

        const PING_PONG: [&str; 3] = ["ping 1, pong 1", "ping 2, pong 2", "ping 3, pong 3"];
        const NAMES: [&str; 3] = ["alice", "bob", "zed"];
        const FRONT_DESK: [&str; 4] = ["Hello, alice!", "Hi, bob!", "Greetings, zed!", "Closed."];
        const RING: [&str; 1] = ["ring of 4, 3 laps: 12 hops"];
        const DEADLINE: [&str; 3] = [
            "quick worker: answered 1 in time",
            "slow worker: no answer within 50 ms",
            "slow worker: answered 2 late",
        ];

        #[test]
        fn ping_pong_across_two_machines() {
            assert_eq!(
                written(ping_pong(), &[], Fifo),
                Ok(PING_PONG.map(String::from).to_vec())
            );
        }

        #[test]
        fn front_desk_greets_in_arrival_order() {
            assert_eq!(
                written(front_desk(), &NAMES, Fifo),
                Ok(FRONT_DESK.map(String::from).to_vec())
            );
        }

        #[test]
        fn a_ring_counts_every_hop() {
            assert_eq!(
                written(ring(), &[], Fifo),
                Ok(RING.map(String::from).to_vec())
            );
        }

        /// Sleeps take virtual time, and a timer fires only when nothing else
        /// can happen: the quick worker always beats its deadline, the slow
        /// one never can.
        #[test]
        fn a_deadline_is_met_by_the_quick_and_missed_by_the_slow() {
            assert_eq!(
                written(deadline(), &[], Fifo),
                Ok(DEADLINE.map(String::from).to_vec())
            );
        }

        /// Each machine waits for the other: under every schedule the runner
        /// reports both as stalled rather than hanging. Kill either, and the
        /// survivor hears its channel close and finishes.
        #[test]
        fn a_deadlock_is_reported_under_every_schedule_and_a_kill_breaks_it() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let stalled = run(deadlock(), &[], Choices::new(bytes.iter().copied()));
                assert!(
                    matches!(stalled, Err(ref stalled) if stalled.machines() == [0, 1]),
                    "neither can go first: {stalled:?}"
                );

                let crashing = Choices::new(bytes.iter().copied()).crashing();
                match run(deadlock(), &[], crashing) {
                    Ok((_, completed)) => assert_ne!(completed.killed(), [], "only a kill ends it"),
                    Err(stalled) => assert_eq!(stalled.machines(), [0, 1]),
                }
            });
        }

        /// Fan-out puts two asks in one batch, twice. Whatever order the
        /// schedule delivers their replies in, for any script, the lines are
        /// the same.
        #[test]
        fn fanout_is_order_independent() {
            bolero::check!()
                .with_type::<(Vec<String>, Vec<u8>)>()
                .for_each(|(names, bytes)| {
                    let script: Vec<&str> = names.iter().map(String::as_str).collect();
                    assert_eq!(
                        written(fanout(), &script, Choices::new(bytes.iter().copied())),
                        Ok(fanned_out(&script))
                    );
                });
        }

        /// Schedule independence and stutter insensitivity: whatever order
        /// the bytes pick, and whatever spurious resumes they insert, the
        /// same lines come out and every machine completes.
        #[test]
        fn every_schedule_writes_the_same_and_completes() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let choices = || Choices::new(bytes.iter().copied());
                assert_eq!(
                    written(ping_pong(), &[], choices()),
                    Ok(PING_PONG.map(String::from).to_vec())
                );
                assert_eq!(
                    written(front_desk(), &NAMES, choices()),
                    Ok(FRONT_DESK.map(String::from).to_vec())
                );
                assert_eq!(
                    written(ring(), &[], choices()),
                    Ok(RING.map(String::from).to_vec())
                );
                assert_eq!(
                    written(deadline(), &[], choices()),
                    Ok(DEADLINE.map(String::from).to_vec())
                );
            });
        }

        /// Crash faults: whichever machines the schedule kills, and whenever,
        /// every survivor still completes — a peer's death closes the
        /// channels it held, and these routines take a closed channel as the
        /// end. With no kill, the run is the usual one.
        #[test]
        fn survivors_complete_whatever_is_killed() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let Ok(()) = survivors_complete(bytes);
            });
        }

        fn survivors_complete(bytes: &[u8]) -> TestResult {
            let crashing = || Choices::new(bytes.iter().copied()).crashing();
            let runs = [
                (run(ping_pong(), &[], crashing()), PING_PONG.as_slice()),
                (run(front_desk(), &NAMES, crashing()), FRONT_DESK.as_slice()),
                (run(ring(), &[], crashing()), RING.as_slice()),
                (run(deadline(), &[], crashing()), DEADLINE.as_slice()),
            ];
            for (ran, expected) in runs {
                let (written, completed) = ran?;
                if completed.killed().is_empty() {
                    assert_eq!(written, expected);
                }
            }
            Ok(())
        }
    }

    mod recorded {
        use super::*;
        use routines::{front_desk::FrontDesk, ping_pong::PingPong};
        use sans_effort_core::boundary::codec::{Reader, Writer};
        use sans_effort_host::{
            contract::{FRAME_ASK, FRAME_TELL, FRAME_WOKE},
            record::{record, replay},
            table,
        };
        use std::collections::HashSet;

        /// Perform each effect and reply; resume each child to begin it and
        /// each machine a `woke` frame names; free each as it completes.
        fn host(root: u64, script: &[&str]) -> TestResult {
            let mut lines = script.iter().copied();
            let mut done = HashSet::new();
            let mut queue = VecDeque::from([(root, table::resume(root)?)]);

            while let Some((handle, (bytes, status))) = queue.pop_front() {
                if status == Status::Complete {
                    table::free(handle)?;
                    done.insert(handle);
                }
                let mut frames = Reader::new(&bytes);
                while let (Ok(kind), Ok(payload)) = (frames.u8(), frames.bytes()) {
                    let mut r = Reader::new(payload);
                    if kind == FRAME_WOKE {
                        let woken = r.u64()?;
                        if !done.contains(&woken) {
                            queue.push_back((woken, table::resume(woken)?));
                        }
                    } else if kind == FRAME_TELL && matches!(r.u8(), Ok(6 | 7)) {
                        let child = r.u64()?;
                        queue.push_back((child, table::resume(child)?));
                    } else if kind == FRAME_ASK {
                        let record = match r.u8()? {
                            2 => {
                                let name = r.str()?;
                                let greeting = if name == "alice" { "Hello" } else { "Hi" };
                                reply(r.u64()?, 1, &String::from(greeting))
                            }
                            3 => {
                                let line =
                                    lines.next().map(String::from).ok_or(ReadLineError::Closed);
                                reply(r.u64()?, 4, &line.to_bytes())
                            }
                            tag => return Err(format!("no answer for tag {tag}"))?,
                        };
                        queue.push_back((handle, table::reply(handle, &record)?));
                    }
                }
            }
            Ok(())
        }

        /// A reply record: `kind · id · payload`, the payload already encoded.
        fn reply<P: Encode + ?Sized>(id: u64, kind: u8, payload: &P) -> Vec<u8> {
            let mut w = Writer::new();
            w.u8(kind);
            w.u64(id);
            payload.encode(&mut w);
            w.finish()
        }

        fn ping_pong() -> u64 {
            table::new(|outbox| PingPong::new(Ctx::<Full>::new(outbox), 3).run())
        }

        fn front_desk() -> u64 {
            table::new(|outbox| FrontDesk::new(Ctx::<Full>::new(outbox)).run())
        }

        #[test]
        fn a_ping_pong_run_replays_byte_for_byte() -> TestResult {
            let root = ping_pong();
            let recorder = record(root);
            host(root, &[])?;
            let log = recorder.finish();
            assert_eq!(log.handles().len(), 2, "the parent and its child");
            replay(&log, ping_pong)?;
            Ok(())
        }

        #[test]
        fn a_front_desk_run_replays_byte_for_byte() -> TestResult {
            let root = front_desk();
            let recorder = record(root);
            host(root, &["alice", "bob", "carol"])?;
            let log = recorder.finish();
            assert_eq!(
                log.handles().len(),
                4,
                "the desk and a pinned clerk per name"
            );
            replay(&log, front_desk)?;
            Ok(())
        }
    }
}
