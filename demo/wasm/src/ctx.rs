//! The five capability traits, served by a JS object.
//!
//! Compare `greeter_tokio`'s `TokioCtx`: there each trait method is a tokio
//! future. Here each one is a call into JS, and, if JS returned a `Promise`,
//! a `JsFuture` awaiting it — which `wasm-bindgen-futures` wires to the
//! event loop's microtask queue. No effect is built and no driver polls; the
//! routine is a task on the JS event loop.

use crate::host::JsHost;
use core::time::Duration;
use js_sys::Promise;
use routines::traits::{Count, Lookup, ReadLine, Sleep, WriteLine};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// A context whose waits are JS calls.
pub struct JsCtx {
    host: JsHost,
}

impl JsCtx {
    /// A context calling into `host`.
    #[must_use]
    pub const fn new(host: JsHost) -> Self {
        Self { host }
    }
}

impl Sleep for JsCtx {
    /// `setTimeout` takes milliseconds as a `number`; `as_secs_f64() * 1000`
    /// is exact for any duration a JS timer can represent.
    async fn sleep(&self, duration: Duration) {
        settle(self.host.sleep(duration.as_secs_f64() * 1000.0)).await;
    }
}

impl Count for JsCtx {
    /// The JS `number` is read as a `u64` if it is a non-negative integer
    /// within `Number.MAX_SAFE_INTEGER`; anything else counts as zero, which
    /// the routine will print and the host author will notice.
    async fn count(&self) -> u64 {
        settle(self.host.count())
            .await
            .as_f64()
            .and_then(safe_integer)
            .unwrap_or(0)
    }
}

impl Lookup for JsCtx {
    async fn lookup(&self, name: String) -> String {
        settle(self.host.lookup(&name))
            .await
            .as_string()
            .unwrap_or_default()
    }
}

impl ReadLine for JsCtx {
    /// A host returning a non-string (`undefined` at end of input, say) ends
    /// the conversation.
    async fn read_line(&self) -> String {
        settle(self.host.read_line())
            .await
            .as_string()
            .unwrap_or_else(|| String::from("quit"))
    }
}

impl WriteLine for JsCtx {
    fn write_line(&self, line: String) {
        self.host.write_line(&line);
    }
}

impl core::fmt::Debug for JsCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsCtx").finish_non_exhaustive()
    }
}

/// `Number.MAX_SAFE_INTEGER`: the largest integer a JS `number` holds exactly.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// A JS `number` as a `u64`, if it is a non-negative integer that `f64`
/// represents exactly.
fn safe_integer(f: f64) -> Option<u64> {
    (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f <= MAX_SAFE_INTEGER).then(|| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked just above: finite, integral, and within 0..=2^53 - 1"
        )]
        let n = f as u64;
        n
    })
}

/// A value JS returned, awaited if it was a `Promise`. A rejected promise
/// yields its rejection reason as the value, which the callers above treat
/// as "not the type I wanted" — a host bug surfaces as a default, not a trap.
async fn settle(value: JsValue) -> JsValue {
    match value.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise)
            .await
            .unwrap_or_else(|rejection| rejection),
        Err(value) => value,
    }
}
