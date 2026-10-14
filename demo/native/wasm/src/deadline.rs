//! The deadline as a JS class: receives raced against sleeps.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::deadline::Deadline;
use sans_effort_core::step::Step;
use wasm_bindgen::prelude::*;

/// Two workers, each raced against a sleep. The quick one wins, so its
/// 30-second sleep is abandoned — and the host's timer aborted; the slow one
/// loses, then answers late. Construct, then `await` [`run`](Self::run).
#[wasm_bindgen(js_name = Deadline)]
#[derive(Debug)]
pub struct JsDeadline {
    routine: Deadline<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Deadline)]
impl JsDeadline {
    /// A deadline writing through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Deadline::new(ctx),
            drained,
        }
    }

    /// Ask both workers. Consumes it; the returned `Promise` resolves once
    /// both workers have finished too.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
