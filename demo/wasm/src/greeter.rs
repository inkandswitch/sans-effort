//! One greeter conversation, owned by JS.

use crate::{effect::Effect, safe_integer::SafeInteger, status::Status};
use effect_routine::run::Run;
use effect_routine_host::machine::Machine;
use greeter_wire::{Ctx, Full, View};
use routines::{fanout::Fanout, greeter::Greeter as Routine};
use wasm_bindgen::prelude::*;

/// `FinalizationRegistry` do it.
#[wasm_bindgen]
pub struct Greeter {
    machine: Machine<Full>,
}

#[wasm_bindgen]
impl Greeter {
    /// Spawn a greeter. Nothing runs until `start`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "constructed from JS as `new Greeter()`; there is no Rust caller to default it for"
    )]
    pub fn new() -> Self {
        Self {
            machine: Machine::from_routine(|outbox| Routine::new(Ctx::new(outbox)).run()),
        }
    }

    /// Spawn the fan-out greeter: two requests per batch, replied to in any
    /// order.
    #[must_use]
    pub fn fanout() -> Self {
        Self {
            machine: Machine::from_routine(|outbox| Fanout::new(Ctx::new(outbox)).run()),
        }
    }

    /// Run the routine to its first wait. Valid once.
    ///
    /// # Errors
    ///
    /// If already started or already finished.
    pub fn start(&mut self) -> Result<Vec<Effect>, JsError> {
        let result = self.machine.start();
        Self::present(result)
    }

    /// Reply to request `id` with a string, and run to the next wait.
    ///
    /// # Errors
    ///
    /// If nothing awaits `id`, it awaits another kind, or the routine has
    /// finished.
    #[wasm_bindgen(js_name = replyStr)]
    pub fn reply_str(&mut self, id: f64, value: String) -> Result<Vec<Effect>, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        Self::present(result)
    }

    /// Reply to request `id` with an integer given as a JS `number`. The
    /// idiomatic path; use [`reply_u64`](Self::reply_u64) above 2^53.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str), or if `value` is not a non-negative
    /// safe integer.
    #[wasm_bindgen(js_name = replyNumber)]
    pub fn reply_number(&mut self, id: f64, value: f64) -> Result<Vec<Effect>, JsError> {
        let result = self
            .machine
            .reply(id_of(id)?, u64::from(SafeInteger::try_from(value)?));
        Self::present(result)
    }

    /// Reply to request `id` with a `u64` given as a JS `BigInt`, for values
    /// a `number` cannot hold exactly.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyU64)]
    pub fn reply_u64(&mut self, id: f64, value: u64) -> Result<Vec<Effect>, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        Self::present(result)
    }

    /// Reply to request `id` with nothing but a wake-up.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyUnit)]
    pub fn reply_unit(&mut self, id: f64) -> Result<Vec<Effect>, JsError> {
        let result = self.machine.reply(id_of(id)?, ());
        Self::present(result)
    }

    /// Reply to request `id` with bytes.
    ///
    /// # Errors
    ///
    /// As [`reply_str`](Self::reply_str).
    #[wasm_bindgen(js_name = replyBytes)]
    pub fn reply_bytes(&mut self, id: f64, value: Vec<u8>) -> Result<Vec<Effect>, JsError> {
        let result = self.machine.reply(id_of(id)?, value);
        Self::present(result)
    }

    /// Whether the routine has run to completion.
    #[wasm_bindgen(js_name = isFinished)]
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.machine.is_finished()
    }

    /// Where the routine stopped after the last call. Read this after a
    /// `start` or `reply` whose returned effects were all fire-and-forget: if
    /// it is `Complete` there is nothing more to do.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn status(&self) -> Status {
        self.machine.status().into()
    }

    fn present(
        result: Result<Vec<View>, effect_routine_host::error::Error>,
    ) -> Result<Vec<Effect>, JsError> {
        let views = result.map_err(|e| JsError::new(&e.to_string()))?;
        Ok(views
            .into_iter()
            .map(Effect::try_from)
            .collect::<Result<_, _>>()?)
    }
}

impl core::fmt::Debug for Greeter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Greeter").finish_non_exhaustive()
    }
}

/// A request id from JS: a `number` that must be a safe integer.
fn id_of(id: f64) -> Result<u64, JsError> {
    Ok(SafeInteger::try_from(id)?.into())
}
