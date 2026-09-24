//! The greeter as a JS class.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::greeter::Greeter;
use sans_effort::run::Run;
use wasm_bindgen::prelude::*;

/// A greeter over a JS host. Construct, then `await` [`run`](Self::run).
#[wasm_bindgen(js_name = Greeter)]
#[derive(Debug)]
pub struct JsGreeter {
    routine: Greeter<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Greeter)]
impl JsGreeter {
    /// A greeter that does everything through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Greeter::new(ctx),
            drained,
        }
    }

    /// Run the conversation until the host says `quit`. Consumes the greeter;
    /// the returned `Promise` resolves when it is over.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
