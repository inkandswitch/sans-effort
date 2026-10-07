# sans-effort-tokio

> Native tokio contexts for `sans-effort` routines

A routine names the effect traits it needs; a context decides what each call does. This crate is the native side: every effect trait in `sans-effort-effects` as a real tokio future, so a routine runs as an ordinary task — `tokio::spawn(routine.run())` — with no driver and no host. Routine authors get it through `sans-effort`'s `tokio` feature.

| Type                   | Implements                                            |
|------------------------|-------------------------------------------------------|
| `clock::TokioClock`    | `Sleep`, `Now`                                        |
| `console::TokioInput`  | `ReadLine`                                            |
| `console::TokioOutput` | `WriteLine`                                           |
| `fs::TokioFs`          | `ReadFile`, `WriteFile`                               |
| `env::TokioEnv`        | `Var`                                                 |
| `random::TokioRandom`  | `Random`                                              |
| `spawn::TokioSpawner`  | `Spawn`, `SpawnPinned`, for the contexts built on it  |
| `ctx::TokioCtx`        | all of the above                                      |

`TokioCtx` is the ready-made context: one value with every effect trait, built from the components. There is one component per effect trait, to compose differently and to grant no more than a routine needs — a clock and output for a routine that never reads, or a paused clock with an in-memory output in a test.

```rust
use sans_effort_effects::console::WriteLine;
use sans_effort_tokio::ctx::TokioCtx;
use tokio_util::task::LocalPoolHandle;

let ctx = TokioCtx::new(&b""[..], Vec::new(), LocalPoolHandle::new(1));
ctx.write_line("hello".into());
let Ok((_, written)) = ctx.into_parts() else { unreachable!("no child shares it") };
assert_eq!(written, b"hello\n");
```

`TokioCtx::new(input, output, pool)` reads and writes whatever it is given; `TokioCtx::stdio(pool)` uses standard input and output. The pool is where pinned children run — a library never starts threads of its own, so the application hands it one. Children spawn as tokio tasks, sharing the parent's input and output.

`ReadLine` here is cancel-safe: a read abandoned mid-line keeps what it had, and the next read returns the whole line. A routine meant for any host must not rely on that (see `ReadLine`'s Cancellation section). `WriteLine` is fire-and-forget, so a failed write has nowhere to go: the first failure is logged with `tracing`, and later lines are dropped.
