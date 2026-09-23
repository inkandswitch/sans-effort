# sans-effort

> _sans-io, without all the effort_

Write ordinary, direct-style Rust (`async fn`, `.await`, loops, `?`) and run it two ways:

1. Call it from Rust as normal: on `tokio` it is a plain future at native speed, with no driver.
2. Drive it from an FFI host language through a sans-io interface: every wait becomes a typed effect that the host answers by id, so the host owns I/O, time, and scheduling — which routine resumes, in what order replies arrive, whether the clock is real or virtual — and the output is typically byte-identical to the native run.

The routine asks for traits, not effects, and cannot tell which way it is running. The compiler writes the state machine; the only `Pin` is one `Box::pin` at an FFI boundary, if and when there is one.

This crate is the mechanism. It is `no_std` + `alloc`.

```text
  ┌───────────────────────────────────────┬───────────────────────────┐
  │ host        tokio or the JS event     │  a Driver polls; Python,  │
  │             loop polls — no driver    │  Java, a test… performs   │
  ├───────────────────────────────────────┼───────────────────────────┤
  │ context     impl Sleep for TokioCtx   │  impl Sleep for Ctx<E>    │
  │             a real future             │  records an effect, waits │
  ├───────────────────────────────────────┴───────────────────────────┤
  │ routine     Greeter<C: Sleep + Lookup + Console>: Run             │  no_std
  │             owns the logic; knows nothing of effects or hosts     │
  └───────────────────────────────────────────────────────────────────┘
```

## What is here

| Item | Role |
|------|------|
| `run::Run` | The shape of a routine: `step` (one loop iteration) and `run` (until it breaks) |
| `driver::Driver` | Turns a routine into something a host can drive: `start()`, then `reply(handle, value)` until finished. Each returns a `Step`: the effects, and the ids of requests abandoned along the way |
| `driver::outbox::Outbox` | What a reifying context writes into: `tell` an effect, or `ask` and await the reply |
| `reply::{ReplyHandle<T>, Reply}` | The typed, single-use capability to answer one `ask` — unforgeable, infallible to reply through — and the sealed four-kind menu (`str`, `u64`, `unit`, `bytes`) it accepts |
| `join::join` | Two waits at once — the reason request ids exist |
| `select::select` | The first of two waits; the other is abandoned, and its id is reported closed |
| `boundary` | What an effect type implements to cross to a host that cannot hold a Rust value: `HostEffect` (handles → ids), `Encode` (the codec), `Pending` |
| `testing::run_now` | Run a routine against a mock whose every future is ready, in one poll, with no driver |

## A routine, and a Rust host

```rust
use core::ops::ControlFlow;
use sans_effort::{driver::{Driver, outbox::Outbox}, reply::handle::ReplyHandle, run::Run};

trait Console {
    async fn read_line(&self) -> String;
    fn write_line(&self, line: String);
}

struct Greeter<C: Console>(C);

impl<C: Console> Run for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        let name = self.0.read_line().await;
        self.0.write_line(format!("Hello, {name}!"));
        ControlFlow::Break(())
    }
}

// A reifying context: each call becomes an effect the host answers through its handle.
enum Effect { ReadLine(ReplyHandle<String>), WriteLine(String) }

struct Ctx(Outbox<Effect>);

impl Console for Ctx {
    async fn read_line(&self) -> String { self.0.ask(Effect::ReadLine).await }
    fn write_line(&self, line: String) { self.0.tell(Effect::WriteLine(line)); }
}

let mut driver = Driver::new(|outbox| Greeter(Ctx(outbox)).run());
for effect in driver.start() {
    if let Effect::ReadLine(reply) = effect {
        for effect in driver.reply(reply, "bob".into()) {
            if let Effect::WriteLine(text) = effect {
                assert_eq!(text, "Hello, bob!");
            }
        }
    }
}
```

This context names the variants of one enum, so it serves one vocabulary. For a context generic over _any_ host's vocabulary — and a standard library of capabilities built on one — see `sans-effort-effects`. A host with a runtime needs none of this: implement `Console` with real futures and `tokio::spawn` the routine. An FFI host cannot hold a `ReplyHandle`; see `sans-effort-host` and `ABI.md` in the repository.

## Features

| Feature | Effect |
|---------|--------|
| `std` (default) | The driver's one lock is `std::sync::Mutex` |
| `spin` | The lock is `spin::Mutex`, for `no_std`; needs CAS atomics unless… |
| `portable-atomic` | …this supplies them, for targets without native CAS or 64-bit atomics. Implies `spin` |
| `critical-section` | `portable-atomic` backed by an application-provided `critical-section` |

At least one of `std` and `spin` must be enabled; the crate refuses to build otherwise, with a message saying so.

## License

MIT or Apache-2.0, at your option.
