# sans-effort-host

> The host side of a routine, for FFI hosts that cannot hold a Rust value — minus the C ABI itself

A Rust host drives a routine through `sans_effort_core::driver::Driver`, replying through typed `ReplyHandle`s. A foreign host has neither the handle nor the type: it has a `u64` request id and some bytes. This crate is the layer between — safe Rust over owned types, with `unsafe_code = "forbid"`. Routine authors get it through `sans-effort`'s `host` feature.

The calls: `abi_version()`; `new(routine) → handle`; `resume(handle) → effects` to begin; `reply(handle, record) → effects` for each answer; `resume` again for a routine waiting on something inside the process, which a `woke` frame in some call's output (or `wakes()`) names; then `free(handle)`. A reply record is `kind · id · payload`: the id came out on the wire with the effect, and the kind is one of the reply menu's four (`str`, `u64`, `unit`, `bytes`). `ABI.md` in the repository is the contract in full.

## What Is Here

| Item | Role |
|------|------|
| `machine::Machine<E, F>` | _Typed._ A `Driver<E, F>` plus the `ReplyHandle`s its effects carried out, keyed by id. `resume()` and `reply(id, value)` return a `Yield<E::View>` — the effects with handles replaced by ids, and the ids the routine abandoned — and check the reply's kind, the one place in the stack a reply can be refused. `reply_str`/`reply_u64`/`reply_unit`/`reply_bytes` serve bindings that cannot call a generic method. Bindings that speak the host language's types — wasm-bindgen, `PyO3`, Rustler — hold one directly |
| `encoded::Encoded<E, F>` | _Bytes._ The same calls over a `Machine`: decode one reply record, call the typed method, encode the views as frames |
| `table` | Machines behind `u64` handles, for a C-ABI or `erl_nif` binding: `new`, `park_pinned`, `resume`, `reply`, `wakes`, `free`. Handles are never `0` and never reused. A migrating machine runs on any thread, one call at a time (`BUSY` on collision); a pinned one — its future not `Send` — is built by the thread that first resumes it and stays there (`WRONG_THREAD` from any other). A machine woken by another's poll is named by a `woke` frame at the end of that call's output; one woken outside any call, by `wakes()`. A panicking routine is caught, removed, and reported as `PANICKED` |
| `record` | A tap on the table: `record(root)` logs every `resume`, `reply`, and `free` on the root and the machines it creates, whichever host makes them, with each outcome; `replay(&log, make_root)` makes the same calls again, re-issuing the recorded handles, and reports the first outcome that differs. Logs encode to bytes, so a failing foreign run becomes a log you can replay in Rust. Exact for a host that makes one call at a time |
| `contract`, `error::Error` | `ABI.md`'s numbers — its `REVISION`, frame kinds, status and error codes — and the errors' Rust form: why a call or a reply was refused (`WrongKind`, `Malformed`, `Stale`, …) |

The application adds the _binding_: the `unsafe` wrapper a foreign host calls — one `#[no_mangle]` function per call in `table`, each a line plus the `unsafe` needed to touch foreign memory.

```text
  app cdylib                                 sans-effort-host
  ────────────────────────────────           ──────────────────────────────────────────
  enum Effect { … }  impl HostEffect
  #[no_mangle] abi_version()     ─────────▶  contract::REVISION
  #[no_mangle] new()             ─────────▶  table::new(|outbox| Greeter::new(Ctx::new(outbox)).run())
  #[no_mangle] reply(h, in*, out*) ───────▶  table::reply(h, &[u8])   -> Result<(Vec<u8>, Status), Error>
  #[no_mangle] resume(h, out*)   ─────────▶  table::resume(h)         -> Result<(Vec<u8>, Status), Error>
  #[no_mangle] wakes(out*)       ─────────▶  table::wakes()           -> Vec<u8>
                                 ◀─────────  (bytes, status)   — app writes the out-pointers
```

`demo/` in the repository has such a binding, driven from Python (ctypes) and Java (Panama).

## Two Kinds of Binding

The raw path: a `cdylib` exports the functions over `table`, and any language that can `dlopen` and hand over a byte buffer drives the routine, decoding effects from a tag table the routine's author documents. The generated path: a `PyO3` or Rustler class holds a `Machine` directly and converts `View`s to native objects — no handle table, no codec, a per-language toolchain. A JS host with wasm-bindgen usually needs neither: the event loop is an executor, so a routine runs there natively. The routine cannot tell which it is under.

## `no_std`

Everything but `table` and `record` is `no_std` + `alloc`. The table needs a process-wide lock, a `HashMap`, and `catch_unwind` for panic isolation, so it is behind the default `std` feature; a binding on a target without `std` holds its `Encoded` machines in statics of its own.
