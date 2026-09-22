# Assumptions

> [!NOTE]
> _Status:_ the host, routine, and target assumptions hold today. The address-space and actor assumptions describe planned work (see [`actors`](actors.md) and [`capabilities`](capabilities.md)).

This document lists what `sans-effort` assumes about its environment. Things the implementation _enforces_ — one thread per handle at a time (`BUSY`), replies of the right kind (`WRONG_KIND`), no reply to an abandoned or answered id (`STALE`), a well-formed reply record (`MALFORMED`), `Send` checked where a routine is built — are not assumptions and are not listed. What is listed can be violated, and says what happens if it is.

## Hosts

### The host is honest

> [!IMPORTANT]
> _Assumption:_ the host routes faithfully. It delivers a posted message to the address it was posted to and nowhere else, answers `spawn` with a fresh handle and `me` with the truth, and does not invent introductions between actors.

The host owns the table of machines and does the routing, so it is trusted by construction — the same position as a vat in E. Nothing in the design protects routines from their host, and no object-capability system claims to. What the discipline does protect against is one routine exceeding its introductions; see [`capabilities`](capabilities.md).

_Consequence of violation:_ an actor can be handed any address, and every capability guarantee between actors is void.

### The host eventually replies or frees

> [!IMPORTANT]
> _Assumption:_ every outstanding request is eventually answered, or the machine is freed.

The mechanism has no timeouts. A routine awaiting a reply that never comes waits forever, holding its slot. A routine that wants a timeout asks for one: it races the wait against a `Sleep` (see [`cancellation`](cancellation.md)).

_Consequence of violation:_ the routine never finishes and its machine is never reclaimed.

### The host checks the ABI revision

> [!IMPORTANT]
> _Assumption:_ a foreign host calls `<prefix>_abi_version()` once, before `new`, and refuses to continue on a mismatch.

Record layouts, codes, and reply kinds are fixed by `ABI.md` alone. Nothing in the bytes says which revision produced them.

_Consequence of violation:_ a host built against one revision silently misparses a binding built against another.

## Routines

### Every wait goes through the context

> [!IMPORTANT]
> _Assumption:_ under a driver, a routine awaits only futures its context produces.

A `Driver` polls with a waker that does nothing. A native future — `tokio::time::sleep`, a channel — that reaches a driven routine will never be woken.

_Consequence of violation:_ the machine reports `STALLED`; under `testing::run_now`, a panic that names the cause.

### Routines are deterministic given their replies

> [!IMPORTANT]
> _Assumption:_ a routine's effects depend only on its construction arguments and the replies it has received.

This is what makes the native and driven runs agree byte for byte, and what makes replay possible: re-running a routine and feeding it the recorded replies reproduces its effects. Reading a clock or a random source directly, instead of asking the context, breaks it.

_Consequence of violation:_ transcripts diverge between hosts; replay fails.

### Routines do not block

> [!IMPORTANT]
> _Assumption:_ a step returns promptly; waiting happens only at `.await` points on the context.

A routine that blocks the thread inside a step blocks whoever is polling it — the tokio worker, or the host's call into the binding.

_Consequence of violation:_ the host (and, under tokio, every task on that worker) stalls.

### Actors are safe Rust

> [!IMPORTANT]
> _Assumption (planned):_ routine crates are `unsafe_code = "forbid"`.

An `Address` has a private constructor, so safe code can obtain one only by introduction. `unsafe` code can fabricate one.

_Consequence of violation:_ an actor can post to any address it can guess. See [`capabilities`](capabilities.md) for the design that would cover untrusted actors.

## Address space

### A binding's machines share one address space

> [!IMPORTANT]
> _Assumption (planned):_ every machine created through one binding lives in one process, in one Rust world.

Machine handles, and the tokens a by-value `spawn` parks, mean something only there. This costs nothing a running routine had: a routine is a pinned future and can never leave its process anyway. See [`actors`](actors.md#spawn).

_Consequence of violation:_ not reachable today. Actors spanning processes would need a translation layer between hosts; see [`capabilities`](capabilities.md#across-hosts).

### Tokens are minted deterministically

> [!IMPORTANT]
> _Assumption (planned):_ request ids and spawn tokens are minted per machine, from `1`, increasing.

Replaying a routine re-mints the same ids and tokens, so a recorded transcript still lines up.

_Consequence of violation:_ replay cannot match recorded replies to requests.

## Targets

### Atomics and an allocator

> [!IMPORTANT]
> _Assumption:_ the target has `alloc`, and either native CAS atomics or a `critical-section` implementation for `portable-atomic`.

_Consequence of violation:_ the crate does not build; the feature check says which feature is missing.

## What we don't assume

| Non-assumption                         | What the design does instead                                               |
|----------------------------------------|----------------------------------------------------------------------------|
| Replies arrive in order                | Requests carry ids; any number may be outstanding, answered in any order   |
| The clock is real                      | `Sleep` is a request; the host may answer at once, after a delay, or virtually |
| The host is written in Rust            | Foreign hosts see bytes and `u64` ids, never Rust values                   |
| The host is single-threaded            | Any thread may drive a handle, one at a time                               |
| A routine never panics                 | A panicking routine is removed and reported as `PANICKED`; the process lives |
| The routine knows how it is run        | It asks for traits; native and driven contexts are indistinguishable to it |
