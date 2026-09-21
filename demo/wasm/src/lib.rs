//! The greeter in the browser (or Node), with no driver.
//!
//! JS is a _runtime_ host, like tokio: it has an event loop, and
//! `wasm-bindgen-futures` bridges Rust wakers to JS microtasks. So the
//! routine is not stepped from outside here — it is spawned on that loop,
//! exactly as `greeter_tokio` spawns it on tokio, and the context that makes
//! that possible is [`ctx::JsCtx`]: each of the five traits is implemented by
//! calling a method on a JS object the caller supplies, awaiting the returned
//! `Promise` where the trait is `async`.
//!
//! ```text
//!   JS:  await new Greeter({
//!          readLine: () => …,           // string | Promise<string>
//!          lookup:   (name) => …,       // string | Promise<string>
//!          sleep:    (ms) => …,         // void   | Promise<void>
//!          count:    () => …,           // number | Promise<number>
//!          write:    (line) => …,       // void
//!        }).run();
//! ```
//!
//! Compare `../cdylib` and `../python`: there Python cannot poll a Rust
//! future, so the routine runs behind a `Driver` and Python replies by id. A
//! JS host _could_ be driven the same way — hold a `Machine` in a
//! `#[wasm_bindgen]` class and step it — and would want to be for a
//! deterministic scheduler or a replay harness; the exploration this library
//! came from has one. For running a routine in a page, the native form is
//! the idiomatic one, and it is what `demo/js/main.mjs` uses.
//!
//! # `Send`
//!
//! `JsCtx` holds a `JsValue`, which is `!Send`, so `Greeter<JsCtx>::run()` is
//! `!Send` too. Nothing here asks: `future_to_promise` needs only `'static`,
//! because wasm32 in a JS host is single-threaded. The same routine is
//! `Send` under `TokioCtx`; the traits never had to say either way.
//!
//! # Panics
//!
//! `wasm32-unknown-unknown` is `panic = "abort"`: a panicking routine traps,
//! the promise never settles, and the host sees a `RuntimeError`.

#![expect(
    clippy::missing_const_for_fn,
    reason = "wasm-bindgen cannot export `const fn`"
)]

pub mod ctx;
pub mod fanout;
pub mod greeter;
pub mod host;
