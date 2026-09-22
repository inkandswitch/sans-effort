# sans-effort

> _sans-io, sans effort_

[![CI](https://github.com/inkandswitch/sans-effort/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/sans-effort/actions/workflows/test-host.yml) [![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT) [![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/sans-effort)

`sans-effort` lets you write a coroutine in direct style as an ordinary `async fn`, where every wait is a typed effect answered by whoever drives it. No waker, no executor, no `Pin` in the routine; one `Box::pin` at the boundary. It is a sans-io state machine that the compiler writes for you — and because the routine asks for _traits_ rather than effects, the very same code is also a plain `async fn` that tokio runs at native speed with no driver at all.

This is the library. The research that motivates it, with the alternatives built out and measured, lives in [`effect-routines-exploration`][exploration]:

| If you want                                  | Read                                          |
|----------------------------------------------|-----------------------------------------------|
| To write one today                           | [_How to Write Effect Routines_][guide] (PDF) |
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
  │ routine      Greeter<C: Sleep + Lookup + ReadLine + WriteLine>: Run  │  no_std
  │              owns the logic; asks for traits; knows nothing of        │
  │              effects, handles, drivers, or hosts                      │
  ├──────────────────────────────────────────────────────────────────────┤
  │ context      impl Sleep for TokioCtx    │  impl Sleep for Ctx<E>      │
  │              each call is a real future │  each call records an       │
  │                                         │  effect and suspends        │
  ├─────────────────────────────────────────┼────────────────────────────┤
  │ host         tokio or the JS event loop │  a Driver polls; a host     │
  │              polls the task — no driver │  performs and replies by id │
  │                                         │  Python · Java · a test · … │
  └─────────────────────────────────────────┴────────────────────────────┘
```

The routine is the foundation and depends on nothing above it. The left column is why you write it this way: it is also an ordinary library function. The right column is what these crates provide.

## The routine

```rust
pub trait Sleep     { async fn sleep(&self, d: Duration); }
pub trait Lookup    { async fn lookup(&self, name: String) -> String; }
pub trait ReadLine  { async fn read_line(&self) -> String; }
pub trait WriteLine { fn write_line(&self, line: String); }

impl<C: Sleep + Lookup + ReadLine + WriteLine> Run for Greeter<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        self.ctx.write_line("Who are you?".into());
        let name = self.ctx.read_line().await;
        let greeting = self.ctx.lookup(name.clone()).await;
        self.ctx.sleep(PAUSE).await;
        self.ctx.write_line(format!("{greeting}, {name}!"));
        ControlFlow::Continue(())
    }
}
```

## Two contexts, one routine

Natively, on tokio — no driver, no effects, tokio polls the task:

```rust
impl Sleep for TokioCtx {
    async fn sleep(&self, d: Duration) { tokio::time::sleep(d).await }
}
// …
tokio::spawn(Greeter::new(TokioCtx::new(stdin)).run());
```

Behind a host — each call records a request and suspends until the host replies by id. The context is generic over the host's vocabulary `E`, so a host can offer a routine _less_ than everything, and the type system enforces it:

```rust
impl<E: From<Asked<Sleep>>> traits::Sleep for Ctx<E> {
    async fn sleep(&self, d: Duration) { self.outbox.request(Sleep(d)).await }
}
// …
Driver::<Full>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());   // ok
Driver::<Quiet>::new(|outbox| Greeter::new(Ctx::new(outbox)).run());  // E0277: Ctx<Quiet>: Lookup
Driver::<Quiet>::new(|outbox| Ticker::new(Ctx::new(outbox), 3).run()); // ok: Ticker needs only Sleep + WriteLine
```

`demo/` has all of this in full — the greeter, the ticker, both contexts, native hosts on tokio and on Node (wasm-bindgen), and a C-ABI binding driven from Python (ctypes) and Java (Panama) — and `nix develop` then `demo` runs all four and checks the transcripts are byte-identical.

### Where this sits

This is the [tagless-final][tf] style with the representation pinned to `impl Future`: the traits are the algebra, a routine is a term abstract in its interpreter, `TokioCtx` is the evaluating interpreter, and `Ctx<E>` is the reifying one — the instance that recovers the initial, tagged encoding (`Full`) from the final one. `Quiet` is a smaller algebra. Rust readers may know the same shape as "capability traits" or MTL-style; the effects style, where the routine emits a tagged enum directly, is the initial encoding, and the two share one mechanism because initial and final encodings are interconvertible. Rust has no higher-kinded types, so the representation cannot vary; the interpreter does.

[tf]: https://okmij.org/ftp/tagless-final/index.html

## How a host drives a routine

```text
  host                                    routine
    │                                        │
    │  start()                               │
    │───────────────────────────────────────▶│  runs until it needs input:
    │                                        │  tells WriteLine, asks ReadLine·1
    │  [WriteLine, ReadLine·1]  AWAITING     │
    │◀───────────────────────────────────────│
    │                                        │
    │  reply(1, "bob")                       │
    │───────────────────────────────────────▶│  resumes; asks Lookup·2
    │  [Lookup·2]            AWAITING        │
    │◀───────────────────────────────────────│
    │                                        │
    │  reply(2, "Hello")                     │
    │───────────────────────────────────────▶│  resumes; tells WriteLine, returns
    │  [WriteLine]           COMPLETE        │
    │◀───────────────────────────────────────│
    │                                        ┴
```

- _Pull-only._ The routine asks for everything it needs, but by _returning_ an effect from `start`/`reply`, never by calling the host. No callbacks, no upcalls, so no foreign value ever enters a Rust frame — which is why the routine is `Send` for free.
- _Typed replies._ Every awaiting effect carries a `ReplyHandle<T>`: the typed, single-use capability to answer it. A Rust host replies through the handle, infallibly; a foreign host replies by id, and the host layer checks the kind.
- _Many waits outstanding._ Requests carry ids, so a routine may `join` two waits and a host may reply in any order.
- _`no_std` core._ The mechanism, the routine, and its boundary crate all build for `wasm32`; the mechanism for `thumbv6m` with `critical-section`.

## Crates

| Crate                                   | Purpose                                                                                                                                                                                  | Target             |
|-----------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------|
| [`sans-effort`](sans-effort/)           | The mechanism: `Run`, `Outbox`, `ReplyHandle`, `Request`, `Driver`, `join`, the reply menu, `boundary`, `testing`                                                                        | `no_std` + `alloc` |
| [`sans-effort-host`](sans-effort-host/) | The host side for foreign hosts: a typed `Machine`, the `Encoded` byte layer, a handle table, panic isolation. No `unsafe`                                                               | `std`              |
| [`ABI.md`](ABI.md)                      | The contract a foreign host assumes                                                                                                                                                      | —                  |
| [`demo/`](demo/)                        | The greeter and ticker; native contexts for tokio and for JS (wasm-bindgen, no driver); a reifying context with `Full`/`Quiet` vocabularies; a C-ABI binding driven from Python and Java | —                  |

### Next

A standard library of common capabilities — `Sleep`, console I/O, and actors (`Post`, `Receive`, `Spawn`) — as `sans-effort-effects`, with the reifying context shipped once and a native tokio context beside it; a derive for the `View`/`Encode` restatement. The demo grows routines that spawn and message each other, with each host's loop as the scheduler.

### Not here, on purpose

A scheduler inside the library (the host is the scheduler, whichever host it is); supervision and linking; deadlock levels; language-side host SDKs beyond the demo; `pyo3`/`rustler` bindings. Each is a natural next layer; none is needed to use what is here.

## Development

```sh
nix develop   # dev shell with a command menu
menu          # list project commands: ci:full, demo, test:no_std, …
```

Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`; the demo's hosts need Python 3, a JDK with `java.lang.foreign` (25), Node, and `wasm-bindgen-cli` at the version pinned in `Cargo.toml`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
