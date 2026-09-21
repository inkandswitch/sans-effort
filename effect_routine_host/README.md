# effect_routine_host

> The host side of an effect routine, for hosts that cannot hold a Rust value

A Rust host drives a routine through `effect_routine::driver::Driver`, replying through typed `ReplyHandle`s. A host in another language has neither the handle nor the type: it has a `u64` request id and some bytes. This crate is the layer between — safe Rust, over owned types, with `unsafe_code = "forbid"`. The application adds the `extern "C"` (or wasm-bindgen, or `erl_nif`) skin: one wrapper per function, each a line plus the `unsafe` needed to touch foreign memory.

```text
  app skin (unsafe, ~40 lines)   ─▶   effect_routine_host (this crate)   ─▶   effect_routine::Driver
  new / start / reply / free            Machine: id → ReplyHandle, kind check
  buf_free                              table:   u64 handles, BUSY, panic isolation
                                        bytes:   decode one reply record, encode the effects
```

## What is here

| Item                                      | Role                                                                                                                                                                                                                                                                              |
|-------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `Machine<E>`                              | A `Driver<E>` plus a wallet of the `ReplyHandle`s its effects carried out, keyed by id. `start()` and `reply(id, value)` return `Vec<E::View>` — effects with handles replaced by ids — and check the reply's kind at run time, the one place in the stack a reply can be refused |
| `Machine::{start_encoded, reply_encoded}` | The byte layer: one reply record (`kind · id · payload`) in, the effects encoded out                                                                                                                                                                                              |
| `table`                                   | `new(routine) → u64`, `start(h)`, `reply(h, record)`, `free(h)`. Handles never `0`, never reused. Any thread, one at a time per handle (`BUSY` on collision). A panicking routine is caught, removed, and reported as `PANICKED`                                                  |
| `code`, `Status`, `Error`                 | The status and error codes as they cross the ABI, and their Rust forms                                                                                                                                                                                                            |

`ABI.md` in the repository root is the contract a foreign host assumes, and `demo/` there has a C-ABI skin with a Python host driving the same routine that runs natively on tokio and on the JS event loop.

## Two kinds of skin

The raw path: a `cdylib` exports the five functions over `table`, and any language that can `dlopen` and hand over a byte buffer drives the routine, decoding effects from a tag table the routine's author documents. The generated path: a `PyO3` or Rustler class holds a `Machine` directly and converts `View`s to native objects — no handle table, no codec, a per-language toolchain. (A JS host with wasm-bindgen usually needs neither: the event loop is an executor, so a routine runs there natively, as `demo/wasm` shows.) The routine cannot tell which it is under.

## License

MIT or Apache-2.0, at your option.
