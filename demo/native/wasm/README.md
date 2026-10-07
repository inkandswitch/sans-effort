# greeter_wasm

The demo's routines in Node or a browser, with no driver. JS is a runtime host like tokio: its event loop is an executor, and `wasm-bindgen-futures` bridges Rust wakers to microtasks. So each routine runs as a task on that loop, and the context — `JsCtx` — makes each effect trait a call into a JS object the caller supplies, or, for spawning, a task of its own.

```text
  host.rs        the extern block, on a JS object: readLine · writeLine · lookup · count ·
                 sleep(ms, signal) · now · randomBytes · env · readFile · writeFile
  ctx.rs         JsCtx: every trait the routines use, over that object, through a
                 dispatcher task, so its futures are Send; awaits a Promise if one comes
                 back; a dropped sleep aborts its signal
  greeter.rs     class Greeter    { constructor(host); run(): Promise<void> }
  fanout.rs      class Fanout     ⎫
  ping_pong.rs   class PingPong   ⎪
  front_desk.rs  class FrontDesk  ⎬ the same shape, each its routine
  ring.rs        class Ring       ⎪
  journal.rs     class Journal    ⎪
  deadline.rs    class Deadline   ⎭
  faults/        class Deadlock: a host check, not an example (demo:faults)
```

## Build

```sh
nix develop --command demo:wasm   # cargo build --target wasm32 + wasm-bindgen --target nodejs → demo/native/js/pkg
node demo/native/js/main.mjs     # or --fanout, --ping-pong, --front-desk, --ring, --journal, --deadline
node demo/native/js/main.mjs --deadlock   # a host check (demo:faults): exits 13
node demo/native/js/main.mjs --ring --seed 7   # a host check (demo:stress): answers settle out of order
```

For a page, run `wasm-bindgen --target web` instead and `await init()` before constructing.

## Use

```js
import { Greeter } from "./pkg/greeter_wasm.js";

const lines = ["alice", "bob"][Symbol.iterator]();
let greeted = 0;

await new Greeter({
  readLine: () => lines.next().value ?? null,         // string | Promise<string>; null: end of input
  lookup:   (name) => GREETINGS[name] ?? "Greetings", // string | Promise<string>
  sleep:    (ms, signal) => new Promise((r) => {             // aborted: the routine stopped waiting
    const timer = setTimeout(r, ms);
    signal.addEventListener("abort", () => { clearTimeout(timer); r(); }, { once: true });
  }),
  count:    () => ++greeted,                          // number | Promise<number>
  writeLine: (line) => console.log(line),
}).run();
```

Every method may return its value or a `Promise` of it. `readLine` returning anything but a string closes the input: the routine sees `ReadLineError::Closed`. The generated `pkg/greeter_wasm.d.ts` has the exact types.

## Why Not a `Machine` Here?

A JS host *could* take Python's role — hold a `Machine` in a class and step the routine by request id — and would want to for a deterministic scheduler or a replay harness, where JS must own the loop. For running a routine in a page or a Node script, the native form is the idiomatic one: no ids, no codec, no status mirror, and the routine cannot tell it is not on tokio.
