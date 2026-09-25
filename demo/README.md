# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — written once against five effect traits, then run four ways that must agree byte for byte; and two routines that spawn children and talk to them over plain channels. Four of the traits (`Sleep`, `ReadLine`, `WriteLine`, `Spawn`) and the reifying context `Ctx<E>` come from `sans-effort-effects`; two (`Count`, `Lookup`) are the demo's own.

```text
  routines/        shared by both paths. The routines, one per module: greeter (the
                   conversation), fanout (two waits at once), ticker (needs only Sleep
                   + WriteLine), ping_pong (spawns a child and plays over two
                   async-channels), front_desk (spawns a pinned clerk per name; each
                   replies on a one-shot channel), ring (16 routines pass a counter
                   4000 hops; the cost of a hop). no_std.
                   traits.rs: the demo's own effect traits, Count and Lookup, each
                   with its effect and its Ctx impl beside it (the orphan rule puts
                   them there). The routines themselves use only traits.
                   Tests: a Recording mock + testing::run_now — no driver, one poll.

  native/          the routine runs as an ordinary task on an executor. No Driver.
    tokio/         DemoCtx wraps sans-effort-tokio's TokioCtx (Sleep, ReadLine,
                   WriteLine, Spawn as tokio futures and tasks), adds Count and Lookup,
                   and forwards the rest; tokio::spawn(Greeter::new(ctx).run()).
                   Tests: paused clock and multithreaded runs, output captured.
    wasm/          JsCtx implements the traits over a JS object the caller supplies —
                   through a dispatcher task, so its futures are Send; the event loop
                   is the executor. Classes Greeter, Fanout, PingPong, FrontDesk, Ring.
    js/            a Node host: five functions and one awaited promise. pkg/ is
                   generated.

  driven/          the routine runs behind a Driver; a foreign host replies by id.
    boundary/      the host vocabularies. Full carries the effects of all five effect
                   traits and, with the default `table` feature, spawning (tags 6 and 7:
                   split registers the child in sans-effort-host's table); Quiet only
                   Sleep + WriteLine. View/HostEffect/Encode: the tag table.
                   Tests: through a Driver, as data; Greeter under Quiet is a
                   compile_fail; a deterministic Rust router runs ping-pong and front
                   desk across machines.
    cdylib/        the C-ABI binding over sans-effort-host: abi_version/new…/resume/
                   reply/wakes/free/buf_free, three unsafe blocks, no mechanism. The
                   only crate allowing unsafe.
    python/        a ctypes host that speaks ABI.md with a byte buffer and no library:
                   perform each effect, then reply; resume spawned children and each
                   machine a woke frame names; ask wakes() when nothing is queued.
    java/          a Panama (java.lang.foreign) host: the same ABI, downcalls only, no
                   JNI. Parallel: a pool of driver threads polls different machines at
                   once (migrating machines move between them, pinned ones stay on the
                   worker that first resumed them); asks run on virtual threads; woke
                   frames schedule resumes; closed frames cancel. --trace logs which
                   thread polled what.
```

Two native runtimes and two foreign hosts, on purpose. tokio and the JS event loop are executors: the routine is spawned on them and its context makes each wait a real future — no driver. Python and Java cannot poll a Rust future, so there the routine runs behind a `Driver`, and the host replies by request id over the C ABI it decodes from `ABI.md`. The routine cannot tell which it is under.

A JS host _could_ take the Python role — hold a `Machine` in a wasm-bindgen class and step it — and would want to for a deterministic scheduler or a replay harness; the exploration this library came from has one. For running a routine in a page, the native form is the idiomatic one.

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio               # native: tokio
nix develop --command demo:wasm && node demo/native/js/main.mjs         # native: wasm-bindgen
cargo build -p greeter_cdylib && python3 demo/driven/python/main.py     # driven: C ABI, ctypes
java --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java     # driven: C ABI, Panama
nix develop --command demo                                              # all four, diffed
```

`--fanout` on any host runs the two-waits-per-batch variant; `--ping-pong`, `--front-desk`, and `--ring` run the spawning routines (`--ring` also prints the host's cost per hop to stderr); `--ticker` on the Python or Java host drives a `Quiet` machine, which can only ever emit tags 4 and 5.
