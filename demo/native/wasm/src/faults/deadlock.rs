//! The deadlock as a JS class: two routines each waiting for the other.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::faults::deadlock::Deadlock;
use sans_effort_core::step::Step;
use wasm_bindgen::prelude::*;

/// Two routines, each waiting for the other to go first. Nothing will ever
/// wake either, so [`run`](Self::run)'s `Promise` never settles: Node's
/// event loop runs dry, and at the top level of a module Node exits 13.
#[wasm_bindgen(js_name = Deadlock)]
#[derive(Debug)]
pub struct JsDeadlock {
    routine: Deadlock<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Deadlock)]
impl JsDeadlock {
    /// A deadlock writing through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Deadlock::new(ctx),
            drained,
        }
    }

    /// Wait, forever. Consumes it.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
