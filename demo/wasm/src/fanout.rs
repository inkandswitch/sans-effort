//! The fan-out greeter as a JS class.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::fanout::Fanout;
use sans_effort::run::Run;
use wasm_bindgen::prelude::*;

/// The fan-out greeter over a JS host: two waits in flight at once, as two
/// `Promise`s awaited together.
#[wasm_bindgen(js_name = Fanout)]
#[derive(Debug)]
pub struct JsFanout {
    routine: Fanout<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = Fanout)]
impl JsFanout {
    /// A fan-out greeter that does everything through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: Fanout::new(ctx),
            drained,
        }
    }

    /// Run the conversation. Consumes the greeter; the returned `Promise`
    /// resolves when it is over.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
