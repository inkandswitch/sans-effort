# sans-effort-effects

> A standard library of effect traits for `sans-effort` routines

A routine names what it needs as traits; a context decides what each call does. Most routines want the same few things — to sleep, to read and write lines — and without this crate every application writes each of them three times: the trait, the effect and its reifying impl, and a native impl. Here the first two are written once. Routine authors get all of this through `sans-effort`, at the same paths.

| Module    | Trait         | Answer                          | If abandoned (a `select`'s loser)                 |
|-----------|---------------|---------------------------------|---------------------------------------------------|
| `time`    | `Sleep`       | `()`                            | Retractable: nothing is lost                      |
| `time`    | `Now`         | `UnixTime`                      | Retractable                                       |
| `console` | `ReadLine`    | `Result<String, ReadLineError>` | Consuming, _not cancel-safe_: the line may be lost |
| `console` | `WriteLine`   | none: told, not asked           | —                                                 |
| `fs`      | `ReadFile`    | `Result<Vec<u8>, FsError>`      | Retractable                                       |
| `fs`      | `WriteFile`   | `Result<(), FsError>`           | Committing: the write may still happen            |
| `env`     | `Var`         | `Option<String>`                | Retractable                                       |
| `random`  | `Random`      | `Vec<u8>`                       | Consuming, but fresh bytes serve as well          |
| `spawn`   | `Spawn`, `SpawnPinned` | none: told, not asked  | —                                                 |

Each module holds the trait, its effect struct (the trait's name plus `Effect`: `SleepEffect`), and the trait's impl for `Ctx` — the reifying context, which records each call as an effect for a host to perform. `Ctx` is generic over the host's vocabulary `E`, so one impl serves every application: an application writes its vocabulary enum, with a `From` impl per effect it offers, and `Ctx<E>` implements exactly the traits that vocabulary can carry. A host can offer a routine _less_ than everything, and the compiler holds it to that. Underneath is `ask`: `Ask` and `Asked`, the pattern that makes a context generic over the enum.

```rust
use core::ops::ControlFlow;
use sans_effort_core::{
    driver::{Driver, status::Status},
    step::Step,
};
use sans_effort_effects::{
    ask::Asked,
    console::{ReadLine, ReadLineEffect, ReadLineError, WriteLine, WriteLineEffect},
    ctx::Ctx,
};
use std::collections::VecDeque;

struct Echo<C>(C);

impl<C: ReadLine + WriteLine> Step for Echo<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        match self.0.read_line().await {
            Ok(line) => self.0.write_line(line),
            Err(ReadLineError::Closed) => return ControlFlow::Break(()),
            Err(_) => self.0.write_line("?".into()),
        }
        ControlFlow::Continue(())
    }
}

// The host's vocabulary: which effects it offers.
enum Effect {
    ReadLine(Asked<ReadLineEffect>),
    WriteLine(WriteLineEffect),
}

impl From<Asked<ReadLineEffect>> for Effect {
    fn from(asked: Asked<ReadLineEffect>) -> Self {
        Effect::ReadLine(asked)
    }
}

impl From<WriteLineEffect> for Effect {
    fn from(write: WriteLineEffect) -> Self {
        Effect::WriteLine(write)
    }
}

let mut driver = Driver::<Effect>::new(|outbox| Echo(Ctx::new(outbox)).run());
let mut input = vec![Ok("hi")].into_iter();
let mut written = Vec::new();
let mut queue: VecDeque<Effect> = driver.resume().into();

while let Some(effect) = queue.pop_front() {
    match effect {
        Effect::WriteLine(WriteLineEffect(line)) => written.push(line),
        Effect::ReadLine(Asked { reply, .. }) => {
            let line = input.next().unwrap_or(Err(ReadLineError::Closed));
            queue.extend(driver.reply(reply, line.map(String::from)));
        }
    }
}

assert_eq!(written, ["hi"]);
assert_eq!(driver.status(), Status::Complete);
```

The native side of every trait is `sans-effort-tokio`. This crate is `no_std` + `alloc`; its `std` and `spin` features only forward `sans-effort-core`'s choice of lock, so a crate that names the traits and nothing else should depend with `default-features = false`.
