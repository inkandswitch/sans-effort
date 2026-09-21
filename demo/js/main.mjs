// The greeter driven from JS, from the wasm-bindgen module in ./pkg.
// The routine is `greeter`, under the reifying context from `greeter_wire`.
//
// The third host of the same routine. Where ../python/main.py decodes bytes
// with a tag table it wrote itself, this one gets classes and typed getters
// from wasm-bindgen and has nothing to decode. What it shares with the other
// two is the loop: perform each effect, reply by request id, queue whatever
// comes back. Fan-out is the same loop.
//
//   nix develop --command demo:wasm
//   node demo/js/main.mjs [--fanout]

import { createRequire } from "node:module";

// `wasm-bindgen --target nodejs` emits CommonJS.
const require = createRequire(import.meta.url);
const { Greeter, Kind, Status } = require("./pkg/greeter_wasm.js");

const GREETINGS = { alice: "Hello", bob: "Hi", carol: "Hey" };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Perform each effect and reply by id until the routine completes. */
async function drive(greeter, script) {
  const lines = script[Symbol.iterator]();
  const written = [];
  let greeted = 0;

  const queue = [...greeter.start()];

  while (queue.length > 0) {
    const e = queue.shift();

    switch (e.kind) {
      case Kind.Write:
        written.push(e.text);
        console.log(e.text);
        continue;
      case Kind.ReadLine:
        queue.push(...greeter.replyStr(e.id, lines.next().value ?? "quit"));
        break;
      case Kind.Lookup:
        queue.push(...greeter.replyStr(e.id, GREETINGS[e.name] ?? "Greetings"));
        break;
      case Kind.Sleep:
        await sleep(e.millis);
        queue.push(...greeter.replyUnit(e.id));
        break;
      case Kind.Count:
        greeted += 1;
        queue.push(...greeter.replyNumber(e.id, greeted));
        break;
      default:
        throw new Error(`unknown effect kind ${e.kind}`);
    }

  }

  if (greeter.status !== Status.Complete) {
    throw new Error(`routine ended with status `);
  }
  return written;
}

const fanout = process.argv.includes("--fanout");
const greeter = fanout ? Greeter.fanout() : new Greeter();
try {
  await drive(greeter, fanout ? ["bob", "carol"] : ["alice", "bob"]);
} finally {
  greeter.free();
}
