# demo

The greeter (prompt, read, look up, pause, greet, count, repeat), written once against five effect traits, then run four ways that must agree byte for byte; and routines that spawn children and talk to them over plain channels (ping-pong, the front desk, a ring, two deadline races). Three of the greeter's traits (`Sleep`, `ReadLine`, `WriteLine`) and the reifying context `Ctx<E>` come from `sans-effort-effects`, as do the spawning routines' `Spawn` and `SpawnPinned`; two (`Count`, `Lookup`) are the demo's own. A journal routine uses the rest of the standard library (`Var`, `ReadFile`, `WriteFile`, `Now`, `Random`), and every host answers it from the same small world (one variable, files in memory, a clock that moves a second per reading, counting random bytes), so its transcripts agree too.

## Layout

Two native runtimes and two foreign hosts, on purpose. tokio and the JS event loop are executors: the routine is spawned on them and its context makes each wait a real future, with no driver. Python and Java cannot poll a Rust future, so there the routine runs behind a `Driver`, and the host replies by request id over the C ABI it decodes from `ABI.md`. The routine cannot tell which it is under.

A JS host _could_ take the Python role (hold a `Machine` in a wasm-bindgen class and step it), and would want to for a deterministic scheduler or a replay harness; the exploration this library came from has one. For running a routine in a page, the native form is the idiomatic one.

| Host   | Directories                        | Path   | Scheduled by                      |
|--------|------------------------------------|--------|-----------------------------------|
| tokio  | `native/tokio/`                    | native | tokio's runtime                   |
| Node   | `native/wasm/`, `native/js/`       | native | the JS event loop                 |
| Python | `driven/cdylib/`, `driven/python/` | driven | a Python loop, through a `Driver` |
| Java   | `driven/cdylib/`, `driven/java/`   | driven | Java threads, through a `Driver`  |

All four run the same routines, from `routines/`. Both driven hosts share `driven/boundary/`, the vocabulary they answer.

### Routines

`routines/` is shared by both paths and is `no_std`. One routine per module:

| Module       | What it does                                                                           |
|--------------|----------------------------------------------------------------------------------------|
| `greeter`    | The conversation                                                                       |
| `fanout`     | Two waits at once                                                                      |
| `ticker`     | Needs only `Sleep` and `WriteLine`                                                     |
| `ping_pong`  | Spawns a child and plays over two `async-channel`s                                     |
| `front_desk` | Spawns a pinned clerk per name; each replies on a one-shot channel                     |
| `ring`       | 16 routines pass a counter 4,000 hops, which measures the cost of a hop                |
| `journal`    | Appends entries stamped with the time and a random id to a file the environment names  |
| `deadline`   | Two workers, each raced against a sleep: the quick one wins, the slow one answers late |

`faults/` holds routines that misbehave on purpose. They test hosts and are not examples to follow. Its one fault, `deadlock`, is two routines each waiting for the other (see [Host Checks](#host-checks)).

`effects/` holds the demo's own effect traits, one module each: `count` (`Count`, `CountEffect`) and `lookup` (`Lookup`, `LookupEffect`). Each trait's reifying `Ctx` impl is beside it, because the orphan rule puts it there. The routines themselves name only traits.

The tests run the routines against a `Recording` mock with `testing::run_now`: no driver, one poll. A `trybuild` test checks that a context whose futures are not `Send` cannot implement an effect trait.

### Native Hosts

In `native/`, the routine runs as an ordinary task on an executor, with no `Driver`.

`tokio/` runs the routine with `tokio::spawn(Greeter::new(ctx).run())`. Its `DemoCtx` wraps `sans-effort-tokio`'s `TokioCtx` and forwards `Sleep`, `ReadLine`, `WriteLine`, `Spawn`, and `SpawnPinned` to it. It adds `Count` and `Lookup`, and answers the journal's effects from the same small world as the other hosts. Its tests use a paused clock and multithreaded runs, with output captured.

`wasm/` is the routine compiled for wasm-bindgen. Its `JsCtx` implements the effect traits over a JS object the caller supplies, through a dispatcher task, so its futures are `Send`. The event loop is the executor. It exports the classes `Greeter`, `Fanout`, `PingPong`, `FrontDesk`, `Ring`, `Journal`, `Deadline`, and `faults::Deadlock`.

`js/` is the Node host: a host object with one function per effect, and one awaited promise. Its `pkg/` is generated.

### Driven Hosts

In `driven/`, the routine runs behind a `Driver`, and a foreign host replies by request id.

`boundary/` holds the host vocabularies:

| Vocabulary | Carries                                                                                                           |
|------------|-------------------------------------------------------------------------------------------------------------------|
| `Full`     | Every effect the routines use (tags 1–5 and 8–12), and, with the default `table` feature, spawning (tags 6 and 7) |
| `Quiet`    | Only `Sleep` and `WriteLine`                                                                                      |
| `View`     | What a host sees of either; with `HostEffect` and `Encode`, the tag table                                         |

When `Full` splits a spawn, it registers the child in `sans-effort-host`'s table. The crate's tests drive the greeter through a `Driver`, as data, and through a `Machine` with raw reply bytes. They run every multi-machine routine under `testing::drive_with`'s schedules, and replay recorded runs. A `compile_fail` doctest checks that `Greeter` cannot run under `Quiet`.

`cdylib/` is the C-ABI binding over `sans-effort-host`. It exports `abi_version`, a `new…` constructor per routine, `resume`, `reply`, `wakes`, `free`, and `buf_free`, each prefixed `greeter_`. It also exports `record` and `record_finish`, which are the binding's own, not ABI. It uses `unsafe` only at the pointers and holds no mechanism; it is the only crate that allows `unsafe`. Its `replay` binary replays a recorded run in Rust.

`python/` is a `ctypes` host that speaks `ABI.md` with a byte buffer and no library. It performs each effect, then replies. It resumes spawned children and each machine a woke frame names, and asks `wakes()` when nothing is queued.

`java/` is a Panama (`java.lang.foreign`) host: the same ABI, downcalls only, no JNI. It is parallel: a pool of driver threads polls different machines at once. Migrating machines move between the threads; pinned ones stay on the worker that first resumed them. Asks run on virtual threads, woke frames schedule resumes, and closed frames cancel. `--trace` logs which thread polled what.

## Running It

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio               # native: tokio
nix develop --command demo:wasm && node demo/native/js/main.mjs         # native: wasm-bindgen
cargo build -p greeter_cdylib && python3 demo/driven/python/main.py     # driven: C ABI, ctypes
java --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java     # driven: C ABI, Panama
nix develop --command demo                                              # all four, diffed
nix develop --command demo:faults                                       # host checks (below)
nix develop --command demo:stress                                       # host checks: seeded schedules
```

`--fanout` on any host runs the two-waits-per-batch variant; `--journal` the journal; `--ping-pong`, `--front-desk`, and `--ring` run the spawning routines (`--ring` also prints the host's cost per hop to stderr: one cold run, over the debug build the demo uses, so a JIT-compiled host such as Java's is mostly warming up: warm, over a release build, the Java host takes about 1.2 µs per hop, of which the library is about 0.4 µs); `--ticker` on the Python or Java host drives a `Quiet` machine, which can only ever emit tags 4 and 5.

`--deadline` races a receive against a sleep, twice. The quick worker beats a 30-second sleep, so that sleep is abandoned: tokio drops its timer, Node's dispatcher aborts the `AbortSignal` it gave the host's `sleep`, and the driven hosts get a closed frame: Python drops its pending timer (its time is virtual: timers fire only when nothing else can happen), Java cancels the virtual thread sleeping it. A host that fails to cancel waits the 30 seconds out, so `demo` and CI run this mode under a time limit. The slow worker cannot answer until told to, so its 50 ms deadline always passes first.

## Host Checks

`nix develop --command demo:faults` runs routines that misbehave on purpose (`routines::faults`) and checks what each host does about them. They are not part of `demo`, which shows how routines are written; these check the hosts.

- `--deadlock` is two machines each waiting for the other, holding the sender the other waits on: nothing can ever wake either. The driven hosts see it (no call to make, no request outstanding, nothing from `wakes`), print `stalled: 2 machines can never progress` to stderr, and exit 1. Node's event loop runs dry with the routine's promise unsettled, and Node exits 13. tokio has no such detector and would hang, so it is not run there. `demo:faults` and CI check the exit codes, and that the hosts agree.
- `--seed N`, on the Python, Node, and Java hosts, makes the host adversarial, reproducibly: Python takes the next action from a machine chosen at random (each machine's own effects stay in order), defers each reply to a random later turn, and now and then resumes a machine for no reason; Node settles every answer after a random 0–3 ms, so answers asked together settle out of order; Java's seed picks its worker count (1 to 4) and delays each ask by up to 2 ms, and resumes spuriously too. tokio's `--workers K` sets its runtime's worker threads, so its own scheduler is the adversary. `nix develop --command demo:stress [SEEDS]` (default 10) runs every mode on every host under each seed and checks each writes what tokio writes on an ordinary run; CI runs it with 3.
- `--record FILE`, on the Python and Java hosts, saves the run's log through the binding's `greeter_record` and `greeter_record_finish`; `cargo run -p greeter_cdylib --bin replay -- FILE MODE` replays it in Rust and reports the first divergence. A sequential host's runs replay exactly, seeded or not, so `demo:stress` records and replays every seeded Python run.
