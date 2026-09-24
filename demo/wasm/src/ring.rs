//! The ring as a JS class: routines passing a counter around a ring.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::ring::Ring;
use sans_effort::run::Run;
use wasm_bindgen::prelude::*;

/// A ring of 16 routines passing a counter 250 times around, over a JS host.
/// Construct, then `await` [`run`](Self::run).
#[wasm_bindgen(js_name = Ring)]
#[derive(Debug)]
pub struct JsRing {
    routine: Ring<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Ring)]
impl JsRing {
    /// A ring writing through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Ring::new(ctx, 16, 250),
            drained,
        }
    }

    /// Pass the counter around. Consumes it; the returned `Promise` resolves
    /// once every node has finished too.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
