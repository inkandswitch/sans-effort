# sans-effort

> _sans-io, without all the effort_

Write ordinary, direct-style Rust (`async fn`, `.await`, loops, `?`) and run it two ways:

1. _Natively:_ on `tokio` or the JS event loop it is a plain future at native speed, with no driver.
2. _Driven_ from an FFI host language through a sans-io interface: every wait becomes a typed effect that the host answers by id. The host owns I/O, time, and scheduling — which routine resumes, in what order replies arrive, whether the clock is real or virtual — and the output is typically byte-identical to the native run.

The routine asks for traits, not effects, and cannot tell which way it is running. Routines may spawn children and talk to them over ordinary channels; the host schedules them all.

This crate is the one dependency a routine author needs. It re-exports the crates underneath:

| Path | From | What |
|---|---|---|
| `sans_effort::{step, join, select}` | `sans-effort-core` | The routine trait `Step`; two waits at once; the first of two |
| `sans_effort::testing` | `sans-effort-core` | Feature `testing`: a one-poll test runner, and `drive` — a routine and its children through drivers, under a schedule |
| `sans_effort::{driver, reply, boundary}` | `sans-effort-core` | The mechanism: `Driver` (`new`, or `local` for a routine that is not `Send`), `resume`/`reply`, `Yield`, `Status` (`Awaiting`, `Idle`, `Complete`) and `on_wake`, reply handles, the boundary traits |
| `sans_effort::{ask, console, ctx, env, fs, random, spawn, time}` | `sans-effort-effects` | The standard effect traits (`Sleep`, `Now`, `ReadLine`, `WriteLine`, `ReadFile`, `WriteFile`, `Var`, `Random`, `Spawn`, `SpawnPinned`) and `Ctx<E>`, the context that turns each into an effect |
| `sans_effort::guide` | `sans-effort-effects` | Writing an effect trait of your own, from the trait to the bytes a host reads |
| `sans_effort::tokio` | `sans-effort-tokio` | Feature `tokio`: every standard effect trait as a real tokio future |
| `sans_effort::host` | `sans-effort-host` | Feature `host`: typed and byte-level machines and the handle table, for a foreign-function binding |

The core and the standard effect traits share one flat namespace: together they are the vocabulary a routine is written in. A runtime or a binding kit is opted into, and keeps its own module. The authors of runtimes and bindings can depend on the underlying crates directly; `sans-effort-core` in particular is kept small and slow to change.

## A Routine

A routine names only the effect traits it uses, as trait bounds. On tokio it is an ordinary task: `tokio::spawn(Countdown { ctx: TokioCtx::new(…), left: 3 }.run())`.

Behind a driver, a host decides what each effect means. It names the effects it offers in a _vocabulary_, an enum with a `From` impl for each: `Asked<…Effect>` for an effect that waits for an answer, the effect itself for one that does not. `Ctx<E>` then implements every trait whose effect `E` carries. The host resumes the driver, performs each effect it yields, and replies by handle until the routine completes:

```rust
use core::{ops::ControlFlow, time::Duration};
use sans_effort::{
    ask::Asked,
    console::{WriteLine, WriteLineEffect},
    ctx::Ctx,
    driver::{Driver, status::Status},
    step::Step,
    time::{Sleep, SleepEffect},
};
use std::collections::VecDeque;

// The routine.
struct Countdown<C> {
    ctx: C,
    left: u32,
}

impl<C: Sleep + WriteLine> Step for Countdown<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.left == 0 {
            return ControlFlow::Break(());
        }
        self.ctx.write_line(format!("{}", self.left));
        self.ctx.sleep(Duration::from_millis(10)).await;
        self.left -= 1;
        ControlFlow::Continue(())
    }
}

// A host's vocabulary: the effects it offers.
enum Effect {
    Sleep(Asked<SleepEffect>),
    WriteLine(WriteLineEffect),
}

impl From<Asked<SleepEffect>> for Effect {
    fn from(asked: Asked<SleepEffect>) -> Self {
        Effect::Sleep(asked)
    }
}

impl From<WriteLineEffect> for Effect {
    fn from(write: WriteLineEffect) -> Self {
        Effect::WriteLine(write)
    }
}

// The host: perform each effect, reply by handle, until it completes.
let mut driver = Driver::<Effect>::new(|outbox| Countdown { ctx: Ctx::new(outbox), left: 3 }.run());
let mut written = Vec::new();
let mut queue: VecDeque<Effect> = driver.resume().into();
while let Some(effect) = queue.pop_front() {
    match effect {
        Effect::WriteLine(WriteLineEffect(line)) => written.push(line),
        // A virtual clock: answer at once rather than wait.
        Effect::Sleep(Asked { reply, .. }) => queue.extend(driver.reply(reply, ())),
    }
}
assert_eq!(written, ["3", "2", "1"]);
assert_eq!(driver.status(), Status::Complete);
```

A host in another language does the same through `sans_effort::host` (the `host` feature), with request ids instead of handles; `ABI.md` in the repository describes it.

## Features

| Feature | Default | Enables |
|---|---|---|
| `std` | yes | The driver's one lock is `std::sync::Mutex` |
| `spin` | | The lock is `spin::Mutex`, for `no_std` |
| `portable-atomic` | | Atomics for targets without native CAS or 64-bit atomics; implies `spin` |
| `critical-section` | | `portable-atomic` backed by an application-provided `critical-section` |
| `host` | | `sans_effort::host` |
| `testing` | | `sans_effort::testing`; enable it in `[dev-dependencies]` |
| `tokio` | | `sans_effort::tokio`; implies `std`; native targets only |

At least one of `std` and `spin` must be enabled somewhere in the build. A library of routines should leave that choice to the binary that runs them, yet still build on its own. Depend with `default-features = false`, and forward a default `std` feature, which a `no_std` binary turns off:

```text
[dependencies]
sans-effort = { version = "0.1", default-features = false }

[features]
default = ["std"]
std = ["sans-effort/std"]
```

## License

MIT or Apache-2.0, at your option.
