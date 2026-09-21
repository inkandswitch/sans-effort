//! The greeter, exported to JS with `wasm-bindgen`.
//!
//! The routine is the unchanged `greeter` crate, under the reifying context
//! from `greeter_wire`. This crate is the boundary:
//! one JS class, [`Greeter`], whose `start()` runs the routine to its first
//! wait and whose `reply*` methods deliver one answer by request id and run
//! to the next, each returning the batch of effects emitted in between.
//!
//! Beside `../cdylib`, this is the _other_ kind of skin. There, the host gets
//! a `u64` handle and a byte buffer and decodes it with a table it wrote
//! itself. Here `wasm-bindgen` generates the class, the enum, and the typed
//! getters, so there is no handle table and no codec — the
//! [`Machine`](effect_routine_host::Machine) is held directly and its
//! [`View`](greeter_wire::View)s are converted to JS objects. What does not change
//! is the contract: Rust never calls JS; the host owns the loop, the clock,
//! and all IO; requests carry ids, and the host may reply in any order.
//!
//! ```text
//!   JS:  const g = new Greeter();
//!        let { status, effects } = g.start();            // [Write, ReadLine·1]
//!        ({ status, effects } = g.replyStr(1, "bob"));   // [Lookup·2]
//! ```
//!
//! # Panics
//!
//! `wasm32-unknown-unknown` is `panic = "abort"`: a panicking routine traps,
//! the host sees a `RuntimeError`, and the instance should be discarded.
//! There is no `PANICKED` status here because there is no unwinding to catch.

#![allow(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen cannot export `const fn`"
)]

use effect_routine::driver::Driver;
use effect_routine_host::{Machine, Status as MachineStatus};
use greeter_wire::{Full, View};
use safe_integer::SafeInteger;
use wasm_bindgen::prelude::*;

/// Which effect this is.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// How many greetings so far. Reply with `replyNumber` (or `replyU64`
    /// for a value above 2^53).
    Count,
    /// The greeting for `name`. Reply with `replyStr`.
    Lookup,
    /// A line of input. Reply with `replyStr`.
    ReadLine,
    /// Wait `millis`. Reply with `replyUnit`.
    Sleep,
    /// Show `text`. No reply.
    Write,
}

/// One effect out of a batch, as JS sees it. `id` is set when the effect
/// awaits a reply; `name`, `text`, and `millis` are set for the kinds that
/// carry them.
#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct Effect {
    kind: Kind,
    id: Option<SafeInteger>,
    name: Option<String>,
    text: Option<String>,
    millis: Option<SafeInteger>,
}

#[wasm_bindgen]
impl Effect {
    /// Which effect.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The request id, if this effect awaits a reply. A plain `number`: ids
    /// count up from 1 per routine and never approach 2^53.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn id(&self) -> Option<f64> {
        self.id.map(f64::from)
    }

    /// The name to look up, for `Lookup`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// The text to show, for `Write`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.text.clone()
    }

    /// How long to wait in milliseconds, for `Sleep`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn millis(&self) -> Option<f64> {
        self.millis.map(f64::from)
    }
}

impl TryFrom<View> for Effect {
    type Error = safe_integer::NotSafe;

    /// Fails only if an id or a duration exceeds 2^53 − 1, which the routine
    /// never produces; the check lives here so the getters can be total.
    fn try_from(view: View) -> Result<Self, Self::Error> {
        let blank = Self {
            kind: Kind::Write,
            id: None,
            name: None,
            text: None,
            millis: None,
        };

        Ok(match view {
            View::Count { id } => Self {
                kind: Kind::Count,
                id: Some(id.try_into()?),
                ..blank
            },
            View::Lookup { name, id } => Self {
                kind: Kind::Lookup,
                id: Some(id.try_into()?),
                name: Some(name),
                ..blank
            },
            View::ReadLine { id } => Self {
                kind: Kind::ReadLine,
                id: Some(id.try_into()?),
                ..blank
            },
            View::Sleep { millis, id } => Self {
                kind: Kind::Sleep,
                id: Some(id.try_into()?),
                millis: Some(millis.try_into()?),
                ..blank
            },
            View::Write { text } => Self {
                text: Some(text),
                ..blank
            },
        })
    }
}

/// Where the routine stopped.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Suspended; at least one effect in the batch awaits a reply.
    Awaiting,
    /// Ran to completion. Further replies throw.
    Complete,
    /// Suspended on something the host cannot answer. A bug in the routine.
    Stalled,
}

impl From<MachineStatus> for Status {
    fn from(status: MachineStatus) -> Self {
        match status {
            MachineStatus::Awaiting => Status::Awaiting,
            MachineStatus::Complete => Status::Complete,
            MachineStatus::Stalled => Status::Stalled,
        }
    }
}

/// What one call produced: the effects recorded before the next wait, and
/// where the routine stopped.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Batch {
    status: Status,
    effects: Vec<Effect>,
}

#[wasm_bindgen]
impl Batch {
    /// Where the routine stopped.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// The effects emitted before it stopped, in order.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn effects(&self) -> Vec<Effect> {
        self.effects.clone()
    }
}

/// One greeter conversation. JS owns it; call `free()` when done, or let
/// `FinalizationRegistry` do it.
#[wasm_bindgen]
pub struct Greeter {
    machine: Machine<Full>,
}

impl core::fmt::Debug for Greeter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Greeter").finish_non_exhaustive()
    }
}

#[wasm_bindgen]
impl Greeter {
    /// Spawn a greeter. Nothing runs until `start`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            machine: Machine::new(Driver::new(greeter_wire::greeter)),
        }
    }

    /// Spawn the fan-out greeter: two requests per batch, replied to in any
    /// order.
    #[must_use]
    pub fn fanout() -> Self {
        Self {
            machine: Machine::new(Driver::new(greeter_wire::fanout)),
        }
    }

    /// Run the routine to its first wait. Valid once.
    ///
    /// # Errors
    ///
    /// If already started or already finished.
    pub fn start(&mut self) -> Result<Batch, JsError> {
        let result = self.machine.start();
        self.present(result)
    }

    /// Reply to request `id` with a string, and run to the next wait.
    ///
    /// # Errors
    ///
    /// If nothing awaits `id`, it awaits another kind, or the routine has
    /// finished.
    #[wasm_bindgen(js_name = replyStr)]
    pub fn reply_str(&mut self, id: f64, value: String) -> Result<Batch, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        self.present(result)
    }

    /// Reply to request `id` with an integer given as a JS `number`. The
    /// idiomatic path; use [`reply_u64`](Self::reply_u64) above 2^53.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str), or if `value` is not a non-negative
    /// safe integer.
    #[wasm_bindgen(js_name = replyNumber)]
    pub fn reply_number(&mut self, id: f64, value: f64) -> Result<Batch, JsError> {
        let result = self
            .machine
            .reply(id_of(id)?, u64::from(SafeInteger::try_from(value)?));
        self.present(result)
    }

    /// Reply to request `id` with a `u64` given as a JS `BigInt`, for values
    /// a `number` cannot hold exactly.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyU64)]
    pub fn reply_u64(&mut self, id: f64, value: u64) -> Result<Batch, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        self.present(result)
    }

    /// Reply to request `id` with nothing but a wake-up.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyUnit)]
    pub fn reply_unit(&mut self, id: f64) -> Result<Batch, JsError> {
        let result = self.machine.reply(id_of(id)?, ());
        self.present(result)
    }

    /// Reply to request `id` with bytes.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyBytes)]
    pub fn reply_bytes(&mut self, id: f64, value: Vec<u8>) -> Result<Batch, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        self.present(result)
    }

    /// Whether the routine has run to completion.
    #[wasm_bindgen(js_name = isFinished)]
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.machine.is_finished()
    }

    fn present(
        &self,
        result: Result<Vec<View>, effect_routine_host::Error>,
    ) -> Result<Batch, JsError> {
        let views = result.map_err(|e| JsError::new(&e.to_string()))?;

        Ok(Batch {
            status: self.machine.status().into(),
            effects: views
                .into_iter()
                .map(Effect::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl Default for Greeter {
    fn default() -> Self {
        Self::new()
    }
}

mod safe_integer;

/// A request id from JS: a `number` that must be a safe integer.
fn id_of(id: f64) -> Result<u64, JsError> {
    Ok(SafeInteger::try_from(id)?.into())
}
