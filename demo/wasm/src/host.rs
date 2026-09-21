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
    /// An object with `readLine`, `lookup`, `sleep`, `count`, and `write`.
    pub type Host;

    /// The next line of input. `string`, or a `Promise` of one.
    #[wasm_bindgen(method, js_name = readLine)]
    pub fn read_line(this: &Host) -> JsValue;

    /// The greeting for `name`. `string`, or a `Promise` of one.
    #[wasm_bindgen(method)]
    pub fn lookup(this: &Host, name: &str) -> JsValue;

    /// Wait `millis`. Anything, or a `Promise` to await.
    #[wasm_bindgen(method)]
    pub fn sleep(this: &Host, millis: f64) -> JsValue;

    /// One more greeting; how many so far. `number`, or a `Promise` of one.
    #[wasm_bindgen(method)]
    pub fn count(this: &Host) -> JsValue;

    /// Show `line`.
    #[wasm_bindgen(method)]
    pub fn write(this: &Host, line: &str);
}
