# Design

This directory explains how `sans-effort` works and why, less formally than a specification. [`ABI.md`](../ABI.md) at the repository root is the normative contract for foreign hosts; these documents are the reasoning around it.

Some documents describe what exists and some describe what is planned. Each one says which at the top.

## Documents

| Document                          | Status            | Purpose                                                                        |
|-----------------------------------|-------------------|--------------------------------------------------------------------------------|
| [`assumptions`](assumptions.md)   | current + planned | What the design assumes about hosts, routines, and targets                     |
| [`effects`](effects.md)           | current + planned | A standard library of effect traits: `sans-effort-effects`, `sans-effort-tokio` |
| [`channels`](channels.md)         | planned           | Plain channels between machines, spawning, and the host as the scheduler       |
| [`capabilities`](capabilities.md) | planned           | Object-capability discipline within a process, given an honest host            |
| [`cancellation`](cancellation.md) | current           | `select`, abandoned requests, and telling the host                             |

## Layers

```mermaid
flowchart TB
    subgraph host["host"]
        direction LR
        native["tokio · the JS event loop<br/>polls the task directly, no driver"]
        foreign["Python · Java · a test<br/>a Driver polls; the host performs effects and replies by id"]
    end

    subgraph context["context"]
        direction LR
        tokio_ctx["TokioCtx<br/>each call is a real future"]
        reify_ctx["Ctx#60;E#62;<br/>each call records an effect and suspends"]
    end

    subgraph routine["routine · no_std"]
        greeter["Greeter#60;C: Sleep + Lookup + ReadLine + WriteLine#62;: Step"]
    end

    native --> tokio_ctx --> greeter
    foreign --> reify_ctx --> greeter
```

## Crates

```mermaid
flowchart BT
    facade["sans-effort<br/>re-exports only · no_std"]
    core["sans-effort-core<br/>mechanism · no_std"]
    host_crate["sans-effort-host<br/>Machine · Encoded · table · std"]
    effects["sans-effort-effects<br/>traits · requests · Ctx#60;E#62; · no_std"]
    tokio_crate["sans-effort-tokio<br/>TokioClock · TokioInput · TokioOutput · TokioCtx · std"]
    binding["an application's binding<br/>extern C · the only unsafe"]

    facade --> core
    facade --> effects
    facade -. feature tokio .-> tokio_crate
    facade -. feature host .-> host_crate
    host_crate --> core
    effects --> core
    tokio_crate --> effects
    binding --> host_crate

    classDef planned stroke-dasharray: 5 5
```

The mechanism crate is meant to be small and close to frozen; everything opinionated lives in a crate above it, so it can change without breaking the mechanism. The `sans-effort` crate holds no code: it re-exports the others so a routine author needs one dependency, while runtime and binding authors depend on the crates beneath it directly.

## Typical Flow

A foreign host driving one routine. Each call returns the effects the routine recorded before its next wait.

```mermaid
sequenceDiagram
    participant H as Host (Python)
    participant R as Routine

    H->>R: abi_version()
    R-->>H: 0
    H->>R: new() → handle
    H->>R: resume(handle)
    R-->>H: [WriteLine "Who are you?", (ReadLine, 1)] · AWAITING
    H->>R: reply(handle, 1, "alice")
    R-->>H: [(Lookup "alice", 2)] · AWAITING
    H->>R: reply(handle, 2, "Hello")
    R-->>H: [WriteLine "Hello, alice!"] · COMPLETE
    H->>R: free(handle)
```

## Design Principles

- _Direct style._ A routine is an ordinary `async fn`. The compiler writes the state machine.
- _Traits, not effects._ A routine names what it needs as effect traits. A context decides whether each call is a real future or a recorded effect.
- _Pull-only._ The host calls in; the routine never calls out. No callbacks, no upcalls, no foreign value in a Rust frame.
- _The host is the scheduler._ Whichever host polls — tokio, Node, a Python loop, a test — decides what runs, when, and in what order replies arrive. The library contains no scheduler.
- _A small, stable mechanism._ Opinions (which effect traits exist, how they run on tokio) live in separate crates with their own versions.
- _`no_std` core._ The mechanism, routines, and their boundary crates build for `wasm32` and `thumbv6m`.
- _`unsafe` only in bindings._ Everything but the per-application binding is `unsafe_code = "forbid"`.
- _Transcripts are data._ What crossed the boundary can be written down, compared byte for byte, and replayed.
