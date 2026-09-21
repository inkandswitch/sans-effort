# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — written
once against four capability traits, then run three ways that must agree
byte for byte.

```
  routines/  the routines, one per module: greeter (the conversation), fanout (two
             waits at once), ticker (needs only Sleep + WriteLine); traits.rs. no_std.
             Imports Run and join, nothing else.
             Tests: a Recording mock + testing::run_now — no driver, one poll.

  tokio/     the native path. TokioCtx implements the traits with tokio futures;
             tokio::spawn(Greeter::new(ctx).run()). No Driver anywhere.
             Tests: paused clock — three 50 ms pauses cost no wall time.

  wire/      the reifying context. Request structs (Lookup, ReadLine, …); Ctx<E>
             implements each trait for any E: From<Asked<…>>; Full carries all five,
             Quiet only Sleep + Write. View/HostEffect/Encode: the tag table.
             Tests: through a Driver, as data; Greeter under Quiet is a compile_fail.

  cdylib/    the C-ABI skin over effect_routine_host: new/start/reply/free/buf_free,
             three unsafe blocks, no mechanism. The only crate allowing unsafe.
  python/    a ctypes host that speaks ABI.md with a byte buffer and no library.

  wasm/      the second native path. JsCtx implements the traits by calling a JS
             object the caller supplies, awaiting Promises; the event loop is the
             executor. No Driver anywhere, like tokio/.
  js/        a Node host: five functions and one awaited promise. pkg/ is generated.
```

Two native runtimes and one foreign host, on purpose. tokio and the JS event
loop are executors: the routine is spawned on them and its context makes each
wait a real future — no driver. Python cannot poll a Rust future, so there the
routine runs behind a `Driver`, and Python replies by request id over the C
ABI it decodes from `ABI.md`. The routine cannot tell which it is under.

A JS host _could_ take the Python role — hold a `Machine` in a wasm-bindgen
class and step it — and would want to for a deterministic scheduler or a
replay harness; the exploration this library came from has one. For running
a routine in a page, the native form is the idiomatic one.

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio          # native
cargo build -p greeter_cdylib && python3 demo/python/main.py       # C ABI
nix develop --command demo:wasm && node demo/js/main.mjs           # wasm-bindgen
nix develop --command demo                                         # all three, diffed
```

`--fanout` on any host runs the two-waits-per-batch variant; `--ticker` on
the Python host drives a `Quiet` machine, which can only ever emit tags 4
and 5.
