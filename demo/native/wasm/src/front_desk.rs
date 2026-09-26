//! The front desk as a JS class: a receptionist that spawns a clerk per
//! visitor.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::front_desk::FrontDesk;
use sans_effort_core::step::Step;
use wasm_bindgen::prelude::*;

/// A front desk over a JS host. Construct, then `await` [`run`](Self::run).
#[wasm_bindgen(js_name = FrontDesk)]
#[derive(Debug)]
pub struct JsFrontDesk {
    routine: FrontDesk<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = FrontDesk)]
impl JsFrontDesk {
    /// A front desk doing everything through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: FrontDesk::new(ctx),
            drained,
        }
    }

    /// Greet everyone the host sends in. Consumes it; the returned `Promise`
    /// resolves once every clerk has finished too.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
