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
//   node demo/js/main.mjs [--fanout | --ping-pong | --front-desk | --ring]

import { createRequire } from "node:module";

// `wasm-bindgen --target nodejs` emits CommonJS.
const require = createRequire(import.meta.url);
const { Greeter, Fanout, PingPong, FrontDesk, Ring } = require("./pkg/greeter_wasm.js");

const GREETINGS = { alice: "Hello", bob: "Hi", carol: "Hey" };

/** The routine's world: scripted input, a fixed directory, a real clock. */
function host(script) {
  const lines = script[Symbol.iterator]();
  let greeted = 0;

  return {
    readLine: () => lines.next().value ?? null, // null: end of input
    lookup: (name) => GREETINGS[name] ?? "Greetings",
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    count: () => ++greeted,
    writeLine: (line) => console.log(line),
  };
}

const mode = (flag) => process.argv.includes(flag);
const routine = mode("--fanout")
  ? new Fanout(host(["bob", "carol"]))
  : mode("--ping-pong")
    ? new PingPong(host([]))
    : mode("--ring")
      ? new Ring(host([]))
      : mode("--front-desk")
        ? new FrontDesk(host(["alice", "bob", "carol"]))
        : new Greeter(host(["alice", "bob"]));
const began = performance.now();
await routine.run();
if (mode("--ring")) {
  const hops = 16 * 250;
  const ms = performance.now() - began;
  console.error(`${hops} hops in ${ms.toFixed(1)} ms: ${((ms * 1e3) / hops).toFixed(2)} µs per hop`);
}
