# The ABI

What a skin built on `effect_routine_host` exports, and what a foreign host may assume. A host that assumes this and nothing else is generic over routines: it can drive any effect routine whose skin speaks it, given only the routine's tag table.

The shape: `new`, `start(handle) → effects`, then `reply(handle, record) → effects` until the status is `COMPLETE`, then `free`. Each call returns the effects the routine recorded before its next wait.

| Element | Contract |
|---------|----------|
| Calls | `<prefix>_new() → u64`, `<prefix>_start(u64, out_ptr, out_len) → i32`, `<prefix>_reply(u64, in_ptr, in_len, out_ptr, out_len) → i32`, `<prefix>_free(u64) → i32`, `<prefix>_buf_free(ptr, len)`. A skin may export more constructors (`<prefix>_new_fanout()`, `<prefix>_new_ticker()`); all return the same handle type and answer to the same calls. |
| Machine handles | `u64`, never `0`, never reused; a stale handle is `BAD_HANDLE`, not a fault. |
| Codes | `i32`; `>= 0` is a status (`0` `OK`/`AWAITING`, `1` `COMPLETE`, `2` `STALLED`), `< 0` an error (`-1` `BUSY`, `-2` `FINISHED`, `-4` `PANICKED`, `-5` `BAD_HANDLE`, `-6` `BAD_INPUT`). |
| Out-buffers | On `>= 0` the host copies the buffer and frees it with `<prefix>_buf_free(ptr, len)`; on `< 0` nothing was written. |
| `start` | Valid once per handle; a second call is `BAD_INPUT`. Returns the effects recorded before the first wait. |
| `reply` | `(ptr, len)` is exactly one reply record: `kind · id · payload`, where kind is `1 str`, `2 u64`, `3 unit`, `4 bytes` — typed by reply kind, not by effect. Trailing bytes are `BAD_INPUT`. Returns the effects recorded before the next wait. |
| Effects | Little-endian; `str` and `bytes` are `u32 len` + payload; one `u8` tag per record, records concatenated; an awaiting effect's record ends with its `u64` request id. |
| Request ids | Per machine, from `1`, increasing. Any number may be outstanding at once, and the host may reply in any order. An id the routine has abandoned is `BAD_INPUT`. |
| Upcalls | None. The host calls in; the routine never calls out. No callback is registered, no host value is held on the Rust side. |
| Threading | Any thread, one at a time per handle: two threads driving one handle get `BUSY`, not a race. A host may pool, and a machine's calls migrate between threads. |
| Panics | A routine that panics is removed; the call returns `PANICKED` and every later call on that handle is `BAD_HANDLE`. The process is not aborted. |

## The reply menu

Four kinds, and no more. A richer reply crosses as `bytes` and is decoded on the routine's side.

| Reply kind | Kind | Payload | Typical use |
|-----------|------|---------|-------------|
| `1` | `str` | `u32 len` + UTF-8 | text |
| `2` | `u64` | 8 bytes LE | counts, ids, timestamps, lengths |
| `3` | `unit` | — | a sleep, an ack |
| `4` | `bytes` | `u32 len` + bytes | anything else |

Replying with the wrong kind for an id is `BAD_INPUT`, and the request stays outstanding, so the host may retry with the right kind.

## What is not in the ABI

And therefore lives with each routine:

- _The tag table_ — which `u8` means which effect, what fields follow it, and which reply kind answers it. The wire crate's `Encode` impl is the source of truth; `demo/wire` documents its five tags on the `View` enum.
- _The world_ — what performing an effect means: where `Write` goes, what `Lookup` consults, whether `Sleep` is real or virtual.
- _The host loop_ — the host's own; `demo/python/main.py` is one in ~40 lines.

## A conversation

The greeter in `demo/`, driven from start to `quit`:

```text
host → start(h)
     ← 05 "Who are you?" · 03 id=1               AWAITING     Write, ReadLine·1
host → reply(h, [01 id=1 "alice"])                       str
     ← 02 "alice" id=2                           AWAITING     Lookup·2
host → reply(h, [01 id=2 "Hello"])                       str
     ← 04 millis=50 id=3                         AWAITING     Sleep·3
host → reply(h, [03 id=3])                               unit
     ← 05 "Hello, alice!" · 01 id=4              AWAITING     Write, Count·4
host → reply(h, [02 id=4 1])                             u64
     ← 05 "(greeted 1 so far)" · 05 "Who are you?" · 03 id=5
                                                 AWAITING
host → reply(h, [01 id=5 "quit"])                        str
     ← 05 "Bye."                                 COMPLETE
host → free(h)                                   OK
```

A `Quiet` machine (`<prefix>_new_ticker()` in the demo) is driven by the same loop and only ever produces tags 4 and 5 — the routine's trait bounds guarantee it, and the host can rely on it.

## Provenance

This descends from `hosts/effects/ABI.md` in [`effect-routines-exploration`](https://tangled.org/expede.wtf/effect-routines-exploration), which eight hosts in five languages implemented. It is not compatible with it and does not try to be: the thread-affine export set is gone, `start` is its own call rather than a tag-`0` input record, `u32` is `u64`, and trailing bytes are rejected. This document is the contract; the exploration is where it was tested.
