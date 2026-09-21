# effect-routine

> _sans-io, sans effort_

[![CI](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)
[![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/effect_routine)

An _effect routine_ is a coroutine written in direct style as an ordinary `async fn`, whose every wait is a typed effect answered by whoever drives it. No waker, no executor, no `Pin` in the routine; one `Box::pin` at the boundary. It is a sans-io state machine that the compiler writes for you — and because the routine asks for _traits_ rather than effects, the very same code is also a plain `async fn` that tokio runs at native speed with no driver at all.

This is the library. The research that motivates it, with the alternatives built out and measured, lives in [`effect-routines-exploration`][exploration]:

| If you want                                  | Read                                          |
|----------------------------------------------|-----------------------------------------------|
| To write an effect routine today             | [_How to Write Effect Routines_][guide] (PDF) |
| The argument, and the counter-argument       | [`analysis/README.md`][analysis]              |
| The same program in six styles, side by side | [`compare/`][compare]                         |
| What it costs per step, per host             | [`hosts/bench/`][bench]                       |

[exploration]: https://tangled.org/expede.wtf/effect-routines-exploration
[guide]: https://tangled.org/expede.wtf/effect-routines-exploration/raw/main/guide/how-to-write-effect-routines.pdf
[analysis]: https://tangled.org/expede.wtf/effect-routines-exploration/blob/main/analysis/README.md
[compare]: https://tangled.org/expede.wtf/effect-routines-exploration/tree/main/compare
[bench]: https://tangled.org/expede.wtf/effect-routines-exploration/tree/main/hosts/bench

## Three layers

```text
  ┌──────────────────────────────────────────────────────────────────────┐
  │ routine      Greeter<C: Clock + Directory + Input + Output>: Run     │  no_std
  │              owns the logic; asks for traits; knows nothing of        │
  │              effects, handles, drivers, or hosts                      │
  ├──────────────────────────────────────────────────────────────────────┤
  │ context      impl Clock for TokioCtx    │  impl Clock for Ctx<E>      │
  │              each call is a real future │  each call records an       │
  │                                         │  effect and suspends        │
  ├─────────────────────────────────────────┼────────────────────────────┤
  │ host         tokio polls the task       │  a Driver polls; a host     │
  │              directly — no driver       │  performs and replies by id │
  │                                         │  Python · JS · a test · …   │
  └─────────────────────────────────────────┴────────────────────────────┘
```

The routine is the foundation and depends on nothing above it. The left column is why you write it this way: it is also an ordinary library function. The right column is what this crate provides.

## The routine

```rust
pub trait Clock     { async fn sleep(&self, d: Duration); }
pub trait Directory { async fn lookup(&self, name: String) -> String; }
pub trait Input     { async fn read_line(&self) -> String; }
pub trait Output    { fn write(&self, line: String); }

impl<C: Clock + Directory + Input + Output> Run for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write("Who are you?".into());
        let name = self.ctx.read_line().await;
        let greeting = self.ctx.lookup(name.clone()).await;
        self.ctx.sleep(PAUSE).await;
        self.ctx.write(format!("{greeting}, {name}!"));
        ControlFlow::Continue(())
    }
}
```

## Two contexts, one routine

Natively, on tokio — no driver, no effects, tokio polls the task:

```rust
impl Clock for TokioCtx {
    async fn sleep(&self, d: Duration) { tokio::time::sleep(d).await }
}
// …
tokio::spawn(Greeter::new(TokioCtx::new(stdin)).run());
```

Behind a host — each call records a request and suspends until the host replies by id. The context is generic over the host's vocabulary `E`, so a host can offer a routine _less_ than everything, and the type system enforces it:

```rust
impl<E: From<Asked<Sleep>>> Clock for Ctx<E> {
    async fn sleep(&self, d: Duration) { self.outbox.request(Sleep(d)).await }
}
// …
Driver::<Full>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());   // ok
Driver::<Quiet>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());  // E0277: Ctx<Quiet, _>: Directory
Driver::<Quiet>::new(|outbox| Ticker::new(Ctx::new(outbox), 3).run()); // ok: Ticker needs only Clock + Output
```

`demo/` has all of this in full — the greeter, the ticker, both contexts, a C-ABI skin with a Python host, a wasm-bindgen skin with a JS host — and `nix develop` then `demo` runs the routine natively and behind both foreign hosts and checks the transcripts are byte-identical.

## How the wire works

```text
  host                                    routine
    │                                        │
    │  start()                               │
    │───────────────────────────────────────▶│  runs until it needs input:
    │                                        │  tells Write, asks ReadLine·1
    │  [Write, ReadLine·1]   AWAITING        │
    │◀───────────────────────────────────────│
    │                                        │
    │  reply(1, "bob")                       │
    │───────────────────────────────────────▶│  resumes; asks Lookup·2
    │  [Lookup·2]            AWAITING        │
    │◀───────────────────────────────────────│
    │                                        │
    │  reply(2, "Hello")                     │
    │───────────────────────────────────────▶│  resumes; tells Write, returns
    │  [Write]               COMPLETE        │
    │◀───────────────────────────────────────│
    │                                        ┴
```

- _Pull-only._ The routine asks for everything it needs, but by _returning_ an effect from `start`/`reply`, never by calling the host. No callbacks, no upcalls, so no foreign value ever enters a Rust frame — which is why the routine is `Send` for free.
- _Typed replies._ Every awaiting effect carries a `ReplyHandle<T>`: the typed, single-use capability to answer it. A Rust host replies through the handle, infallibly; a foreign host replies by id, and the host layer checks the kind.
- _Many waits outstanding._ Requests carry ids, so a routine may `join` two waits and a host may reply in any order.
- _`no_std` core._ The mechanism, the routine, and its wire crate all build for `wasm32`; the mechanism for `thumbv6m` with `critical-section`.

## Crates

| Crate                                         | Purpose                                                                                                        | Target             |
|-----------------------------------------------|----------------------------------------------------------------------------------------------------------------|--------------------|
| [`effect_routine`](effect_routine/)           | The mechanism: `Run`, `Outbox`, `ReplyHandle`, `Request`, `Driver`, `join`, the reply menu, `wire`, `testing`    | `no_std` + `alloc` |
| [`effect_routine_host`](effect_routine_host/) | The host side for foreign hosts: a typed `Machine`, a byte layer, a handle table, panic isolation. No `unsafe` | `std`              |
| [`ABI.md`](ABI.md)                            | The contract a foreign host assumes                                                                            | —                  |
| [`demo/`](demo/)                              | The greeter and ticker; a native tokio host; a reifying context with `Full`/`Quiet` vocabularies; a C-ABI skin + Python host; a wasm-bindgen skin + JS host | — |

### Not here, on purpose

An in-process scheduler that routes between many routines; child routines; deadlock levels; language-side host SDKs beyond the demo; a derive for the `View`/`Encode` restatement (next); `pyo3`/`rustler` skins. Each is a natural next layer; none is needed to use what is here.

## Development

```sh
nix develop   # dev shell with a command menu
menu          # list project commands: ci:full, demo, test:no_std, …
```

Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`; the demo's foreign hosts need Python 3, Node, and `wasm-bindgen-cli` at the version pinned in `Cargo.toml`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
