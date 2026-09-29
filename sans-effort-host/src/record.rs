//! Recording a run through the handle table, and replaying it.
//!
//! A recording is a tap: it changes nothing about the calls it sees. Start
//! one on a root machine with [`record`] before the root's first `resume`;
//! every machine the root creates, and every machine those create, joins the
//! same session. Each [`resume`](crate::table::resume),
//! [`reply`](crate::table::reply), and [`free`](crate::table::free) on a
//! session machine is logged in call order, with what went in and what came
//! out. Machines outside the session — another test's, say — never appear.
//!
//! A routine is deterministic given its inputs, and request ids are minted
//! deterministically, so the calls alone are enough to run it again:
//! [`replay`] builds the root anew, makes the same calls in the same order,
//! and compares every outcome with the recorded one. The table issues the
//! recorded handles again while it replays, so outputs that name machines —
//! `woke` frames, a vocabulary's spawn effects — compare byte for byte.
//!
//! ```text
//!   record(root) ──▶ host drives root and its children ──▶ finish() → Log
//!                                                                   │
//!   replay(&log, || new_root()) ──▶ same calls, in order, compared ◀┘
//! ```
//!
//! Replay is sequential. A run from a host that makes one call at a time
//! replays exactly. A run from a parallel host, whose calls on machines that
//! talk to each other overlapped in time, may not — and then replay reports
//! a [`Divergence`] rather than passing quietly. Wakes outside any call
//! ([`wakes`](crate::table::wakes)) are not recorded: they change no machine.

use crate::{
    contract::{code_of, status_code},
    error::Error,
    table,
};
use alloc::vec::Vec;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};
use sans_effort_core::driver::status::Status;

/// Begin recording `root` and every machine it creates. Call it before the
/// root's first `resume`, and [`finish`](Recorder::finish) it to get the
/// log.
#[must_use]
pub fn record(root: u64) -> Recorder {
    Recorder {
        session: Some(table::start_recording(root)),
    }
}

/// Make the calls `log` recorded again, against a root `root` builds, and
/// check each outcome against the recorded one.
///
/// `root` must build the same routine the recording's root was — the same
/// constructor a binding exports, say. The recorded machines must have been
/// freed (a finished run frees them), so their handles can be issued again.
/// Every machine the replay creates is freed before it returns.
///
/// # Errors
///
/// [`Divergence`] at the first call whose outcome differs, or that names a
/// machine the replay never created.
pub fn replay<R: FnOnce() -> u64>(log: &Log, root: R) -> Result<(), Divergence> {
    let reissuing = table::reissue(&log.handles);
    let recorder = record(root());
    let session = recorder.session.unwrap_or_default();

    let result = log.events.iter().enumerate().try_for_each(|(at, event)| {
        let machine = event.machine();
        let handle =
            table::recorded_handle(session, machine).ok_or(Divergence::Missing { at, machine })?;
        let actual = match event {
            Event::Resume { .. } => outcome(&table::resume(handle)),
            Event::Reply { record, .. } => outcome(&table::reply(handle, record)),
            Event::Free { .. } => Outcome {
                code: code_of(table::free(handle)),
                bytes: Vec::new(),
            },
        };
        if actual == *event.outcome() {
            Ok(())
        } else {
            Err(Divergence::Outcome {
                at,
                expected: event.outcome().clone(),
                actual,
            })
        }
    });

    drop(reissuing);
    for handle in recorder.finish().handles {
        // Already freed, most of them: the recording's own frees.
        let _gone = table::free(handle);
    }
    result
}

/// A recording in progress. Dropping it without [`finish`](Self::finish)
/// stops recording and discards the log.
#[derive(Debug)]
pub struct Recorder {
    /// `None` once finished.
    session: Option<u32>,
}

impl Recorder {
    /// Stop recording: the calls made so far.
    #[must_use]
    pub fn finish(mut self) -> Log {
        self.session
            .take()
            .map(table::finish_recording)
            .unwrap_or_default()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            drop(table::finish_recording(session));
        }
    }
}

/// A recorded run: the machines in the session, in the order they were
/// created (the root first), and every call on them, in order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Log {
    handles: Vec<u64>,
    events: Vec<Event>,
}

impl Log {
    /// Each machine's handle when it was recorded, by its number in the
    /// session.
    #[must_use]
    pub fn handles(&self) -> &[u64] {
        &self.handles
    }

    /// The calls, in order.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Add a machine to the session: its number.
    pub(crate) fn adopt(&mut self, handle: u64) -> u32 {
        self.handles.push(handle);
        u32::try_from(self.handles.len() - 1).unwrap_or(u32::MAX)
    }

    pub(crate) fn push(&mut self, event: Event) {
        self.events.push(event);
    }

    /// The handle of the session's `machine`th machine.
    pub(crate) fn handle(&self, machine: u32) -> Option<u64> {
        usize::try_from(machine)
            .ok()
            .and_then(|at| self.handles.get(at))
            .copied()
    }
}

/// The handle count and each handle as `u64`, then the event count and each
/// event: a tag (`1` resume, `2` reply, `3` free), the machine as `u32`, the
/// reply record as `bytes` for a reply, then the outcome.
impl Encode for Log {
    fn encode(&self, w: &mut Writer) {
        w.u32(u32::try_from(self.handles.len()).unwrap_or(u32::MAX));
        for handle in &self.handles {
            w.u64(*handle);
        }
        w.u32(u32::try_from(self.events.len()).unwrap_or(u32::MAX));
        for event in &self.events {
            event.encode(w);
        }
    }
}

impl Decode for Log {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let handles = (0..r.u32()?).map(|_| r.u64()).collect::<Result<_, _>>()?;
        let events = (0..r.u32()?)
            .map(|_| Event::decode(r))
            .collect::<Result<_, _>>()?;
        Ok(Log { handles, events })
    }
}

/// One call on a recorded machine, named by its number in the session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// [`resume`](crate::table::resume).
    Resume {
        /// Which machine.
        machine: u32,
        /// What it returned.
        outcome: Outcome,
    },

    /// [`reply`](crate::table::reply).
    Reply {
        /// Which machine.
        machine: u32,
        /// The reply record the host sent.
        record: Vec<u8>,
        /// What it returned.
        outcome: Outcome,
    },

    /// [`free`](crate::table::free).
    Free {
        /// Which machine.
        machine: u32,
        /// What it returned: the code only.
        outcome: Outcome,
    },
}

impl Event {
    /// Which machine the call was on.
    #[must_use]
    pub const fn machine(&self) -> u32 {
        match self {
            Event::Resume { machine, .. }
            | Event::Reply { machine, .. }
            | Event::Free { machine, .. } => *machine,
        }
    }

    /// What the call returned.
    #[must_use]
    pub const fn outcome(&self) -> &Outcome {
        match self {
            Event::Resume { outcome, .. }
            | Event::Reply { outcome, .. }
            | Event::Free { outcome, .. } => outcome,
        }
    }
}

impl Encode for Event {
    fn encode(&self, w: &mut Writer) {
        match self {
            Event::Resume { machine, outcome } => {
                w.u8(1);
                w.u32(*machine);
                outcome.encode(w);
            }
            Event::Reply {
                machine,
                record,
                outcome,
            } => {
                w.u8(2);
                w.u32(*machine);
                w.bytes(record);
                outcome.encode(w);
            }
            Event::Free { machine, outcome } => {
                w.u8(3);
                w.u32(*machine);
                outcome.encode(w);
            }
        }
    }
}

impl Decode for Event {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            1 => Ok(Event::Resume {
                machine: r.u32()?,
                outcome: Outcome::decode(r)?,
            }),
            2 => Ok(Event::Reply {
                machine: r.u32()?,
                record: r.bytes()?.to_vec(),
                outcome: Outcome::decode(r)?,
            }),
            3 => Ok(Event::Free {
                machine: r.u32()?,
                outcome: Outcome::decode(r)?,
            }),
            tag => Err(DecodeError::UnknownTag { tag }),
        }
    }
}

/// What a call returned: its code — a status (`>= 0`) or an error (`< 0`),
/// as `ABI.md` numbers them — and the effect frames it wrote.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    /// The status or error code.
    pub code: i32,
    /// The frames written, empty on an error.
    pub bytes: Vec<u8>,
}

/// The code as `u32` bits, then the frames as `bytes`.
impl Encode for Outcome {
    fn encode(&self, w: &mut Writer) {
        w.u32(u32::from_le_bytes(self.code.to_le_bytes()));
        w.bytes(&self.bytes);
    }
}

impl Decode for Outcome {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let code = i32::from_le_bytes(r.u32()?.to_le_bytes());
        let bytes = r.bytes()?.to_vec();
        Ok(Outcome { code, bytes })
    }
}

/// Where a replay stopped agreeing with its recording.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Divergence {
    /// A call's outcome differed.
    #[error("call {at}: recorded {expected:?}, replayed {actual:?}")]
    Outcome {
        /// Which call, counting from 0.
        at: usize,
        /// What the recording says it returned.
        expected: Outcome,
        /// What it returned this time.
        actual: Outcome,
    },

    /// A call named a machine the replay never created.
    #[error("call {at}: machine {machine} was never created")]
    Missing {
        /// Which call, counting from 0.
        at: usize,
        /// The machine's number in the session.
        machine: u32,
    },
}

/// A call's result, as a recording keeps it.
pub(crate) fn outcome(result: &Result<(Vec<u8>, Status), Error>) -> Outcome {
    match result {
        Ok((bytes, status)) => Outcome {
            code: status_code(*status),
            bytes: bytes.clone(),
        },
        Err(e) => Outcome {
            code: e.code(),
            bytes: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::fixtures::{Both, Echo};
    use alloc::string::String;
    use sans_effort_core::step::Step;

    fn echo() -> u64 {
        table::new(|outbox| Echo(outbox).run())
    }

    /// A `str` reply record: kind `1`, the id, the string.
    fn str_reply(id: u64, s: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.u64(id);
        w.str(s);
        w.finish()
    }

    /// Record an echo answered with `answer`: resume, reply, free.
    fn recorded_echo(answer: &str) -> Log {
        let root = echo();
        let recorder = record(root);
        table::resume(root).expect("begins, asking id 1");
        table::reply(root, &str_reply(1, answer)).expect("answered");
        table::free(root).expect("freed");
        recorder.finish()
    }

    #[test]
    fn a_run_replays_exactly() {
        let log = recorded_echo("hi");
        assert_eq!(log.events().len(), 3, "resume, reply, free");
        assert_eq!(log.handles().len(), 1, "just the root");
        replay(&log, echo).expect("the same calls, the same outcomes");
    }

    #[test]
    fn a_changed_reply_diverges_where_it_was_made() {
        let mut log = recorded_echo("hi");
        let Some(Event::Reply { record, .. }) = log.events.get_mut(1) else {
            unreachable!("the second call is the reply");
        };
        *record = str_reply(1, "ho");
        assert!(matches!(
            replay(&log, echo),
            Err(Divergence::Outcome { at: 1, .. })
        ));
    }

    #[test]
    fn another_routine_diverges_at_its_first_call() {
        let log = recorded_echo("hi");
        let both = || table::new(|outbox| Both(outbox).run());
        assert!(matches!(
            replay(&log, both),
            Err(Divergence::Outcome { at: 0, .. })
        ));
    }

    #[test]
    fn machines_outside_the_session_are_not_recorded() {
        let root = echo();
        let other = echo();
        let recorder = record(root);
        table::resume(other).expect("another machine");
        table::resume(root).expect("the root");
        let log = recorder.finish();
        table::free(other).expect("freed");
        table::free(root).expect("freed");
        assert_eq!(log.events().len(), 1);
        assert_eq!(log.handles(), [root]);
    }

    #[test]
    fn logs_round_trip_as_bytes() {
        let log = recorded_echo(&String::from("round trip"));
        assert_eq!(Log::from_bytes(&log.to_bytes()), Ok(log));
    }
}
