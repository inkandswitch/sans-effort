//! One greeter conversation, owned by JS.

use crate::{batch::Batch, effect::Effect, safe_integer::SafeInteger};
use effect_routine_host::machine::Machine;
use greeter_wire::{Full, View};
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
    pub fn new() -> Self {
        Self {
            machine: Machine::drive(greeter_wire::greeter),
        }
    }

    /// Spawn the fan-out greeter: two requests per batch, replied to in any
    /// order.
    #[must_use]
    pub fn fanout() -> Self {
        Self {
            machine: Machine::drive(greeter_wire::fanout),
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
        result: Result<Vec<View>, effect_routine_host::error::Error>,
    ) -> Result<Batch, JsError> {
        let views = result.map_err(|e| JsError::new(&e.to_string()))?;

        let effects = views
            .into_iter()
            .map(Effect::try_from)
            .collect::<Result<_, _>>()?;
        Ok(Batch::new(self.machine.status().into(), effects))
    }
}

impl Default for Greeter {
    fn default() -> Self {
        Self::new()
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
