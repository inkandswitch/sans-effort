//! The greeter as a JS class.

use crate::{ctx::JsCtx, host::JsHost};
use effect_routine::run::Run;
use routines::greeter::Greeter;
use wasm_bindgen::prelude::*;

/// A greeter over a JS host. Construct, then `await` [`run`](Self::run).
#[wasm_bindgen(js_name = Greeter)]
#[derive(Debug)]
pub struct JsGreeter {
    routine: Greeter<JsCtx>,
}

#[wasm_bindgen(js_class = Greeter)]
impl JsGreeter {
    /// A greeter that does everything through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        Self {
            routine: Greeter::new(JsCtx::new(host)),
        }
    }

    /// Run the conversation until the host says `quit`. Consumes the greeter;
    /// the returned `Promise` resolves when it is over.
    pub async fn run(self) {
        self.routine.run().await;
    }
}
