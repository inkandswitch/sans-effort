# greeter_wasm

The greeter in Node or a browser, with no driver. JS is a runtime host like tokio: its event loop is an executor, and `wasm-bindgen-futures` bridges Rust wakers to microtasks. So the routine runs as a task on that loop, and the context — `JsCtx` — makes each of its five waits a call into a JS object the caller supplies.

```
  host.rs     the extern block: readLine · lookup · sleep · count · write on a JS object
  ctx.rs      JsCtx: the five traits over that object; awaits a Promise if one comes back
  greeter.rs  class Greeter { constructor(host); run(): Promise<void> }
  fanout.rs   class Fanout  { constructor(host); run(): Promise<void> }
```

## Build

```sh
nix develop --command demo:wasm        # cargo build --target wasm32 + wasm-bindgen --target nodejs → ../js/pkg
node ../js/main.mjs                    # or --fanout
```

For a page, run `wasm-bindgen --target web` instead and `await init()` before constructing.

## Use

```js
import { Greeter } from "./pkg/greeter_wasm.js";

const lines = ["alice", "bob"][Symbol.iterator]();
let greeted = 0;

await new Greeter({
  readLine: () => lines.next().value ?? "quit",       // string | Promise<string>
  lookup:   (name) => GREETINGS[name] ?? "Greetings", // string | Promise<string>
  sleep:    (ms) => new Promise((r) => setTimeout(r, ms)),
  count:    () => ++greeted,                          // number | Promise<number>
  write:    (line) => console.log(line),
}).run();
```

Every method may return its value or a `Promise` of it. `readLine` returning anything but a string ends the conversation. The generated `pkg/greeter_wasm.d.ts` has the exact types.

## Why not a `Machine` here?

A JS host *could* take Python's role — hold a `Machine` in a class and step the routine by request id — and would want to for a deterministic scheduler or a replay harness, where JS must own the loop. For running a routine in a page or a Node script, the native form is the idiomatic one: no ids, no codec, no status mirror, and the routine cannot tell it is not on tokio.
