# sans-effort-core

> The mechanism under `sans-effort`: the routine trait, the driver, and the boundary

_Writing routines? Depend on [`sans-effort`](https://crates.io/crates/sans-effort) instead._ It re-exports this crate's modules at the same paths (`sans_effort::driver`, `sans_effort::step`, …), with the standard effect traits beside them. This crate is for the authors of runtimes and bindings, and is kept small and slow to change. It is `no_std` + `alloc`.

_sans-io, without all the effort._ Write an ordinary `async fn` — loops, `?`, `.await` — and run it two ways:

1. Call it from Rust as normal: on tokio it is a plain future at native speed, with no driver.
2. Drive it from an FFI host language through a sans-io interface: every wait becomes a typed effect that the host answers by id, so the host owns I/O, time, and scheduling — which routine resumes, in what order replies arrive, whether the clock is real or virtual — and the output is typically byte-identical to the native run.

An _effect routine_ — "routine" from here on — is such an `async fn`. It asks for traits, not effects, and cannot tell which way it is running. The compiler writes the state machine; the only `Pin` is one `Box::pin` at an FFI boundary, if there is one.

```text
  host                                    routine
    │                                         │
    │  resume()                               │
    │────────────────────────────────────────▶│ run until input needed
    │                                         │
    │        [WriteLine, (ReadLine, 1)]       │
    │◀────────────────────────────────────────┊
    │                                         ┊ AWAITING
    │             reply(1, "bob")             ┊
    │────────────────────────────────────────▶┊
    │                                         │
    │              [(Lookup, 2)]              │
    │◀────────────────────────────────────────┊
    │                                         ┊ AWAITING
    │            reply(2, "Hello")            ┊
    │────────────────────────────────────────▶┊
    │                                         │
    │  [WriteLine]                            │
    │◀────────────────────────────────────────│ COMPLETE
    │                                         ┴
```

## Three Layers

The _host_ is whoever polls: tokio, or an FFI host language over a `Driver`. A _context_ implements the routine's traits and decides what each call does: a real future on a runtime, or an effect recorded for a host. The _routine_ asks for traits and never sees an effect.

```text
  ┌───────────────────────────────────────┬───────────────────────────┐
  │ host        tokio or the JS event     │  a Driver polls; Python,  │
  │             loop polls — no driver    │  Java, a test… performs   │
  ├───────────────────────────────────────┼───────────────────────────┤
  │ context     impl Sleep for TokioCtx   │  impl Sleep for Ctx<E>    │
  │             a real future             │  records an effect, waits │
  ├───────────────────────────────────────┴───────────────────────────┤
  │ routine     Greeter<C: Sleep + Lookup + Console>: Step             │  no_std
  │             owns the logic; knows nothing of effects or hosts     │
  └───────────────────────────────────────────────────────────────────┘
```

The routine is the foundation and depends on nothing above it. The native path is why you write it this way: the routine is also a plain `async fn`, usable at native speed by code that has never heard of this crate. The driven path is what this crate provides.

## The Pieces

| Item | Role |
|------|------|
| `step::Step` | The shape of a routine: `step` (one iteration of its loop) and `run` (until it breaks) |
| `driver::outbox::Outbox` | What a reifying context writes into: `tell` an effect and move on, or `ask` one and await the reply |
| `reply::handle::ReplyHandle` | The typed, single-use capability to answer one `ask`. It accepts the `Answer` the routine waits for (`String`, `Result<String, ReadLineError>`, …), which crosses as one of four sealed wire kinds (`str`, `u64`, `unit`, `bytes`); a wire value that does not decode as the answer is refused |
| `driver::Driver` | Turns a routine into something a host can step: `resume()` to begin, then `reply(handle, value)` until it is finished. Each returns a `Yield`: the effects, and the ids of requests the routine abandoned. `Driver::new` takes a `Send` routine; `Driver::local` one that is not, which stays on its thread |
| `driver::status::Status` | Where the routine stopped: `Awaiting` a reply, `Complete`, or `Idle` — waiting on something inside the process, such as a channel another routine sends on. `on_wake` sets the hook the driver calls when that wait may be over, once per wait |
| `join::join`, `select::select` | Two waits at once, or the first of two — the reason request ids exist. A `select`'s loser is abandoned and its id reported closed |
| `boundary` | What an effect type implements to cross to a host that cannot hold a Rust value: `HostEffect` (handles → ids), `Encode` (the codec), `Pending` |
| `testing` (feature `testing`) | `run_now` runs a routine against a mock whose every future is ready, in one poll; `drive`/`drive_with` run it and everything it spawns through drivers, in an order a schedule picks, on a virtual clock, optionally killing machines |

## A Routine, a Reifying Context, and a Rust Host

The routine names what it needs as traits. Nothing from this crate appears in it but `Step`:

```rust
use core::ops::ControlFlow;
use sans_effort_core::{
    driver::{Driver, outbox::Outbox, status::Status},
    reply::handle::ReplyHandle,
    step::Step,
};
use std::collections::VecDeque;

trait Console {
    async fn read_line(&self) -> String;
    fn write_line(&self, line: String);
}

struct Greeter<C: Console> {
    ctx: C,
}

impl<C: Console> Step for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write_line("Who are you?".into());
        let name = self.ctx.read_line().await;
        self.ctx.write_line(format!("Hello, {name}!"));
        ControlFlow::Break(())
    }
}

// The host's vocabulary: an effect per wait, carrying the handle that
// answers it, and one per message.
enum Effect {
    ReadLine(ReplyHandle<String>),
    WriteLine(String),
}

// A reifying context: `ask` builds an effect around a fresh handle and
// awaits the reply; `tell` records one and moves on.
struct Ctx(Outbox<Effect>);

impl Console for Ctx {
    async fn read_line(&self) -> String {
        self.0.ask(Effect::ReadLine).await
    }

    fn write_line(&self, line: String) {
        self.0.tell(Effect::WriteLine(line));
    }
}

// A Rust host: match on the effects, reply through the handles.
let mut driver = Driver::new(|outbox| Greeter { ctx: Ctx(outbox) }.run());
let mut queue: VecDeque<Effect> = driver.resume().into();
let mut written = Vec::new();

while let Some(effect) = queue.pop_front() {
    match effect {
        Effect::WriteLine(text) => written.push(text),
        Effect::ReadLine(reply) => queue.extend(driver.reply(reply, "bob".into())),
    }
}

assert_eq!(driver.status(), Status::Complete);
assert_eq!(written, ["Who are you?", "Hello, bob!"]);
```

This context names the variants of one enum, so it serves one vocabulary. A context generic over _any_ host's vocabulary — and a standard library of effect traits built on one — is `sans-effort-effects`. A host with a runtime needs none of this: implement `Console` with real futures and `tokio::spawn` the routine. An FFI host cannot hold a `ReplyHandle`; see `sans-effort-host`, and `ABI.md` in the repository.

## Features

| Feature | Effect |
|---------|--------|
| `std` (default) | The driver's one lock is `std::sync::Mutex` |
| `spin` | The lock is `spin::Mutex`, for `no_std`; needs CAS atomics unless… |
| `portable-atomic` | …this supplies them, for targets without native CAS or 64-bit atomics. Implies `spin` |
| `critical-section` | `portable-atomic` backed by an application-provided `critical-section` |
| `testing` | The `testing` module; enable it in `[dev-dependencies]` |

At least one of `std` and `spin` must be enabled; the crate refuses to build otherwise, with a message saying so.
