# effect-routine

> _sans-io, sans effort_

[![CI](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)
[![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/effect_routine)

An _effect routine_ is an isolated coroutine, written in direct style as an ordinary `async fn`, whose every wait is a typed effect answered by whoever drives it. No waker, no executor, no `Pin` in the program; one `Box::pin` at the boundary. The same program runs as a plain tokio task, a single-threaded Wasm loop, or a foreign runtime over FFI, and cannot tell the difference.

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
    │                                        │  emits Write, awaits ReadLine·1
    │  [Write, ReadLine·1]   AWAITING        │
    │◀───────────────────────────────────────│
    │                                        │
    │  answer(1, "bob")                      │
    │───────────────────────────────────────▶│  resumes; awaits Lookup·2
    │  [Lookup·2]            AWAITING        │
    │◀───────────────────────────────────────│
    │                                        │
    │  answer(2, "Hello")                    │
    │───────────────────────────────────────▶│  resumes; emits Write, returns
    │  [Write]               COMPLETE        │
    │◀───────────────────────────────────────│
    │                                        ┴
```

- _Pull-only._ The routine asks for everything it needs, but by _returning_ an effect from `step`, never by calling the host. Answers arrive as the argument to the next `step`. No callbacks, no upcalls, so no foreign value ever enters a Rust frame.
- _Typed effects._ Every wait is a value in a closed `Effect` type, which doubles as the wire.
- _Many waits outstanding._ Requests carry ids, so a routine may fan out.
- _`no_std` core._ No allocator requirement beyond the routine's own state; buildable for `wasm32` and `thumbv6m`.

## Crates

| Crate                               | Purpose                                         |
|-------------------------------------|-------------------------------------------------|
| [`effect_routine`](effect_routine/) | Core: the routine, its effects, and the drivers |

## Development

```sh
nix develop # dev shell with a command menu
```

Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
