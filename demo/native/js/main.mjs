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
//   node demo/native/js/main.mjs [--fanout | --ping-pong | --front-desk | --ring | --journal | --deadline | --deadlock] [--seed N]
//
// `--seed N` makes the host adversarial: every answer arrives as a promise
// that settles after a random 0–3 ms, so answers asked together settle in
// varied orders. The seed reproduces the order. The transcript must not change.

import { createRequire } from "node:module";

// `wasm-bindgen --target nodejs` emits CommonJS.
const require = createRequire(import.meta.url);
const { Deadline, Deadlock, Greeter, Fanout, PingPong, FrontDesk, Ring, Journal } = require("./pkg/greeter_wasm.js");

const GREETINGS = { alice: "Hello", bob: "Hi", carol: "Hey" };

// The journal's world, the same in every host so transcripts agree: one
// variable, files in memory, a clock that starts at 1 700 000 000 000 ms and
// moves a second per reading, and random bytes that count up from 00.
const ENVIRONMENT = { JOURNAL: "notes.txt" };
const EPOCH_MILLIS = 1_700_000_000_000;

/** mulberry32: a small seeded generator of floats in [0, 1). */
function dice(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const seedAt = process.argv.indexOf("--seed");
const roll = seedAt >= 0 ? dice(Number(process.argv[seedAt + 1])) : null;

/** The answer — or, seeded, a promise of it that settles a random 0–3 ms later. */
const late = (value) =>
  roll === null ? value : new Promise((resolve) => setTimeout(() => resolve(value), Math.floor(roll() * 4)));

/** The routine's world: scripted input, a fixed directory, a real clock. */
function host(script) {
  const lines = script[Symbol.iterator]();
  let greeted = 0;
  const files = new Map();
  let readings = 0;
  let nextByte = 0;

  return {
    readLine: () => late(lines.next().value ?? null), // null: end of input
    lookup: (name) => late(GREETINGS[name] ?? "Greetings"),
    // Aborted when the routine stops waiting (the losing side of a race):
    // clear the timer, or it keeps the event loop alive until it fires.
    sleep: (ms, signal) =>
      new Promise((resolve) => {
        const timer = setTimeout(resolve, ms);
        signal.addEventListener("abort", () => { clearTimeout(timer); resolve(); }, { once: true });
      }),
    count: () => late(++greeted),
    writeLine: (line) => console.log(line),
    now: () => late(EPOCH_MILLIS + 1_000 * readings++),
    randomBytes: (len) => late(Uint8Array.from({ length: len }, () => nextByte++ % 256)),
    env: (name) => late(ENVIRONMENT[name] ?? null),
    readFile: (path) => late(files.get(path) ?? null), // null: no such file
    writeFile: (path, bytes) => late(void files.set(path, bytes)),
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
          : mode("--deadline")
            ? new Deadline(host([]))
            : mode("--deadlock")
              ? new Deadlock(host([]))
              : new Greeter(host(["alice", "bob"]));
const began = performance.now();
await routine.run();
if (mode("--ring")) {
  const hops = 16 * 250;
  const ms = performance.now() - began;
  console.error(`${hops} hops in ${ms.toFixed(1)} ms: ${((ms * 1e3) / hops).toFixed(2)} µs per hop`);
}
