# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — written once against five effect traits, then run four ways that must agree byte for byte; and routines that spawn children and talk to them over plain channels (ping-pong, the front desk, a ring, two deadline races). Three of the greeter's traits (`Sleep`, `ReadLine`, `WriteLine`) and the reifying context `Ctx<E>` come from `sans-effort-effects`, as do the spawning routines' `Spawn` and `SpawnPinned`; two (`Count`, `Lookup`) are the demo's own. A journal routine uses the rest of the standard library — `Var`, `ReadFile`, `WriteFile`, `Now`, `Random` — and every host answers it from the same small world (one variable, files in memory, a clock that moves a second per reading, counting random bytes), so its transcripts agree too.

```text
  routines/        shared by both paths. The routines, one per module: greeter (the
                   conversation), fanout (two waits at once), ticker (needs only Sleep
                   + WriteLine), ping_pong (spawns a child and plays over two
                   async-channels), front_desk (spawns a pinned clerk per name; each
                   replies on a one-shot channel), ring (16 routines pass a counter
                   4000 hops; the cost of a hop), journal (appends entries stamped
                   with the time and a random id to a file the environment names),
                   deadline (two workers, each raced against a sleep: the quick one
                   wins, the slow one answers late). no_std.
                   faults/: routines that misbehave on purpose, for testing hosts,
                   not examples to follow — deadlock (two routines each waiting
                   for the other). See Host Checks.
                   effects/: the demo's own effect traits, one module each — count
                   (Count, CountEffect) and lookup (Lookup, LookupEffect) — each
                   with its reifying Ctx impl beside it (the orphan rule puts it
                   there). The routines themselves use only traits.
                   Tests: a Recording mock + testing::run_now — no driver, one poll.

  native/          the routine runs as an ordinary task on an executor. No Driver.
    tokio/         DemoCtx wraps sans-effort-tokio's TokioCtx (Sleep, ReadLine,
                   WriteLine, Spawn as tokio futures and tasks), adds Count and Lookup,
                   and forwards the rest; tokio::spawn(Greeter::new(ctx).run()).
                   Tests: paused clock and multithreaded runs, output captured.
    wasm/          JsCtx implements the traits over a JS object the caller supplies —
                   through a dispatcher task, so its futures are Send; the event loop
                   is the executor. Classes Greeter, Fanout, PingPong, FrontDesk, Ring,
                   Journal, Deadline, and faults::Deadlock.
    js/            a Node host: five functions and one awaited promise. pkg/ is
                   generated.

  driven/          the routine runs behind a Driver; a foreign host replies by id.
    boundary/      the host vocabularies. Full carries every effect the routines use
                   (tags 1–5 and 8–12) and, with the default `table` feature, spawning (tags 6 and 7:
                   split registers the child in sans-effort-host's table); Quiet only
                   Sleep + WriteLine. View/HostEffect/Encode: the tag table.
                   Tests: through a Driver, as data; Greeter under Quiet is a
                   compile_fail; a deterministic Rust router runs ping-pong and front
                   desk across machines.
    cdylib/        the C-ABI binding over sans-effort-host: abi_version/new…/resume/
                   reply/wakes/free/buf_free, plus record/record_finish (a binding
                   export, not ABI); unsafe only at the pointers, no mechanism. The
                   only crate allowing unsafe. Its replay binary replays a recorded
                   run in Rust.
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
nix develop --command demo:faults                                       # host checks (below)
nix develop --command demo:stress                                       # host checks: seeded schedules
```

`--fanout` on any host runs the two-waits-per-batch variant; `--journal` the journal; `--ping-pong`, `--front-desk`, and `--ring` run the spawning routines (`--ring` also prints the host's cost per hop to stderr); `--ticker` on the Python or Java host drives a `Quiet` machine, which can only ever emit tags 4 and 5.

`--deadline` races a receive against a sleep, twice. The quick worker beats a 30-second sleep, so that sleep is abandoned: tokio drops its timer, Node's dispatcher aborts the `AbortSignal` it gave the host's `sleep`, and the driven hosts get a closed frame — Python drops its pending timer (its time is virtual: timers fire only when nothing else can happen), Java cancels the virtual thread sleeping it. A host that fails to cancel waits the 30 seconds out, so `demo` and CI run this mode under a time limit. The slow worker cannot answer until told to, so its 50 ms deadline always passes first.

## Host Checks

`nix develop --command demo:faults` runs routines that misbehave on purpose — `routines::faults` — and checks what each host does about them. They are not part of `demo`, which shows how routines are written; these check the hosts.

- `--deadlock` is two machines each waiting for the other, holding the sender the other waits on: nothing can ever wake either. The driven hosts see it — no call to make, no request outstanding, nothing from `wakes` — print `stalled: 2 machines can never progress` to stderr, and exit 1. Node's event loop runs dry with the routine's promise unsettled, and Node exits 13. tokio has no such detector and would hang, so it is not run there. `demo:faults` and CI check the exit codes, and that the hosts agree.
- `--seed N`, on the Python, Node, and Java hosts, makes the host adversarial, reproducibly: Python takes the next action from a machine chosen at random (each machine's own effects stay in order), defers each reply to a random later turn, and now and then resumes a machine for no reason; Node settles every answer after a random 0–3 ms, so answers asked together settle out of order; Java's seed picks its worker count (1 to 4) and delays each ask by up to 2 ms, and resumes spuriously too. tokio's `--workers K` sets its runtime's worker threads, so its own scheduler is the adversary. `nix develop --command demo:stress [SEEDS]` (default 10) runs every mode on every host under each seed and checks each writes what tokio writes on an ordinary run; CI runs it with 3.
- `--record FILE`, on the Python and Java hosts, saves the run's log through the binding's `greeter_record` and `greeter_record_finish`; `cargo run -p greeter_cdylib --bin replay -- FILE MODE` replays it in Rust and reports the first divergence. A sequential host's runs replay exactly, seeded or not, so `demo:stress` records and replays every seeded Python run.
