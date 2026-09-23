# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — written once against five capability traits, then run four ways that must agree byte for byte. Three of the traits (`Sleep`, `ReadLine`, `WriteLine`) and the reifying context `Ctx<E>` come from `sans-effort-effects`; two (`Count`, `Lookup`) are the demo's own.

```text
  routines/  the routines, one per module: greeter (the conversation), fanout (two
             waits at once), ticker (needs only Sleep + WriteLine). no_std.
             traits.rs: the demo's own capabilities, Count and Lookup, each with
             its effect and its Ctx impl beside it (the orphan rule puts them
             there), like a module of the standard library. The routines
             themselves use only traits.
             Tests: a Recording mock + testing::run_now — no driver, one poll.

  tokio/     the native path. DemoCtx wraps sans-effort-tokio's TokioCtx (Sleep,
             ReadLine, WriteLine as tokio futures), adds Count and Lookup, and
             forwards the rest; tokio::spawn(Greeter::new(ctx).run()). No Driver.
             Tests: paused clock, output captured — three 50 ms pauses cost no
             wall time, and the transcript is checked.

  boundary/  the host vocabularies. Full carries all five capabilities' effects,
             Quiet only Sleep + WriteLine; Ctx<E> implements each trait for any
             E that can carry its effect. View/HostEffect/Encode: the tag table.
             Tests: through a Driver, as data; Greeter under Quiet is a compile_fail.

  cdylib/    the C-ABI binding over sans-effort-host: abi_version/new/start/reply/
             free/buf_free, three unsafe blocks, no mechanism. The only crate allowing unsafe.
  python/    a ctypes host that speaks ABI.md with a byte buffer and no library;
             the smallest loop: perform each effect, then reply.
  java/      a Panama (java.lang.foreign) host: the same ABI, downcalls only, no JNI.
             Shaped like a production host: asks run on virtual threads, replies
             arrive as effects finish, closed frames cancel, one driver thread calls in.

  wasm/      the second native path. JsCtx implements the traits by calling a JS
             object the caller supplies, awaiting Promises; the event loop is the
             executor. No Driver anywhere, like tokio/.
  js/        a Node host: five functions and one awaited promise. pkg/ is generated.
```

Two native runtimes and two foreign hosts, on purpose. tokio and the JS event loop are executors: the routine is spawned on them and its context makes each wait a real future — no driver. Python and Java cannot poll a Rust future, so there the routine runs behind a `Driver`, and the host replies by request id over the C ABI it decodes from `ABI.md`. The routine cannot tell which it is under.

A JS host _could_ take the Python role — hold a `Machine` in a wasm-bindgen class and step it — and would want to for a deterministic scheduler or a replay harness; the exploration this library came from has one. For running a routine in a page, the native form is the idiomatic one.

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio          # native
cargo build -p greeter_cdylib && python3 demo/python/main.py       # C ABI, ctypes
java --enable-native-access=ALL-UNNAMED demo/java/Main.java       # C ABI, Panama
nix develop --command demo:wasm && node demo/js/main.mjs           # wasm-bindgen
nix develop --command demo                                         # all four, diffed
```

`--fanout` on any host runs the two-waits-per-batch variant; `--ticker` on the Python or Java host drives a `Quiet` machine, which can only ever emit tags 4 and 5.
