# The ABI

What a skin built on `effect_routine_host` exports, and what a foreign host may assume. A host that assumes this and nothing else is generic over routines: it can drive any effect routine whose skin speaks it, given only the routine's tag table.

The shape is sans-io's: `new`, `step(handle, input) → effects`, `free`.

| Element | Contract |
|---------|----------|
| Calls | `<prefix>_new() → u64`, `<prefix>_step(u64, in_ptr, in_len, out_ptr, out_len) → i32`, `<prefix>_free(u64) → i32`, `<prefix>_buf_free(ptr, len)`. A skin may export more constructors (`<prefix>_new_fanout()`); all return the same handle type. |
| Machine handles | `u64`, never `0`, never reused; a stale handle is `BAD_HANDLE`, not a fault. |
| Codes | `i32`; `>= 0` is a status (`0` `OK`/`AWAITING`, `1` `COMPLETE`, `2` `STALLED`), `< 0` an error (`-1` `BUSY`, `-2` `FINISHED`, `-4` `PANICKED`, `-5` `BAD_HANDLE`, `-6` `BAD_INPUT`). |
| Out-buffers | On `>= 0` the host copies the buffer and frees it with `<prefix>_buf_free(ptr, len)`; on `< 0` nothing was written. |
| Inputs | `(ptr, len)` byte slices, valid for the duration of the call; exactly one record per call: `0` start, then one reply per awaited effect — `1 id str`, `2 id u64`, `3 id`, `4 id bytes` — typed by reply kind, not by effect. Trailing bytes are `BAD_INPUT`. |
| Effects | Little-endian; `str` and `bytes` are `u32 len` + payload; one `u8` tag per record, records concatenated; an awaiting effect's record ends with its `u64` request id. |
| Request ids | Per machine, from `1`, increasing. Any number may be outstanding at once, and the host may reply in any order. An id the routine has abandoned is `BAD_INPUT`. |
| Upcalls | None. The host calls in; the routine never calls out. No callback is registered, no host value is held on the Rust side. |
| Threading | Any thread, one at a time per handle: two threads stepping one handle get `BUSY`, not a race. A host may pool, and a machine's steps migrate between threads. |
| Panics | A routine that panics is removed; the call returns `PANICKED` and every later call on that handle is `BAD_HANDLE`. The process is not aborted. |

## The reply menu

Four kinds, and no more. A richer reply crosses as `bytes` and is decoded on the routine's side.

| Input tag | Kind | Payload | Typical use |
|-----------|------|---------|-------------|
| `1` | `str` | `u32 len` + UTF-8 | text |
| `2` | `u64` | 8 bytes LE | counts, ids, timestamps, lengths |
| `3` | `unit` | — | a sleep, an ack |
| `4` | `bytes` | `u32 len` + bytes | anything else |

Replying with the wrong kind for an id is `BAD_INPUT`, and the request stays outstanding, so the host may retry with the right kind.

## What is not in the ABI

And therefore lives with each routine:

- _The tag table_ — which `u8` means which effect, what fields follow it, and which reply kind answers it. The routine's `Encode` impl is the source of truth; `demo/greeter` documents its five tags on the `View` enum.
- _The world_ — what performing an effect means: where `Write` goes, what `Lookup` consults, whether `Sleep` is real or virtual.
- _The driver loop_ — the host's own; `demo/python/main.py` is one in ~40 lines.

## A conversation

The greeter in `demo/`, driven from start to `quit`:

```text
host → step(h, [00])                                   start
     ← 05 "Who are you?" · 03 id=1               AWAITING     Write, ReadLine·1
host → step(h, [01 id=1 "alice"])                      reply str
     ← 02 "alice" id=2                           AWAITING     Lookup·2
host → step(h, [01 id=2 "Hello"])
     ← 04 millis=50 id=3                         AWAITING     Sleep·3
host → step(h, [03 id=3])                              reply unit
     ← 05 "Hello, alice!" · 01 id=4              AWAITING     Write, Count·4
host → step(h, [02 id=4 1])                            reply u64
     ← 05 "(greeted 1 so far)" · 05 "Who are you?" · 03 id=5
                                                 AWAITING
host → step(h, [01 id=5 "quit"])
     ← 05 "Bye."                                 COMPLETE
host → free(h)                                   OK
```

## Relationship to the exploration

This is `hosts/effects/ABI.md` from [`effect-routines-exploration`](https://tangled.org/expede.wtf/effect-routines-exploration), with the `local` (thread-affine) export set removed — a library ships one driver — and `u32` widened to `u64`. Hosts written against that dialect need those two changes and nothing else.
