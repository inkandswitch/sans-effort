//! The five capability traits, served by a JS object behind a dispatcher.
//!
//! Compare `greeter_tokio`'s `TokioCtx`: there each trait method is a tokio
//! future. Here each one is a call into JS and, if JS returned a `Promise`, a
//! `JsFuture` awaiting it — which `wasm-bindgen-futures` wires to the event
//! loop's microtask queue. No effect is built and no driver polls; the
//! routine is a task on the JS event loop.
//!
//! # A Dispatcher, So the Futures Are `Send`
//!
//! A capability's future must be `Send`, and `JsValue`s and `JsFuture`s are
//! not. So `JsCtx` holds no JS value at all: it holds a channel to a
//! dispatcher task that owns the [`JsHost`], and each call sends it a request
//! and awaits the reply. The dispatcher calls the host's methods in the order
//! requests arrive and awaits each returned promise in a task of its own, so
//! two waits in flight stay in flight together. It is the usual way to reach
//! a resource tied to one thread from code that must be `Send`, and it costs
//! one channel hop per call.

use crate::host::JsHost;
use async_channel::{Receiver, Sender};
use core::time::Duration;
use js_sys::Promise;
use routines::traits::{Count, Lookup};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    spawn::Spawn,
    time::Sleep,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

/// A context whose waits are JS calls, made by a dispatcher on the JS thread.
/// Cloning shares the dispatcher, as a spawned child's context does.
#[derive(Clone, Debug)]
pub struct JsCtx {
    requests: Sender<Request>,
}

impl JsCtx {
    /// A context calling into `host`, and the dispatcher's [`Drained`]: await
    /// it after the routine, so every request the routine made — its last
    /// `write_line` included — has reached the host. The dispatcher runs
    /// until every clone of the context is dropped, spawned children's too.
    #[must_use]
    pub fn new(host: JsHost) -> (Self, Drained) {
        let (requests, incoming) = async_channel::unbounded();
        let (done, drained) = async_channel::bounded(1);
        spawn_local(dispatch(host, incoming, done));
        (Self { requests }, Drained(drained))
    }

    /// Send the dispatcher a request that carries its reply channel, and
    /// await the reply. `None` if the dispatcher is gone.
    async fn ask<T>(&self, request: impl FnOnce(Sender<T>) -> Request) -> Option<T> {
        let (reply, answer) = async_channel::bounded(1);
        self.requests.send(request(reply)).await.ok()?;
        answer.recv().await.ok()
    }
}

impl Sleep for JsCtx {
    async fn sleep(&self, duration: Duration) {
        self.ask(|reply| Request::Sleep(duration, reply)).await;
    }
}

impl Count for JsCtx {
    /// Zero unless the host answered a non-negative integer a JS `number`
    /// holds exactly.
    async fn count(&self) -> u64 {
        self.ask(Request::Count).await.unwrap_or(0)
    }
}

impl Lookup for JsCtx {
    async fn lookup(&self, name: String) -> String {
        self.ask(|reply| Request::Lookup(name, reply))
            .await
            .unwrap_or_default()
    }
}

impl ReadLine for JsCtx {
    /// A host returning a non-string (`null` or `undefined` at end of input)
    /// closes the input; so does a dispatcher that is gone.
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.ask(Request::ReadLine)
            .await
            .unwrap_or(Err(ReadLineError::Closed))
    }
}

impl WriteLine for JsCtx {
    /// Queued for the dispatcher, in order with every other call.
    fn write_line(&self, line: String) {
        drop(self.requests.try_send(Request::WriteLine(line)));
    }
}

/// A child's context shares the dispatcher. Both methods run the child as a
/// local task: JS has one thread, so there is nowhere else to go.
impl Spawn for JsCtx {
    type Child = Self;

    fn spawn<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static>(
        &self,
        f: F,
    ) {
        spawn_local(f(self.clone()));
    }

    fn spawn_pinned<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + 'static>(
        &self,
        f: F,
    ) {
        spawn_local(f(self.clone()));
    }
}

/// Resolves once the dispatcher has served every request and stopped.
#[derive(Debug)]
pub struct Drained(Receiver<()>);

impl Drained {
    /// Wait for it.
    pub async fn wait(self) {
        // Nothing is ever sent: the dispatcher drops its end when it stops.
        let _closed = self.0.recv().await;
    }
}

/// What a context asks the dispatcher to do, with where to send the answer.
#[derive(Debug)]
enum Request {
    Count(Sender<u64>),
    Lookup(String, Sender<String>),
    ReadLine(Sender<Result<String, ReadLineError>>),
    Sleep(Duration, Sender<()>),
    WriteLine(String),
}

/// Serve requests until every context is gone. Host methods are called here,
/// in order; each promise is awaited in a task of its own.
async fn dispatch(host: JsHost, requests: Receiver<Request>, _done: Sender<()>) {
    while let Ok(request) = requests.recv().await {
        match request {
            Request::WriteLine(line) => host.write_line(&line),
            Request::Sleep(duration, reply) => {
                // `setTimeout` takes milliseconds as a `number`;
                // `as_secs_f64() * 1000` is exact for any duration a JS timer
                // can represent.
                let value = host.sleep(duration.as_secs_f64() * 1000.0);
                answer(value, reply, |_| ());
            }
            Request::Count(reply) => {
                answer(host.count(), reply, |value| {
                    value.as_f64().and_then(safe_integer).unwrap_or(0)
                });
            }
            Request::Lookup(name, reply) => {
                answer(host.lookup(&name), reply, |value| {
                    value.as_string().unwrap_or_default()
                });
            }
            Request::ReadLine(reply) => {
                answer(host.read_line(), reply, |value| {
                    value.as_string().ok_or(ReadLineError::Closed)
                });
            }
        }
    }
}

/// Await `value` if it is a promise, in a local task, and send what it
/// settled to — read by `read` — back through `reply`.
fn answer<T: 'static>(value: JsValue, reply: Sender<T>, read: impl FnOnce(JsValue) -> T + 'static) {
    spawn_local(async move {
        drop(reply.send(read(settle(value).await)).await);
    });
}

/// `Number.MAX_SAFE_INTEGER`: the largest integer a JS `number` holds exactly.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// A JS `number` as a `u64`, if it is a non-negative integer that `f64`
/// represents exactly. Anything else counts as zero, which the routine will
/// print and the host's author will notice.
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
/// yields its rejection reason as the value, which the readers above treat
/// as "not the type I wanted" — a host bug surfaces as a default, not a trap.
async fn settle(value: JsValue) -> JsValue {
    match value.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise)
            .await
            .unwrap_or_else(|rejection| rejection),
        Err(value) => value,
    }
}
