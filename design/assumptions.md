# Assumptions

> [!NOTE]
> _Status:_ the host, routine, and target assumptions hold today. The address-space assumptions, and the parts about spawning and channels, describe planned work (see [`channels`](channels.md) and [`capabilities`](capabilities.md)).

This document lists what `sans-effort` assumes about its environment. Things the implementation _enforces_ — one thread per handle at a time (`BUSY`), replies of the right kind (`WRONG_KIND`), no reply to an abandoned or answered id (`STALE`), a well-formed reply record (`MALFORMED`), `Send` checked where a routine is built — are not assumptions and are not listed. What is listed can be violated, and says what happens if it is.

## Hosts

### The Host Is Honest

> [!IMPORTANT]
> _Assumption:_ the host performs effects faithfully and schedules fairly. It answers each request with a true reply, resumes or frees every child a routine spawns, keeps each pinned machine on the thread that started it, and resumes idle machines eventually.

The host owns the table of machines, performs every effect, and decides when each machine runs, so it is trusted by construction — the same position as a vat in E. Nothing in the design protects routines from their host, and no object-capability system claims to. What the discipline does protect against is one routine exceeding the effects its context grants or the channel ends it holds; see [`capabilities`](capabilities.md).

_Consequence of violation:_ a routine can be told anything, or never run again; a machine that never resumes holds its messages forever.

### The Host Eventually Replies or Frees

> [!IMPORTANT]
> _Assumption:_ every outstanding request is eventually answered, or the machine is freed.

The mechanism has no timeouts. A routine awaiting a reply that never comes waits forever, holding its slot. A routine that wants a timeout asks for one: it races the wait against a `Sleep` (see [`cancellation`](cancellation.md)).

_Consequence of violation:_ the routine never finishes and its machine is never reclaimed.

### The Host Checks the ABI Revision

> [!IMPORTANT]
> _Assumption:_ a foreign host calls `<prefix>_abi_version()` once, before `new`, and refuses to continue on a mismatch.

Record layouts, codes, and reply kinds are fixed by `ABI.md` alone. Nothing in the bytes says which revision produced them.

_Consequence of violation:_ a host built against one revision silently misparses a binding built against another.

## Routines

### Every Wait Is One the Process Can End

> [!IMPORTANT]
> _Assumption:_ under a driver, a routine awaits only futures its context produces, or in-process primitives — channels, locks — that other routines complete.

A driver's waker records wakes at most; it drives no reactor. A future that needs a runtime's reactor — `tokio::time::sleep`, a socket — never completes when a driven routine awaits it. A channel is fine: another machine's send completes it, and the host resumes the waiting machine.

_Consequence of violation:_ the machine stays `IDLE` for ever, and the host's stall check eventually reports it; under `testing::run_now`, a panic that names the cause.

### Routines Are Deterministic Given Their Replies

> [!IMPORTANT]
> _Assumption:_ a routine's effects depend only on its construction arguments and the replies it has received.

This is what makes the native and driven runs agree byte for byte, and what makes replay possible: re-running a routine and feeding it the recorded replies reproduces its effects. Reading a clock or a random source directly, instead of asking the context, breaks it.

_Consequence of violation:_ transcripts diverge between hosts; replay fails.

### Routines Do Not Block

> [!IMPORTANT]
> _Assumption:_ a step returns promptly; waiting happens only at `.await` points on the context.

A routine that blocks the thread inside a step blocks whoever is polling it — the tokio worker, or the host's call into the binding.

_Consequence of violation:_ the host (and, under tokio, every task on that worker) stalls.

### Routines Are Safe Rust

> [!IMPORTANT]
> _Assumption (planned):_ routine crates are `unsafe_code = "forbid"`.

A channel end is an object reference, and an effect is reached only through the context a routine was given: safe code can use only what it was given. `unsafe` code can read any memory, including the ends other routines hold.

_Consequence of violation:_ every guarantee between routines is void. See [`capabilities`](capabilities.md) for the design that would cover untrusted guests.

## Address Space

### A Binding's Machines Share One Address Space

> [!IMPORTANT]
> _Assumption (planned):_ every machine created through one binding lives in one process, in one Rust world.

Machine handles mean something only there; channel ends are memory in it; a spawned child is built in its parent's process. This costs nothing a running routine had: a routine is a pinned future and can never leave its process anyway. See [`channels`](channels.md#spawning).

_Consequence of violation:_ not reachable today. Routines spanning processes would need a translation layer between hosts; see [`capabilities`](capabilities.md#across-hosts).

### Request Ids Are Minted Deterministically

> [!IMPORTANT]
> _Assumption:_ request ids are minted per machine, from `1`, increasing, at the moment their effect is recorded.

Replaying a routine re-mints the same ids, so a recorded transcript still lines up. Where routines exchange messages, the host's `resume`s are part of the record too: messages bypass the host, so their order follows its schedule. Minting when a request is recorded — when it is awaited, or when `join`, `select`, or `poll_once` start it — rather than when `ask` is called also makes them gapless: a request dropped before it was awaited never takes a number.

_Consequence of violation:_ replay cannot match recorded replies to requests.

## Targets

### Atomics and an Allocator

> [!IMPORTANT]
> _Assumption:_ the target has `alloc`, and either native CAS atomics or a `critical-section` implementation for `portable-atomic`.

_Consequence of violation:_ the crate does not build; the feature check says which feature is missing.

## What We Don't Assume

| Non-assumption                         | What the design does instead                                               |
|----------------------------------------|----------------------------------------------------------------------------|
| Replies arrive in order                | Requests carry ids; any number may be outstanding, answered in any order   |
| The clock is real                      | `Sleep` is a request; the host may answer at once, after a delay, or virtually |
| The host is written in Rust            | Foreign hosts see bytes and `u64` ids, never Rust values                   |
| The host is single-threaded            | Any thread may drive a handle, one at a time                               |
| A routine never panics                 | A panicking routine is removed and reported as `PANICKED`; the process lives |
| The routine knows how it is run        | It asks for traits; native and driven contexts are indistinguishable to it |
