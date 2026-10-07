# sans-effort

> _sans-io, without all the effort_

[![CI](https://github.com/inkandswitch/sans-effort/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/sans-effort/actions/workflows/test-host.yml) [![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT) [![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/sans-effort)

`sans-effort` lets you write ordinary, "direct-style" Rust (`async fn`, `.await`, loops, `?`, etc) and run it two ways:

1. Call it from Rust as normal: on `tokio` it is a plain future at native speed, with no driver.
2. Drive it from a FFI host language through a sans-io interface: every wait becomes a typed effect that the host answers by id, so the host owns I/O, time, and scheduling — which routine resumes, in what order replies arrive, whether the clock is real or virtual — and the output is typically byte-identical to the native run.

The routine asks for traits, not effects, and cannot tell which way it is running. It may spawn children and talk to them over ordinary channels; whichever host runs them is the scheduler. The compiler writes the state machine; the only `Pin` is one `Box::pin` at any FFI boundary (if and when it exists).

This repo contains the library. The research exploration including alternatives built out and measured live in [`effect-routines-exploration`][exploration]:

[exploration]: https://tangled.org/expede.wtf/effect-routines-exploration
[guide]: https://tangled.org/expede.wtf/effect-routines-exploration/raw/main/guide/how-to-write-effect-routines.pdf
[analysis]: https://tangled.org/expede.wtf/effect-routines-exploration/blob/main/analysis/README.md
[compare]: https://tangled.org/expede.wtf/effect-routines-exploration/tree/main/compare
[bench]: https://tangled.org/expede.wtf/effect-routines-exploration/tree/main/hosts/bench

## Three Layers

```mermaid
flowchart TB
    subgraph host["host"]
        direction LR
        native["tokio · the JS event loop<br/>polls the task directly, no driver"]
        foreign["Python · Java · a test<br/>a Driver polls; the host performs effects and replies by id"]
    end

    subgraph context["context"]
        direction LR
        tokio_ctx["impl Sleep for TokioCtx<br/>each call is a real future"]
        reify_ctx["impl Sleep for Ctx#60;E#62;<br/>each call records an effect and suspends"]
    end

    subgraph routine["routine · no_std"]
        greeter["Greeter#60;C: Sleep + Lookup + ReadLine + WriteLine#62;: Step<br/>owns the logic; asks for traits; knows nothing of hosts"]
    end

    native --> tokio_ctx --> greeter
    foreign --> reify_ctx --> greeter
```

The routine is the foundation and depends on nothing above it. The native path is why you write it this way: the routine is also an ordinary library function. The driven path is what these crates provide. [`design/concepts.md`](design/concepts.md) names every piece and how they connect.

## The Routine

```rust
use sans_effort::{console::{ReadLine, WriteLine}, time::Sleep};  // the standard library

pub trait Lookup {                                                       // the application's own
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send;
}

impl<C: Sleep + Lookup + ReadLine + WriteLine> Step for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write_line("Who are you?".into());
        let Ok(name) = self.ctx.read_line().await else {                  // input can end
            return ControlFlow::Break(());
        };
        let greeting = self.ctx.lookup(name.clone()).await;
        self.ctx.sleep(PAUSE).await;
        self.ctx.write_line(format!("{greeting}, {name}!"));
        ControlFlow::Continue(())
    }
}
```

## Two Contexts, One Routine

Natively, on tokio — no driver, no effects, tokio polls the task:

```rust
// in sans-effort-tokio: one component per trait, composed into TokioCtx
impl Sleep for TokioClock {
    fn sleep(&self, d: Duration) -> impl Future<Output = ()> + Send { tokio::time::sleep(d) }
}
// …
let pool = LocalPoolHandle::new(1);  // where pinned children run: the application's, not the library's
tokio::spawn(Greeter::new(DemoCtx::new(TokioCtx::stdio(pool))).run());  // DemoCtx adds Lookup
```

Behind a host — each call records a request and suspends until the host replies by id. The context is generic over the host's vocabulary `E`, so a host can offer a routine _less_ than everything, and the type system enforces it:

```rust
// in sans-effort-effects, written once for every application (simplified):
impl<E: From<Asked<SleepEffect>>> Sleep for Ctx<E> {
    async fn sleep(&self, d: Duration) { self.ask(SleepEffect(d)).await }
}
// …
Driver::<Full>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());   // ok
Driver::<Quiet>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());  // E0277: Ctx<Quiet>: Lookup
Driver::<Quiet>::new(|outbox| Ticker::new(Ctx::new(outbox), 3).run()); // ok: Ticker needs only Sleep + WriteLine
```

`demo/` has all of this in full: routines from a greeter to a ring of machines passing a token, both contexts, native runtimes on tokio and on Node (wasm-bindgen), and a C-ABI binding driven from Python (ctypes) and Java (Panama, with a pool of driver threads). `nix develop` then `demo` runs every routine on all four and checks the transcripts are byte-identical.

### Where This Sits

This is the [tagless-final][tf] style with the representation pinned to `impl Future`: the traits are the algebra, a routine is a term abstract in its interpreter, `TokioCtx` is the evaluating interpreter, and `Ctx<E>` is the reifying one — the instance that recovers the initial, tagged encoding (`Full`) from the final one. `Quiet` is a smaller algebra. Rust readers may know the same shape as "capability traits" or MTL-style; the effects style, where the routine emits a tagged enum directly, is the initial encoding, and the two share one mechanism because initial and final encodings are interconvertible. Rust has no higher-kinded types, so the representation cannot vary; the interpreter does.

[tf]: https://okmij.org/ftp/tagless-final/index.html

## How a Host Drives a Routine

```text
  host                                    routine
    │                                         │
    │  resume()                               │
    │────────────────────────────────────────▶│ run until input needed
    │                                         │ 
    │        [WriteLine, (ReadLine, 1)]       │
    │◀────────────────────────────────────────🮘
    │                                         🮘 AWAITING
    │             reply(1, "bob")             🮘
    │────────────────────────────────────────▶🮘 
    │                                         │
    │              [(Lookup, 2)]              │
    │◀────────────────────────────────────────🮘 
    │                                         🮘 AWAITING
    │             reply(2, "Hello")           🮘
    │────────────────────────────────────────▶🮘 
    │                                         │
    │  [WriteLine]                            │
    │◀────────────────────────────────────────│ COMPLETE
    │                                         ┴
```

- _Pull-only._ The routine asks for everything it needs, but by _returning_ an effect from `resume`/`reply`, never by calling the host. No callbacks, no upcalls, so no foreign value ever enters a Rust frame — which is why the routine is `Send` for free.
- _Typed replies._ Every awaiting effect carries a `ReplyHandle<T>`: the typed, single-use capability to answer it. A Rust host replies through the handle, infallibly; a foreign host replies by id, and the host layer checks the kind.
- _Many waits outstanding._ Requests carry ids, so a routine may `join` two waits — or `select` the first of them — and a host may reply in any order. A request the routine abandons is reported to the host, which may stop the work.
- _Many routines._ A routine may spawn children; each is a machine of its own, which the host starts and steps like the first. They talk over ordinary channels, inside the process: a routine waiting on one is `IDLE`, and when another machine's step may have unblocked it, that call's output names it in a `woke` frame, so the host knows whom to resume. A host can tell a finished program from a deadlock.
- _`no_std` core._ The mechanism, the routine, and its boundary crate all build for `wasm32`; the mechanism for `thumbv6m` with `critical-section`.

## Crates

| Crate                                         | Purpose                                                                                                                                                                                          | Target             |
|-----------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------|
| [`sans-effort`](sans-effort/)                 | The one dependency for routine authors: re-exports `-core` and `-effects` in one flat namespace; `tokio` and `host` behind features                                                               |
| [`sans-effort-core`](sans-effort-core/)       | The mechanism: `Step`, `Outbox`, `ReplyHandle`, `Answer`, `Driver`, `join`, `select`, the reply menu, `boundary`, `testing`                                                                      | `no_std` + `alloc` |
| [`sans-effort-effects`](sans-effort-effects/) | A standard library of effect traits — `time`, `console`, `fs`, `env`, `random`, `spawn` — the traits, their effects, and the reifying `Ctx<E>`, written once | `no_std` + `alloc` |
| [`sans-effort-tokio`](sans-effort-tokio/)     | Native tokio contexts: one component per effect trait — `TokioClock`, `TokioInput`, `TokioOutput`, `TokioFs`, `TokioEnv`, `TokioRandom`, `TokioSpawner` — and `TokioCtx` with all of them — real futures, no driver | `std`              |
| [`sans-effort-host`](sans-effort-host/)       | The host side for foreign hosts: a typed `Machine`, the `Encoded` byte layer, a handle table, panic isolation, and record/replay of a foreign host's run. No `unsafe`                                  | `std`              |
| [`ABI.md`](ABI.md)                            | The contract a foreign host assumes                                                                                                                                                              | —                  |
| [`design/`](design/)                          | How it works and why: assumptions, the effects stdlib, channels and spawning, capabilities, cancellation, related work                                                                         | —                  |
| [`spec/`](spec/)                              | The host protocol as a model-checked spec (Quint): the library's rules and the recommended host loop                                                                                            | —                  |
| [`demo/`](demo/)                              | Routines from a greeter to a ring of machines; native contexts for tokio and for JS (wasm-bindgen, no driver); a reifying context with `Full`/`Quiet` vocabularies; a C-ABI binding driven from Python and Java | —                  |

### Not Here, on Purpose

A scheduler inside the library (the host is the scheduler, whichever host it is); supervision and linking; deadlock levels; language-side host SDKs beyond the demo; `pyo3`/`rustler` bindings. Each is a natural next layer; none is needed to use what is here.

## How It Is Checked

- _Four runtimes, one transcript._ Every demo routine runs natively on tokio and on Node, and driven from Python and Java; their outputs must match byte for byte.
- _Adversarial schedules._ In Rust, a test runner replays routines under bolero-chosen schedules — replies delayed, machines interleaved, spurious resumes, machines killed — and the output must not change. Every foreign host has a seeded adversarial mode too, checked against tokio's transcript, and every seeded Python run is recorded and replayed in Rust.
- _The protocol, model-checked._ `spec/host_protocol.qnt` states the host protocol in Quint; TLC proves no wake is lost and every verdict — finished, or stalled — is sound, and catches the mistaken hosts that ignore `woke` or closed frames. Traces drawn from the spec are replayed against the real handle table, call by call.
- _Concurrency._ `loom` explores every interleaving of the driver's waker and outbox; a host of 2–4 uncoordinated threads drives the handle table, collisions and all.
- _Mutation testing._ `cargo-mutants` over the library crates: every mutant is caught, or excluded with its reason.

## Development

```sh
nix develop   # dev shell with a command menu
menu          # list project commands: ci:full, demo, test:no_std, …
```

CI runs the same commands in the flake's `ci` shell, so its tools are the ones `flake.lock` pins. Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`; the demo's hosts need Python 3, a JDK with `java.lang.foreign` (25), Node, and `wasm-bindgen-cli` at the version pinned in `Cargo.toml`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
