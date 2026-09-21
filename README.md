# effect-routine

> _sans-io, sans effort_

[![CI](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)
[![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/effect_routine)

An _effect routine_ is a coroutine written in direct style as an ordinary `async fn`, whose every wait is a typed effect answered by whoever drives it. No waker, no executor, no `Pin` in the routine; one `Box::pin` at the boundary. It is a sans-io state machine that the compiler writes for you, and the same routine is driven — unchanged — by a Rust host, by Python over a C ABI, or by anything that can hold a byte buffer.

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

## How it works

```text
  host                                    routine
    │                                        │
    │  step()                                │
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

- _Pull-only._ The routine asks for everything it needs, but by _returning_ an effect from `step`, never by calling the host. Replies arrive as the argument to the next `step`. No callbacks, no upcalls, so no foreign value ever enters a Rust frame — which is why the routine is `Send` for free.
- _Typed effects._ Every wait is a value in a closed `Effect` type the routine owns, carrying a `ReplyHandle<T>`: the typed, single-use capability to answer it. The enum doubles as the wire.
- _Many waits outstanding._ Requests carry ids, so a routine may fan out and a host may reply in any order.
- _`no_std` core._ Builds for `wasm32` and, with `critical-section`, for `thumbv6m`.

## A routine

```rust
enum Effect {
    ReadLine(ReplyHandle<String>),
    Write(String),
}

impl<O: Post<Effect>> Run for Greeter<O> {
    async fn turn(&mut self) -> ControlFlow<()> {
        self.outbox.tell(Effect::Write("Who are you?".into()));
        let name = self.outbox.ask(Effect::ReadLine).await;
        self.outbox.tell(Effect::Write(format!("Hello, {name}!")));
        ControlFlow::Break(())
    }
}
```

And a host, in Rust, in full:

```rust
let mut driver = Driver::new(|outbox| Greeter { outbox }.run());
let mut queue: VecDeque<Effect> = driver.start().into();

while let Some(effect) = queue.pop_front() {
    match effect {
        Effect::Write(text) => println!("{text}"),
        Effect::ReadLine(reply) => queue.extend(driver.reply(reply, read_line())),
    }
}
```

`demo/` has the full greeter — four kinds of wait, fan-out — with a Rust host, a C-ABI skin, and a Python host that produce byte-identical transcripts. `nix develop` then `demo` runs all three.

## Crates

| Crate                                         | Purpose                                                                                                          | Target             |
|-----------------------------------------------|------------------------------------------------------------------------------------------------------------------|--------------------|
| [`effect_routine`](effect_routine/)           | The mechanism: `Run`, `Post`, `ReplyHandle`, `Driver`, `join`, the reply menu, and the `wire` traits             | `no_std` + `alloc` |
| [`effect_routine_host`](effect_routine_host/) | The host side for foreign hosts: a typed `Machine`, a byte layer, a handle table, panic isolation. No `unsafe`   | `std`              |
| [`ABI.md`](ABI.md)                            | The contract a foreign host assumes                                                                              | —                  |
| [`demo/`](demo/)                              | The greeter, a Rust host, the C-ABI skin (the one crate with `unsafe`), and a Python host                       | —                  |

### Three styles, one mechanism

The library does not pick how you write the routine; the mechanism is the same for all three.

| Style          | The routine…                                                                          | Pick it when                                                          |
|----------------|---------------------------------------------------------------------------------------|-----------------------------------------------------------------------|
| _effects_      | owns a closed `Effect` enum; the enum is the wire                                     | the default: one routine, driven by a host, simulator, or replay      |
| _coeffects_    | describes waits as `Request` structs and states `E: From<Asked<Lookup>>` bounds       | several routines share a host, or a host must attenuate what it offers |
| _capabilities_ | asks for traits (`C: Clock + Directory`) and never sees an effect; a context decides  | the same logic must also run as a plain `async fn` on tokio            |

### Not here, on purpose

An in-process scheduler that routes between many routines; child routines; deadlock levels; language-side host SDKs beyond the demo; `pyo3`/`rustler`/`wasm-bindgen` skins. Each is a natural next layer; none is needed to use what is here.

## Development

```sh
nix develop   # dev shell with a command menu
menu          # list project commands: ci:full, demo, test:no_std, …
```

Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`, and `demo/python/main.py` needs only a Python 3 interpreter.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
