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
//! # From JS
//!
//! The host is a plain object with five methods. Each may return its value
//! directly or a `Promise` of it; the context awaits either. The simplest
//! host is synchronous except where it genuinely waits:
//!
//! ```js
//! import { Greeter } from "./pkg/greeter_wasm.js";
//!
//! const lines = ["alice", "bob"][Symbol.iterator]();
//! let greeted = 0;
//!
//! await new Greeter({
//!   readLine: () => lines.next().value ?? "quit",
//!   lookup:   (name) => ({ alice: "Hello", bob: "Hi" })[name] ?? "Greetings",
//!   sleep:    (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
//!   count:    () => ++greeted,
//!   writeLine: (line) => console.log(line),
//! }).run();
//! ```
//!
//! In a page, the same five methods reach the DOM, and `readLine` resolves
//! when the user presses Enter:
//!
//! ```js
//! import init, { Greeter } from "./pkg/greeter_wasm.js";   // --target web
//! await init();
//!
//! const input = document.querySelector("#line");
//! const output = document.querySelector("#transcript");
//!
//! const host = {
//!   readLine: () => new Promise((resolve) => {
//!     input.addEventListener("keydown", (e) => {
//!       if (e.key === "Enter") { resolve(input.value); input.value = ""; }
//!     }, { once: true });
//!   }),
//!   lookup:   async (name) => (await fetch(`/greeting/`)).text(),
//!   sleep:    (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
//!   count:    () => Number(localStorage.greeted = (Number(localStorage.greeted) || 0) + 1),
//!   writeLine: (line) => output.append(line, document.createElement("br")),
//! };
//!
//! await new Greeter(host).run();
//! ```
//!
//! `Fanout` has the same constructor; its `run` awaits two host promises at
//! once, so `lookup` and `count` are in flight together.
//! `PingPong` and `FrontDesk` have it too, and spawn children of their own;
//! each `run` resolves once every child has finished.
//!
//! Compare `../../driven/cdylib` and `../../driven/python`: there Python cannot poll a Rust
//! future, so the routine runs behind a `Driver` and Python replies by id. A
//! JS host _could_ be driven the same way — hold a `Machine` in a
//! `#[wasm_bindgen]` class and step it — and would want to be for a
//! deterministic scheduler or a replay harness; the exploration this library
//! came from has one. For running a routine in a page, the native form is
//! the idiomatic one, and it is what `demo/native/js/main.mjs` uses.
//!
//! # `Send`
//!
//! Effect trait futures must be `Send`, and a `JsValue` is not, so
//! [`ctx::JsCtx`] holds none: it talks over a channel to a dispatcher task
//! that owns the host object. `Greeter<JsCtx>::run()` is then `Send`, like
//! the same routine under `TokioCtx` — though nothing here needs it to be:
//! wasm32 in a JS host is single-threaded, and every task is local.
//!
//! # Panics
//!
//! `wasm32-unknown-unknown` is `panic = "abort"`: a panicking routine traps,
//! the promise never settles, and the host sees a `RuntimeError`.

pub mod ctx;
pub mod fanout;
pub mod front_desk;
pub mod greeter;
pub mod host;
pub mod ping_pong;
pub mod ring;
