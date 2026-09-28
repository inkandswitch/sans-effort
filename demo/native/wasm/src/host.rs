//! The JS object a caller supplies: the routines' effect traits as methods.
//!
//! Declared, not defined — `wasm-bindgen` generates the glue to call whatever
//! the caller passed. Each method may return its value directly or a
//! `Promise` of it; [`JsCtx`](crate::ctx::JsCtx) accepts either, so a host
//! written with plain functions and one written with `async` functions both
//! work.

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// An object with `readLine`, `lookup`, `sleep`, `count`, and `writeLine`;
    /// for the journal, also `now`, `randomBytes`, `env`, `readFile`, and
    /// `writeFile`.
    /// `Host` to JS; the `Js` prefix is the Rust side's, as with the classes.
    #[wasm_bindgen(js_name = Host)]
    pub type JsHost;

    /// The next line of input: a `string`, or a `Promise` of one; anything
    /// else (`null` at end of input) closes the input.
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

    /// The wall-clock time in milliseconds since the Unix epoch, as
    /// `Date.now()` returns it. `number`, or a `Promise` of one.
    #[wasm_bindgen(method)]
    pub fn now(this: &JsHost) -> JsValue;

    /// `len` random bytes: a `Uint8Array`, or a `Promise` of one.
    #[wasm_bindgen(method, js_name = randomBytes)]
    pub fn random_bytes(this: &JsHost, len: u32) -> JsValue;

    /// The variable `name`: a `string`, or anything else (`null`) if unset.
    /// Called `env` because `var` is a keyword in JS.
    #[wasm_bindgen(method)]
    pub fn env(this: &JsHost, name: &str) -> JsValue;

    /// The file at `path`: a `Uint8Array`, `null` if there is none, or a
    /// `Promise` of either. A rejected promise is some other failure.
    #[wasm_bindgen(method, js_name = readFile)]
    pub fn read_file(this: &JsHost, path: &str) -> JsValue;

    /// Replace the file at `path` with `bytes` (a copy the host may keep).
    /// Anything, or a `Promise`; a rejected promise is a failure.
    #[wasm_bindgen(method, js_name = writeFile)]
    pub fn write_file(this: &JsHost, path: &str, bytes: &Uint8Array) -> JsValue;
}
