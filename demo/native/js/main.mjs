// The greeter in Node, from the wasm-bindgen module in ./pkg.
//
// The third runtime for the same routine, and the second *native* one: like
// tokio, the JS event loop is an executor, so the routine runs as a task on
// it and nothing here loops over effects. The host supplies five functions —
// the routine's effect traits — and awaits one promise. Compare
// ../python/main.py, where Python cannot poll a Rust future and so drives the
// routine step by step over the C ABI.
//
//   nix develop --command demo:wasm
//   node demo/native/js/main.mjs [--fanout | --ping-pong | --front-desk | --ring | --journal]

import { createRequire } from "node:module";

// `wasm-bindgen --target nodejs` emits CommonJS.
const require = createRequire(import.meta.url);
const { Greeter, Fanout, PingPong, FrontDesk, Ring, Journal } = require("./pkg/greeter_wasm.js");

const GREETINGS = { alice: "Hello", bob: "Hi", carol: "Hey" };

// The journal's world, the same in every host so transcripts agree: one
// variable, files in memory, a clock that starts at 1 700 000 000 000 ms and
// moves a second per reading, and random bytes that count up from 00.
const ENVIRONMENT = { JOURNAL: "notes.txt" };
const EPOCH_MILLIS = 1_700_000_000_000;

/** The routine's world: scripted input, a fixed directory, a real clock. */
function host(script) {
  const lines = script[Symbol.iterator]();
  let greeted = 0;
  const files = new Map();
  let readings = 0;
  let nextByte = 0;

  return {
    readLine: () => lines.next().value ?? null, // null: end of input
    lookup: (name) => GREETINGS[name] ?? "Greetings",
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    count: () => ++greeted,
    writeLine: (line) => console.log(line),
    now: () => EPOCH_MILLIS + 1_000 * readings++,
    randomBytes: (len) => Uint8Array.from({ length: len }, () => nextByte++ % 256),
    env: (name) => ENVIRONMENT[name] ?? null,
    readFile: (path) => files.get(path) ?? null, // null: no such file
    writeFile: (path, bytes) => void files.set(path, bytes),
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
        : mode("--journal")
          ? new Journal(host([]))
          : new Greeter(host(["alice", "bob"]));
const began = performance.now();
await routine.run();
if (mode("--ring")) {
  const hops = 16 * 250;
  const ms = performance.now() - began;
  console.error(`${hops} hops in ${ms.toFixed(1)} ms: ${((ms * 1e3) / hops).toFixed(2)} µs per hop`);
}
