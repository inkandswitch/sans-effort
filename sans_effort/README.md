# sans_effort

> Host-driven async coroutines whose every wait is a typed effect

A routine is an ordinary `async fn` whose waits are answered by whoever drives it: no waker, no executor, one `Box::pin` at the boundary. It is a sans-io state machine the compiler writes for you — and because the routine asks for _traits_ rather than effects, the same code is also a plain `async fn` that tokio runs natively with no driver at all.

This crate is the mechanism. It is `no_std` + `alloc`.

```text
  routine     Greeter<C: Sleep + Console>: Run      asks for traits; knows nothing of hosts
  context     impl Sleep for TokioCtx  │ impl Sleep for Ctx<E>
              a real future            │ records an effect, waits for the reply
  host        tokio polls the task     │ a Driver polls; Python, JS, a test… replies by id
```

## What is here

| Item | Role |
|------|------|
| `run::Run` | The shape of a routine: `step` (one loop iteration) and `run` (until it breaks) |
| `driver::Driver` | Turns a routine into something a host can drive: `start()`, then `reply(handle, value)` until finished |
| `driver::outbox::Outbox` | What a reifying context writes into: `tell` an effect, or `ask` and await the reply |
| `reply::ReplyHandle<T>` | The typed, single-use capability to answer one `ask`. Unforgeable; infallible to reply through |
| `request::{Request, Asked}` | Waits as values, so a context is generic over the host's vocabulary and a host can offer a routine less than everything |
| `join::join` | Two waits at once — the reason request ids exist |
| `wire` | The four-kind reply menu, and the traits (`HostEffect`, `Encode`) an effect type implements to cross to a host that cannot hold a Rust value |
| `testing::run_now` | Run a routine against a mock whose every future is ready, in one poll, with no driver |

## A routine, and a Rust host

```rust
use core::ops::ControlFlow;
use sans_effort::{driver::{Driver, outbox::Outbox}, request::{Asked, Request}, run::Run};

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

// A reifying context: each call becomes a request the host answers by id.
struct ReadLine;
struct WriteLine(String);
impl Request for ReadLine { type Reply = String; }

struct Ctx<E>(Outbox<E>);

impl<E: From<Asked<ReadLine>> + From<WriteLine>> Console for Ctx<E> {
    async fn read_line(&self) -> String { self.0.request(ReadLine).await }
    fn write_line(&self, line: String) { self.0.notify(WriteLine(line)); }
}

enum Effect { ReadLine(Asked<ReadLine>), WriteLine(WriteLine) }
impl From<Asked<ReadLine>> for Effect { fn from(a: Asked<ReadLine>) -> Self { Effect::ReadLine(a) } }
impl From<WriteLine> for Effect { fn from(w: WriteLine) -> Self { Effect::WriteLine(w) } }

let mut driver = Driver::<Effect>::new(|outbox| Greeter(Ctx(outbox)).run());
for effect in driver.start() {
    if let Effect::ReadLine(Asked { reply, .. }) = effect {
        for effect in driver.reply(reply, "bob".into()) {
            if let Effect::WriteLine(WriteLine(text)) = effect {
                assert_eq!(text, "Hello, bob!");
            }
        }
    }
}
```

A host with a runtime needs none of this: implement `Console` with real futures and `tokio::spawn` the routine. A host in another language cannot hold a `ReplyHandle`; see `sans_effort_host` and `ABI.md` in the repository.

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
