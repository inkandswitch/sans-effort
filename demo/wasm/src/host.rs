//! The JS object a caller supplies: the five capabilities as methods.
//!
//! Declared, not defined — `wasm-bindgen` generates the glue to call whatever
//! the caller passed. Each method may return its value directly or a
//! `Promise` of it; [`JsCtx`](crate::ctx::JsCtx) accepts either, so a host
//! written with plain functions and one written with `async` functions both
//! work.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// An object with `readLine`, `lookup`, `sleep`, `count`, and `writeLine`.
    /// `Host` to JS; the `Js` prefix is the Rust side's, as with the classes.
    #[wasm_bindgen(js_name = Host)]
    pub type JsHost;

    /// The next line of input. `string`, or a `Promise` of one.
    #[wasm_bindgen(method, js_name = readLine)]
    pub fn read_line(this: &JsHost) -> JsValue;

    /// The greeting for `name`. `string`, or a `Promise` of one.
    #[wasm_bindgen(method)]
    pub fn lookup(this: &JsHost, name: &str) -> JsValue;

    /// Wait `millis`. Anything, or a `Promise` to await.
    #[wasm_bindgen(method)]
    pub fn sleep(this: &JsHost, millis: f64) -> JsValue;

    /// One more greeting; how many so far. `number`, or a `Promise` of one.
    #[wasm_bindgen(method)]
    pub fn count(this: &JsHost) -> JsValue;

    /// Show `line`.
    #[wasm_bindgen(method, js_name = writeLine)]
    pub fn write_line(this: &JsHost, line: &str);
}
