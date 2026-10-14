//! Ping-pong as a JS class: a parent and the child it spawns, talking over
//! channels.

use crate::{
    ctx::{Drained, JsCtx},
    host::JsHost,
};
use routines::ping_pong::PingPong;
use sans_effort_core::step::Step;
use wasm_bindgen::prelude::*;

/// Three rounds of ping-pong over a JS host. Construct, then `await`
/// [`run`](Self::run).
#[wasm_bindgen(js_name = PingPong)]
#[derive(Debug)]
pub struct JsPingPong {
    routine: PingPong<JsCtx>,
    drained: Drained,
}

#[wasm_bindgen(js_class = PingPong)]
impl JsPingPong {
    /// Ping-pong writing through `host`.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(host: JsHost) -> Self {
        let (ctx, drained) = JsCtx::new(host);
        Self {
            routine: PingPong::new(ctx, 3),
            drained,
        }
    }

    /// Play. Consumes it; the returned `Promise` resolves once the child has
    /// finished too.
    pub async fn run(self) {
        self.routine.run().await;
        self.drained.wait().await;
    }
}
