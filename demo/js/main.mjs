// The greeter in Node, from the wasm-bindgen module in ./pkg.
//
// The third runtime for the same routine, and the second *native* one: like
// tokio, the JS event loop is an executor, so the routine runs as a task on
// it and nothing here loops over effects. The host supplies five functions —
// the routine's capabilities — and awaits one promise. Compare
// ../python/main.py, where Python cannot poll a Rust future and so drives the
// routine step by step over the C ABI.
//
//   nix develop --command demo:wasm
//   node demo/js/main.mjs [--fanout]

import { createRequire } from "node:module";

// `wasm-bindgen --target nodejs` emits CommonJS.
const require = createRequire(import.meta.url);
const { Greeter, Fanout } = require("./pkg/greeter_wasm.js");

const GREETINGS = { alice: "Hello", bob: "Hi", carol: "Hey" };

/** The routine's world: scripted input, a fixed directory, a real clock. */
function host(script) {
  const lines = script[Symbol.iterator]();
  let greeted = 0;

  return {
    readLine: () => lines.next().value ?? "quit",
    lookup: (name) => GREETINGS[name] ?? "Greetings",
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    count: () => ++greeted,
    writeLine: (line) => console.log(line),
  };
}

const fanout = process.argv.includes("--fanout");
await (fanout ? new Fanout(host(["bob", "carol"])) : new Greeter(host(["alice", "bob"]))).run();
