//! The fan-out greeter as a JS class.

use crate::{ctx::JsCtx, host::JsHost};
use effect_routine::run::Run;
use routines::fanout::Fanout;
use wasm_bindgen::prelude::*;

/// The fan-out greeter over a JS host: two waits in flight at once, as two
/// `Promise`s awaited together.
#[wasm_bindgen(js_name = Fanout)]
#[derive(Debug)]
pub struct JsFanout {
    routine: Fanout<JsCtx>,
}

#[wasm_bindgen(js_class = Fanout)]
impl JsFanout {
    /// A fan-out greeter that does everything through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        Self {
            routine: Fanout::new(JsCtx::new(host)),
        }
    }

    /// Run the conversation. Consumes the greeter; the returned `Promise`
    /// resolves when it is over.
    pub async fn run(self) {
        self.routine.run().await;
    }
}
