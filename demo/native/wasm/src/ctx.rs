//! The routines' effect traits, served by a JS object behind a dispatcher.
//!
//! Compare `greeter_tokio`'s `TokioCtx`: there each trait method is a tokio
//! future. Here each one is a call into JS and, if JS returned a `Promise`, a
//! `JsFuture` awaiting it — which `wasm-bindgen-futures` wires to the event
//! loop's microtask queue. No effect is built and no driver polls; the
//! routine is a task on the JS event loop.
//!
//! # A Dispatcher, So the Futures Are `Send`
//!
//! An effect trait's future must be `Send`, and `JsValue`s and `JsFuture`s are
//! not. So `JsCtx` holds no JS value at all: it holds a channel to a
//! dispatcher task that owns the [`JsHost`], and each call sends it a request
//! and awaits the reply. The dispatcher calls the host's methods in the order
//! requests arrive and awaits each returned promise in a task of its own, so
//! two waits in flight stay in flight together. It is the usual way to reach
//! a resource tied to one thread from code that must be `Send`, and it costs
//! one channel hop per call.
//!
//! # Abandoned Sleeps
//!
//! A routine that races a sleep and loses drops it. Dropping a future cannot
//! reach JS — the future holds no JS value — so a dropped sleep sends the
//! dispatcher one more request, `Abandon`, and the dispatcher aborts the
//! `AbortSignal` it gave the host's `sleep`, which clears its timer. Without
//! it a lost 30-second race would keep Node's event loop alive for 30
//! seconds. It is the native counterpart of a closed frame.

use crate::host::JsHost;
use async_channel::{Receiver, Sender};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use js_sys::{Promise, Uint8Array};
use routines::effects::{count::Count, lookup::Lookup};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    env::Var,
    fs::{FsError, ReadFile, WriteFile},
    random::Random,
    spawn::{Spawn, SpawnPinned},
    time::{Now, Sleep, UnixTime},
};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};
use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, spawn_local};

/// A context whose waits are JS calls, made by a dispatcher on the JS thread.
/// Cloning shares the dispatcher, as a spawned child's context does.
#[derive(Clone, Debug)]
pub struct JsCtx {
    requests: Sender<Request>,
    /// Numbers each sleep, so an abandoned one can be named.
    tickets: Arc<AtomicU64>,
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
        let tickets = Arc::new(AtomicU64::new(0));
        (Self { requests, tickets }, Drained(drained))
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
    /// Dropped before it finishes — the losing side of a race — it has the
    /// dispatcher abort the host's timer.
    async fn sleep(&self, duration: Duration) {
        let ticket = self.tickets.fetch_add(1, Ordering::Relaxed);
        let abandon = Abandon {
            requests: Some(self.requests.clone()),
            ticket,
        };
        self.ask(|reply| Request::Sleep(duration, ticket, reply))
            .await;
        abandon.disarm();
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

impl Now for JsCtx {
    /// The epoch itself unless the host answered a non-negative integer of
    /// milliseconds.
    async fn now(&self) -> UnixTime {
        let millis = self.ask(Request::Now).await.unwrap_or(0);
        UnixTime::from_since_epoch(Duration::from_millis(millis))
    }
}

impl Random for JsCtx {
    /// No bytes unless the host answered a `Uint8Array`.
    async fn random_bytes(&self, len: u32) -> Vec<u8> {
        self.ask(|reply| Request::Random(len, reply))
            .await
            .unwrap_or_default()
    }
}

impl Var for JsCtx {
    async fn var(&self, name: String) -> Option<String> {
        self.ask(|reply| Request::Var(name, reply)).await.flatten()
    }
}

impl ReadFile for JsCtx {
    async fn read_file(&self, path: String) -> Result<Vec<u8>, FsError> {
        self.ask(|reply| Request::ReadFile(path, reply))
            .await
            .unwrap_or(Err(FsError::Other))
    }
}

impl WriteFile for JsCtx {
    async fn write_file(&self, path: String, bytes: Vec<u8>) -> Result<(), FsError> {
        self.ask(|reply| Request::WriteFile(path, bytes, reply))
            .await
            .unwrap_or(Err(FsError::Other))
    }
}

/// A child's context shares the dispatcher. Both kinds of spawn run the child
/// as a local task: JS has one thread, so there is nowhere else to go.
impl Spawn for JsCtx {
    type Child = Self;

    fn spawn<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static>(
        &self,
        f: F,
    ) {
        spawn_local(f(self.clone()));
    }
}

/// As for [`Spawn`]: a local task.
impl SpawnPinned for JsCtx {
    type Child = Self;

    fn spawn_pinned<F: FnOnce(Self) -> Fut + Send + 'static, Fut: Future<Output = ()> + 'static>(
        &self,
        f: F,
    ) {
        spawn_local(f(self.clone()));
    }
}

/// Sends `Abandon` for its ticket when dropped, unless disarmed first: a
/// sleep that finished needs no aborting.
struct Abandon {
    requests: Option<Sender<Request>>,
    ticket: u64,
}

impl Abandon {
    fn disarm(mut self) {
        self.requests = None;
    }
}

impl Drop for Abandon {
    fn drop(&mut self) {
        if let Some(requests) = self.requests.take() {
            // The dispatcher may be gone; then so is the timer.
            drop(requests.try_send(Request::Abandon(self.ticket)));
        }
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
    Sleep(Duration, u64, Sender<()>),
    /// The sleep with this ticket was dropped: abort its timer.
    Abandon(u64),
    WriteLine(String),
    Now(Sender<u64>),
    Random(u32, Sender<Vec<u8>>),
    Var(String, Sender<Option<String>>),
    ReadFile(String, Sender<Result<Vec<u8>, FsError>>),
    WriteFile(String, Vec<u8>, Sender<Result<(), FsError>>),
}

/// Serve requests until every context is gone. Host methods are called here,
/// in order; each promise is awaited in a task of its own.
async fn dispatch(host: JsHost, requests: Receiver<Request>, _done: Sender<()>) {
    // Each sleep in flight, by ticket: the controller whose signal its host
    // timer listens to.
    let timers: Rc<RefCell<HashMap<u64, AbortController>>> = Rc::default();
    while let Ok(request) = requests.recv().await {
        match request {
            Request::WriteLine(line) => host.write_line(&line),
            Request::Sleep(duration, ticket, reply) => {
                // `setTimeout` takes milliseconds as a `number`;
                // `as_secs_f64() * 1000` is exact for any duration a JS timer
                // can represent.
                let controller = AbortController::new();
                let value = host.sleep(duration.as_secs_f64() * 1000.0, &controller.signal());
                timers.borrow_mut().insert(ticket, controller);
                let timers = Rc::clone(&timers);
                spawn_local(async move {
                    settle(value).await;
                    timers.borrow_mut().remove(&ticket);
                    // The context may be gone; then nobody is waiting.
                    let _unheard = reply.send(()).await;
                });
            }
            Request::Abandon(ticket) => {
                if let Some(controller) = timers.borrow_mut().remove(&ticket) {
                    controller.abort();
                }
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
            Request::Now(reply) => {
                answer(host.now(), reply, |value| {
                    value.as_f64().and_then(safe_integer).unwrap_or(0)
                });
            }
            Request::Random(len, reply) => {
                answer(host.random_bytes(len), reply, |value| {
                    value
                        .dyn_into::<Uint8Array>()
                        .map(|bytes| bytes.to_vec())
                        .unwrap_or_default()
                });
            }
            Request::Var(name, reply) => {
                answer(host.env(&name), reply, |value| value.as_string());
            }
            Request::ReadFile(path, reply) => {
                let value = host.read_file(&path);
                spawn_local(async move {
                    let read = match settle_result(value).await {
                        Ok(value) if value.is_null() || value.is_undefined() => {
                            Err(FsError::NotFound)
                        }
                        Ok(value) => value
                            .dyn_into::<Uint8Array>()
                            .map(|bytes| bytes.to_vec())
                            .map_err(|_| FsError::Other),
                        Err(_) => Err(FsError::Other),
                    };
                    drop(reply.send(read).await);
                });
            }
            Request::WriteFile(path, bytes, reply) => {
                let value = host.write_file(&path, &Uint8Array::from(bytes.as_slice()));
                spawn_local(async move {
                    let written = settle_result(value)
                        .await
                        .map(drop)
                        .map_err(|_| FsError::Other);
                    // The context may be gone; then nobody is waiting.
                    let _unheard = reply.send(written).await;
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

/// A value JS returned, awaited if it was a `Promise`, keeping a rejection
/// apart from a value: for the file methods, where a rejection means failure.
async fn settle_result(value: JsValue) -> Result<JsValue, JsValue> {
    match value.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise).await,
        Err(value) => Ok(value),
    }
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

#[wasm_bindgen]
extern "C" {
    /// The platform's `AbortController`, in Node and browsers alike: how the
    /// dispatcher tells a host's `sleep` to stop its timer.
    type AbortController;

    #[wasm_bindgen(constructor)]
    fn new() -> AbortController;

    /// The `AbortSignal` handed to the host.
    #[wasm_bindgen(method, getter)]
    fn signal(this: &AbortController) -> JsValue;

    #[wasm_bindgen(method)]
    fn abort(this: &AbortController);
}
