# sans-effort

> _sans-io, without all the effort_

Write ordinary, direct-style Rust (`async fn`, `.await`, loops, `?`) and run it two ways:

1. _Natively:_ on `tokio` or the JS event loop it is a plain future at native speed, with no driver.
2. _Driven_ from an FFI host language through a sans-io interface: every wait becomes a typed effect that the host answers by id. The host owns I/O, time, and scheduling — which routine resumes, in what order replies arrive, whether the clock is real or virtual — and the output is typically byte-identical to the native run.

The routine asks for traits, not effects, and cannot tell which way it is running.

This crate is the one dependency a routine author needs. It re-exports the crates underneath:

| Path | From | What |
|---|---|---|
| `sans_effort::{step, join, select, testing}` | `sans-effort-core` | The routine trait `Step`; two waits at once; the first of two; a one-poll test runner |
| `sans_effort::{driver, reply, boundary}` | `sans-effort-core` | The mechanism: `Driver`, `resume`/`reply`, `Yield`, reply handles, the boundary traits |
| `sans_effort::{time, console, spawn, ctx, request}` | `sans-effort-effects` | The standard capabilities (`Sleep`, `ReadLine`, `WriteLine`, `Spawn`) and `Ctx<E>`, the context that turns each into an effect |
| `sans_effort::tokio` | `sans-effort-tokio` | Feature `tokio`: every standard capability as a real tokio future |
| `sans_effort::host` | `sans-effort-host` | Feature `host`: typed and byte-level machines and the handle table, for a foreign-function binding |

The authors of runtimes and bindings can depend on the underlying crates directly; `sans-effort-core` in particular is kept small and slow to change.

## A Routine

A routine names only the capabilities it uses, as trait bounds:

```rust
use core::{ops::ControlFlow, time::Duration};
use sans_effort::{console::WriteLine, step::Step, time::Sleep};

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
```

On tokio, `tokio::spawn(Countdown { ctx: TokioCtx::new(…), left: 3 }.run())`. Behind a driver, `Driver::new(|outbox| Countdown { ctx: Ctx::new(outbox), left: 3 }.run())`, then `resume()` and `reply(handle, value)` until it finishes.

## Features

| Feature | Default | Enables |
|---|---|---|
| `std` | yes | The driver's one lock is `std::sync::Mutex` |
| `spin` | | The lock is `spin::Mutex`, for `no_std` |
| `portable-atomic` | | Atomics for targets without native CAS or 64-bit atomics; implies `spin` |
| `critical-section` | | `portable-atomic` backed by an application-provided `critical-section` |
| `host` | | `sans_effort::host` |
| `tokio` | | `sans_effort::tokio`; implies `std` |

At least one of `std` and `spin` must be enabled somewhere in the build. A library of routines should depend with `default-features = false` and leave the choice to the binary.

## License

MIT or Apache-2.0, at your option.
