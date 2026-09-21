//! What one call produced.

use crate::{effect::Effect, status::Status};
use wasm_bindgen::prelude::*;

/// where the routine stopped.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Batch {
    status: Status,
    effects: Vec<Effect>,
}

impl Batch {
    pub(crate) const fn new(status: Status, effects: Vec<Effect>) -> Self {
        Self { status, effects }
    }
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
