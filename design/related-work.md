# Related Work

> [!NOTE]
> _Status:_ current. Comparisons are against `corophage` 0.6.0 and `effing-mad` 0.1.0, read from their source in 2026-09.

Several Rust libraries let direct-style code perform effects that something else handles. They differ on one question above all: _where the handler lives_. This document places `sans-effort` among them, so a reader who knows one can see what is the same and what is not.

## The Design Space

|                                              | Handler lives                                                                  | Who drives                  | Effects per computation at once | Routine source tied to the library       |
|----------------------------------------------|--------------------------------------------------------------------------------|-----------------------------|---------------------------------|------------------------------------------|
| `effing-mad`                                 | in the library's runner, as closures                                           | the runner                  | one                             | yes (`#[effectful]`, nightly coroutines) |
| `corophage`                                  | in the library's runner, as closures                                           | the runner                  | one                             | yes (`Yielder`, `yield_!`)               |
| IO-monad libraries (ZIO / Cats Effect style) | in the library's interpreter                                                   | the interpreter             | many, through its combinators   | yes (`IO<A>` / `Effect<A, E, R>` values) |
| `sans-effort`                                | outside: a host, possibly in another language — or no handler at all, natively | the host, one step per call | many, with ids                  | no (plain `async fn` over effect traits) |

## `corophage` and `effing-mad`

`corophage` is algebraic effects with handlers on stable Rust, the successor in spirit to `effing-mad` (which needs nightly coroutines). A body yields effect values through a `Yielder`; a `Program` collects one handler closure per effect; `run_sync()` or `run().await` drives the body to completion, calling a handler at each yield and resuming the body with its answer.

```text
corophage — the runner owns the loop; handlers are closures inside it

  run_sync() ──▶ poll body ──yield_(FileRead)──▶ dispatch on the effect's type ──▶ handler
       ▲                                                                            │
       └─────────────────── resume with the answer (same call stack) ───────────────┘
  ...until the body returns ──▶ Result<R, Cancelled>

sans-effort — the host owns the loop; each call is one step

  host ──resume()──▶ Driver polls the routine ──▶ [Ask#1, Ask#2, Tell] ──▶ host
  host ──reply(#2, v)──▶ Driver polls the routine ──▶ [...]           ──▶ host

  ...or no driver at all: the same routine on tokio or the JS event loop
```

### How Effects Leave the Future

`corophage` carries them through the `Waker`. Its runner polls the body with a waker whose data points at a slot for the yielded effect and the answer; `yield_` recognises that waker by its vtable and reads and writes the slot through a raw pointer (in `fauxgen`, which `corophage` builds on). It is fast and needs no handle in the body, but it relies on `unsafe` code, and any combinator that polls the body with a waker of its own — a select built from per-branch wakers, `FuturesUnordered`, a spawned task — cuts the channel. Its documentation says to keep effect operations in the directly awaited flow.

`sans-effort` carries them through an `Outbox` the routine holds, usually inside its context. An ask records itself there on its first poll and waits on a mail slot under its id. The waker is used only to wake, and a wake becomes a `woke` frame for the host. Any combinator works, and dropping an ask's future is its cancellation: the host is told the id closed.

### Concurrency Within One Computation

A `corophage` `Yielder` takes `&mut self` for each effect, so two effects cannot be outstanding together: a `join!` over two of them does not compile, and no request ids are needed. A `sans-effort` routine can have many asks outstanding — a `join` is one batch, answered in any order by id; a `select` closes its loser — and machines spawn one another and talk over channels, with the host scheduling them.

### Typing the Effect Set

|                       | `corophage`                                                                  | `sans-effort`                                                                                           |
|-----------------------|------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| A computation's needs | a type-level coproduct, `Effects![Log, FileRead]`                            | trait bounds, `C: ReadLine + Sleep`                                                                     |
| Every effect handled  | checked at `run`: the handler list must cover the coproduct                  | checked by trait resolution: a context implements a trait only if its vocabulary carries that effect    |
| Composition           | `invoke!` embeds a sub-program whose effects are a subset of the outer one's | ordinary Rust: pass the context to a function with fewer bounds                                         |
| Answers               | a generic associated type, so an answer may borrow (`&'r str`)               | owned values from a closed set of wire kinds, since an answer may arrive as bytes from another language |

### Handling, Formally

`corophage` is algebraic effects in the usual sense: every operation is reified, and a runner interprets it by dispatching to a handler. `sans-effort` is closer to tagless final, or capability passing: a routine is polymorphic over its context, and to handle an effect is to implement its trait. One interpreter — `Ctx` over an `Outbox` — reifies operations into requests, and the handler is outside the process boundary. Natively nothing is reified: `TokioCtx` returns tokio futures directly. `corophage` reifies always and handles inside; `sans-effort` reifies only at the boundary and handles outside.

### Stepping, Replay, and Cancellation

`corophage` has no public stepping interface. Its interactive debugger example blocks inside a handler, and steps back by re-running the program from the start while replaying recorded answers — the idea `sans-effort` provides as a tool, recording every call through the host table and replaying it. Cancellation in `corophage` is a handler's answer, `Control::cancel()`, which aborts the whole computation; in `sans-effort` it is per request (drop the future) or per machine (the host frees it).

### Cost

`corophage` reports roughly 10–14 ns per synchronously handled effect, with one allocation per run and none per dispatch. `sans-effort` measures roughly 120 ns per ask round trip, with one allocation per yield and several short lock acquisitions. The difference is what each ask carries here and not there: a lock that lets a driver move between threads, an id and a mail slot so several asks can be outstanding, tracking of closed ids, and a fresh list of effects handed to the host. The two measurements are from different machines, so read them as an order of magnitude, not a ratio.

### What Carries Over

- _Allocation:_ one per run and none per dispatch is a fair target for the per-yield list `sans-effort` allocates today.
- _Cancellation from the handler's side:_ `Control::cancel()` corresponds to a host freeing a machine.

What does not: the waker channel (it gives up "any combinator works" and needs `unsafe`), coproduct effect sets (trait bounds already compose), and borrowed answers (they cannot cross a language boundary).

## IO-Monad Libraries

Libraries in the ZIO and Cats Effect tradition — in Rust, `effect-rs`, `functype-io`, `id_effect`, and others — describe a program as a lazy value, `IO<A>` or `Effect<A, E, R>`, built from combinators or a do-notation macro and run later by the library's interpreter. In Rust, `async fn` is already a suspendable, effectful computation in direct style, so the monad adds a second way of sequencing without adding a boundary: the interpreter is still in-process, with no stepwise interface and no wire format. The environment parameter `R` — what the program requires — is what a `sans-effort` routine's context bounds already express.

## Ancestors

- _Sans-I/O protocol libraries_ (the Python sans-io pattern; in Rust, `quinn-proto` and `rustls`'s unbuffered API): a state machine that consumes input and emits what to do, owning no I/O. `sans-effort` applies the pattern to whole routines, with the compiler writing the state machine from an `async fn`.
- _Effekt:_ effect handlers via capability passing, where a computation receives the capability for each effect it performs. A `sans-effort` context is that capability.
- _Algebraic effects and handlers_ (Plotkin and Pretnar): the theory behind both `corophage` and the reifying context; a `Driver` is a handler whose clauses run in another process.
