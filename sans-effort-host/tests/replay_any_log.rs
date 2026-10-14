//! A log may come from outside the process: a file a host saved, say. Replay
//! must be total over whatever such a file decodes to: it reports a
//! divergence or passes, and never panics, whatever handles, machines, and
//! reply records the log names.
//!
//! Its own test binary, so its own handle table: the handles a crafted log
//! names are issued again, and in a shared process they could be handles
//! another test still holds.

use sans_effort_core::{
    boundary::{
        codec::{Decode, Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::{BoxedRoutine, outbox::Outbox},
    reply::handle::ReplyHandle,
};
use sans_effort_host::{
    record::{self, Event, Log, Outcome},
    table,
};
use testresult::TestResult;

enum Effect {
    Ask(ReplyHandle<u64>),
    Spawn(Box<dyn FnOnce(Outbox<Effect>) -> BoxedRoutine + Send>),
}

enum View {
    Ask(u64),
    Spawned(u64),
}

impl HostEffect for Effect {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Effect::Ask(reply) => (View::Ask(reply.id()), Some(Pending::U64(reply))),
            Effect::Spawn(make) => (View::Spawned(table::new_boxed(make)), None),
        }
    }
}

impl Encode for View {
    fn encode(&self, w: &mut Writer) {
        let (tag, value) = match self {
            View::Ask(id) => (1, id),
            View::Spawned(handle) => (2, handle),
        };
        w.u8(tag);
        w.u64(*value);
    }
}

/// Asks, spawns a child that asks, and asks again: enough to exercise
/// replies, a second machine, and re-issued handles.
fn root() -> u64 {
    table::new(|outbox: Outbox<Effect>| async move {
        let _answer = outbox.ask(Effect::Ask).await;
        outbox.tell(Effect::Spawn(Box::new(|child: Outbox<Effect>| {
            Box::pin(async move {
                let _answer = child.ask(Effect::Ask).await;
            })
        })));
        let _answer = outbox.ask(Effect::Ask).await;
    })
}

/// A `u64` reply record: kind `2`, the id, the value.
fn u64_reply(id: u64, value: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(2);
    w.u64(id);
    w.u64(value);
    w.finish()
}

/// A log with these handles and events, through its documented encoding,
/// as a file would hold it.
fn log(handles: &[u64], events: &[Event]) -> TestResult<Log> {
    let mut w = Writer::new();
    w.u32(u32::try_from(handles.len())?);
    for handle in handles {
        w.u64(*handle);
    }
    w.u32(u32::try_from(events.len())?);
    for event in events {
        event.encode(&mut w);
    }
    Ok(Log::from_bytes(&w.finish())?)
}

/// One generated call: which kind, on which machine, the reply (a
/// well-formed one for a small id, or raw bytes), and a recorded outcome.
type Call = (u8, u8, Option<u8>, Vec<u8>, i32, Vec<u8>);

fn event(handles: usize, (kind, machine, id, raw, code, bytes): Call) -> Event {
    let machine = u32::from(machine) % u32::try_from(handles + 1).unwrap_or(u32::MAX);
    let outcome = Outcome { code, bytes };
    match kind % 3 {
        0 => Event::Resume { machine, outcome },
        1 => Event::Reply {
            machine,
            record: id.map_or(raw, |id| u64_reply(u64::from(id % 4), 7)),
            outcome,
        },
        _ => Event::Free { machine, outcome },
    }
}

fn replays_without_panicking((handles, calls): &(Vec<u64>, Vec<Call>)) -> TestResult {
    let events: Vec<Event> = calls
        .iter()
        .cloned()
        .map(|call| event(handles.len(), call))
        .collect();
    let log = log(handles, &events)?;
    drop(record::replay(&log, root));
    Ok(())
}

#[test]
fn replaying_any_log_never_panics() {
    bolero::check!()
        .with_type::<(Vec<u64>, Vec<Call>)>()
        .for_each(|input| {
            let Ok(()) = replays_without_panicking(input);
        });
}

/// The extremes, which a generator may not reach: no handle is `0`, and the
/// last handle is not one past anything.
#[test]
fn a_log_naming_the_extreme_handles_replays_without_panicking() -> TestResult {
    for handles in [
        [0, 0],
        [u64::MAX, u64::MAX],
        [u64::MAX - 1, u64::MAX],
        [u64::MAX, 0],
    ] {
        let events = [
            Event::Resume {
                machine: 0,
                outcome: Outcome::default(),
            },
            Event::Reply {
                machine: 0,
                record: u64_reply(1, 7),
                outcome: Outcome::default(),
            },
            Event::Resume {
                machine: 1,
                outcome: Outcome::default(),
            },
        ];
        drop(record::replay(&log(&handles, &events)?, root));
    }
    let fresh = root();
    assert_ne!(fresh, 0, "a handle is never 0");
    assert_eq!(table::free(fresh), Ok(()));
    Ok(())
}
