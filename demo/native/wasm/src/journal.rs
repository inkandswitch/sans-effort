//! The journal as a JS class: entries stamped with the host's time and
//! randomness, in a file the host keeps.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::journal::Journal;
use sans_effort_core::step::Step;
use wasm_bindgen::prelude::*;

/// A journal appending three entries through a JS host. Construct, then
/// `await` [`run`](Self::run).
#[wasm_bindgen(js_name = Journal)]
#[derive(Debug)]
pub struct JsJournal {
    routine: Journal<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Journal)]
impl JsJournal {
    /// A journal keeping its file through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Journal::new(ctx, 3),
            drained,
        }
    }

    /// Append the entries. Consumes it; the returned `Promise` resolves once
    /// the host has seen every call.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
