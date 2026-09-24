# The ABI

A _binding_ is the thin `unsafe` wrapper an application puts around `sans-effort-host` so a foreign host can call it. This is what a binding exports, and what a foreign host may assume. A host that assumes this and nothing else is generic over routines: it can drive any routine whose binding speaks it, given only the routine's tag table.

The shape: `abi_version` once, then `new`, `start(handle) → effects`, then `reply(handle, record) → effects` — or `resume(handle) → effects` while the status is `IDLE` — until the status is `COMPLETE`, then `free`. Each call returns the effects the routine recorded before its next wait.

| Element | Contract |
|---------|----------|
| Calls | `<prefix>_abi_version() → u8`, `<prefix>_new() → u64`, `<prefix>_start(u64, out_ptr, out_len) → i32`, `<prefix>_reply(u64, in_ptr, in_len, out_ptr, out_len) → i32`, `<prefix>_resume(u64, out_ptr, out_len) → i32`, `<prefix>_wakes(out_ptr, out_len) → i32`, `<prefix>_free(u64) → i32`, `<prefix>_buf_free(ptr, len)`. A binding may export more constructors, with or without an in-buffer of encoded arguments (`<prefix>_new_ticker()`, `<prefix>_new_greeter(in_ptr, in_len)`); all return the same handle type and answer to the same calls. |
| Machine handles | `u64`, never `0`, never reused; a stale handle is `BAD_HANDLE`, not a fault. |
| Codes | `i32`; `>= 0` is a status (`0` `OK`/`AWAITING`, `1` `COMPLETE`, `2` `IDLE`), `< 0` an error (`-1` `BUSY`, `-2` `FINISHED`, `-3` `WRONG_KIND`, `-4` `PANICKED`, `-5` `BAD_HANDLE`, `-6` `BAD_INPUT`, `-7` `MALFORMED`, `-8` `STALE`, `-9` `WRONG_THREAD`). `IDLE` means suspended with no request outstanding: the routine waits on something inside the process, such as a channel another routine sends on. `BAD_INPUT` is a second `start`, a `reply` or `resume` before `start`, or an id never issued; `MALFORMED` a reply record that does not parse; `STALE` a reply to an id that was issued but is no longer awaited — answered already, or closed — and is harmless. `WRONG_THREAD` is a call for a pinned machine from a thread other than the one that started it; nothing changed, and the call may be made again from the right thread. |
| Out-buffers | On `>= 0` the host copies the buffer and frees it with `<prefix>_buf_free(ptr, len)`; on `< 0` nothing was written. |
| `start` | Valid once per handle; a second call is `BAD_INPUT`. Returns the effects recorded before the first wait. |
| `reply` | `(ptr, len)` is exactly one reply record: `kind · id · payload`, where kind is `1 str`, `2 u64`, `3 unit`, `4 bytes` — typed by reply kind, not by effect. Trailing bytes are `MALFORMED`. Returns the effects recorded before the next wait. |
| `resume` | Poll without a reply: returns the effects recorded before the next wait. For an `IDLE` routine, once something it waits on may have changed — a message sent by another routine. Resuming when nothing has changed is harmless: the routine suspends again and the batch is empty. A host learns when to from `woke` frames (see [Frames](#frames)); it may also simply resume its `IDLE` machines whenever it has nothing else to do. |
| `wakes` | Returns only `woke` frames: the machines woken outside any call — a receiver whose sender was dropped by `free`, or by a panic — since the last `wakes`. Always `OK`. A host with no calls to make, no requests outstanding, and nothing from `wakes` has reached the end, or a stall. |
| Effects | Little-endian; `str` and `bytes` are `u32 len` + payload. One frame per effect, frames concatenated: `u8 kind · u32 len · payload`, where the payload is the routine's record — one `u8` tag, its fields, and, for an ask, its `u64` request id last. See [Frames](#frames). |
| Request ids | Per machine, from `1`, gapless, in the order effects are recorded: each id a host sees is one more than the last. An id is assigned when its effect is recorded, so a request the routine drops before recording it never gets one. Any number may be outstanding at once, and the host may reply in any order. A reply to an id at or below the highest seen that is no longer outstanding — answered, or reported in a closed frame — is `STALE`; to an id above it, `BAD_INPUT`. |
| Upcalls | None. The host calls in; the routine never calls out. No callback is registered, no host value is held on the Rust side. |
| Threading | Per machine. A _migrating_ machine may be driven from any thread, one at a time per handle: two threads driving one handle get `BUSY`, not a race; a host may pool, and its calls migrate between threads. A _pinned_ machine is bound to the thread that calls its `start`: every later call for it must come from there, and from any other is `WRONG_THREAD`. Machines a binding constructs are migrating unless it documents otherwise; a spawned child's kind is named by the routine's own tag table. |
| Spawned machines | A routine may create machines. Each reaches the host as an effect naming its new handle, per the routine's tag table. The host must `start` or `free` every handle it is given, as it must every handle from `new`. |
| Panics | A routine that panics is removed; the call returns `PANICKED` and every later call on that handle is `BAD_HANDLE`. The process is not aborted. Every request it had outstanding is closed, without a closed frame; so is every request of a machine that is freed. |

## Frames

Each effect, and each closed request, crosses as one frame: `u8 kind · u32 len · payload`. An effect's payload is the routine's own record, laid out as its tag table says; the frame around it is the ABI's, so a host can split a batch into records without knowing any tag table.

| Frame kind | Holds                                                                                          | The host…                                                                              |
|------------|------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------|
| `1` tell   | an effect that awaits no reply                                                                 | performs it; skips it if it does not know the tag                                      |
| `2` ask    | an effect that awaits a reply; its payload ends with the request id                            | performs it and replies; refuses to continue if it does not know the tag — nobody else will answer |
| `3` closed | one `u64` request id the routine abandoned: a race's losing branch, or anything outstanding at completion | may stop that work; a reply to it is `STALE`                                           |
| `4` woke   | one `u64` machine handle: that machine may be able to progress — something it waits on inside the process changed, such as a channel this call's routine sent on | `resume`s it when it chooses; it may be this machine, or any other, and it may have been freed since (`BAD_HANDLE`) |
| other      | reserved for records a later revision adds                                                     | skips the frame                                                                        |

Closed frames follow the step's other frames, and woke frames follow those. A woke frame names a machine at most once per wait: until that machine is next polled, further wakes are not reported again. Correctness never depends on woke frames — resuming an idle machine is always harmless — so a host that ignores them and resumes idle machines some other way is merely slower. A closed id means the routine no longer needs the reply, not that the effect did not happen: the host may stop the work, but the effect may already be under way.

A host should check that parsing a payload used exactly `len` bytes. A mismatch means its copy of the tag table disagrees with the routine's, and it is caught at the record where it happens rather than corrupting everything after it.

Every length — a frame's `len`, and the length prefix of a `str` or `bytes` field — is a `u32`, so no frame or field exceeds 2³² − 1 bytes. Data larger than that crosses as several effects, not one. A routine that tries to encode more panics, and the call returns `PANICKED`; the process is not aborted.

## Versioning

`<prefix>_abi_version()` returns the revision of this document the binding implements; this text is revision `0`: pre-release, nothing published yet. A host checks it once, before `new`, and refuses to continue on a mismatch. The number is independent of the crates' versions: the ABI is meant to outlive them.

It changes when a host written against the previous revision could misbehave against a binding written against the new one — a new code, a new reply kind, a change to a record's layout or to a call's signature. It does not change for what the table already leaves to the binding — a new constructor, a routine's tag table — nor for a new frame kind, which older hosts skip. A binding may version its own vocabulary however it likes (`<prefix>_schema_version()` is a reasonable convention); that is not this number.

## The Reply Menu

Four kinds, and no more. A richer reply crosses as `bytes` and is decoded on the routine's side.

| Reply kind | Kind | Payload | Typical use |
|-----------|------|---------|-------------|
| `1` | `str` | `u32 len` + UTF-8 | text |
| `2` | `u64` | 8 bytes LE | counts, ids, timestamps, lengths |
| `3` | `unit` | — | a sleep, an ack |
| `4` | `bytes` | `u32 len` + bytes | anything else |

Replying with the wrong kind for an id is `WRONG_KIND`, and the request stays outstanding, so the host may retry with the right kind. A reply to an id no longer awaited is `STALE`; to an id never issued, `BAD_INPUT`.

## What Is Not in the ABI

And therefore lives with each routine:

- _The tag table_ — which `u8` means which effect, what fields follow it, and which reply kind answers it. The boundary crate's `Encode` impl is the source of truth; `demo/boundary` documents its five tags on the `View` enum.
- _The world_ — what performing an effect means: where `WriteLine` goes, what `Lookup` consults, whether `Sleep` is real or virtual.
- _The host loop_ — the host's own. `demo/python/main.py` is the smallest: it performs each effect, then replies. `demo/java/Main.java` is shaped like a production host: each ask on its own virtual thread, replies in whatever order the effects finish, closed frames cancel the thread, and one driver thread makes every call.

## A Conversation

The greeter in `demo/`, driven from start to the end of its input. Each frame is written `tell[…]` or `ask[…]`, with its length left out. `ReadLine` is fallible, so its replies are `bytes` holding an encoded `Result` — `⟨00 "alice"⟩` is `Ok("alice")`, `⟨01 00⟩` is `Err(Closed)`:

```text
host → start(h)
     ← tell[05 "Who are you?"] · ask[03 id=1]        AWAITING     WriteLine, ReadLine·1
host → reply(h, [04 id=1 ⟨00 "alice"⟩])                  bytes: Ok("alice")
     ← ask[02 "alice" id=2]                          AWAITING     Lookup·2
host → reply(h, [01 id=2 "Hello"])                       str
     ← ask[04 millis=50 id=3]                        AWAITING     Sleep·3
host → reply(h, [03 id=3])                               unit
     ← tell[05 "Hello, alice!"] · ask[01 id=4]       AWAITING     WriteLine, Count·4
host → reply(h, [02 id=4 1])                             u64
     ← tell[05 "(greeted 1 so far)"] · tell[05 "Who are you?"] · ask[03 id=5]
                                                     AWAITING
host → reply(h, [04 id=5 ⟨01 00⟩])                       bytes: Err(Closed)
     ← tell[05 "Bye."]                               COMPLETE
host → free(h)                                       OK
```

A `Quiet` machine (`<prefix>_new_ticker()` in the demo) is driven by the same loop and only ever produces tags 4 and 5 — the routine's trait bounds guarantee it, and the host can rely on it.

## Provenance

This descends from `hosts/effects/ABI.md` in [`effect-routines-exploration`](https://tangled.org/expede.wtf/effect-routines-exploration), which eight hosts in five languages implemented. It is not compatible with it and does not try to be: the thread-affine export set is gone, `start` is its own call rather than a tag-`0` input record, `u32` is `u64`, and trailing bytes are rejected. This document is the contract; the exploration is where it was tested.
