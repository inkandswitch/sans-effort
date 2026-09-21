# effect-routine

> _sans-io, sans effort_

[![CI](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml/badge.svg)](https://github.com/inkandswitch/effect-routine/actions/workflows/test-host.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)
[![no_std](https://img.shields.io/badge/no__std-compatible-green)](https://docs.rs/effect_routine)

An _effect routine_ is an isolated coroutine, written in direct style as an ordinary `async fn`, whose every wait is a typed effect answered by whoever drives it. No waker, no executor, no `Pin` in the program; one `Box::pin` at the boundary. The same program runs as a plain tokio task, a single-threaded Wasm loop, or a foreign runtime over FFI, and cannot tell the difference.

This is the library. The research that motivates it, with the alternatives built out and measured, lives in [`effect-routines-exploration`](https://github.com/inkandswitch/effect-routines-exploration).

## How it works

```text
host                                  routine
step(0)             ──────▶           emits Write, awaits ReadLine·1
◀── [Write, ReadLine·1], AWAITING
answer 1 "bob"      ──────▶           resumes; emits Lookup·2
```

- _Pull-only._ The host calls in; the routine never calls out.
- _Typed effects._ Every wait is a value in a closed `Effect` type, which doubles as the wire.
- _Many waits outstanding._ Requests carry ids, so a routine may fan out.
- _`no_std` core._ No allocator requirement beyond the routine's own state; buildable for `wasm32` and `thumbv6m`.

## Crates

| Crate | Purpose |
|-------|---------|
| [`effect_routine`](effect_routine/) | Core: the routine, its effects, and the drivers |

## Development

```sh
nix develop   # dev shell with a command menu
menu          # list project commands
```

Without Nix, `rust-toolchain.toml` pins the toolchain for `rustup`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
