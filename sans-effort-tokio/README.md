# sans-effort-tokio

> Native tokio contexts for `sans-effort` routines

Every capability in `sans-effort-effects` as a real tokio future, so a routine runs as an ordinary task — `tokio::spawn(routine.run())` — with no driver and no host.

| Type                   | Implements       |
|------------------------|------------------|
| `clock::TokioClock`    | `Sleep`          |
| `console::TokioInput`  | `ReadLine`       |
| `console::TokioOutput` | `WriteLine`      |
| `ctx::TokioCtx`        | all of the above |

`TokioCtx` is the ready-made context: one value with every capability, built from the components, with `TokioCtx::stdio()` for standard input and output. There is one component per capability, so a context can be composed to grant no more than a routine needs — a clock and output for a routine that never reads, or a paused clock with an in-memory output in a test.

An application with capabilities of its own wraps a `TokioCtx` in a type it owns, implements its traits there, and forwards the stdlib ones in one line each. The orphan rule requires the impls to live on a type the application owns.

`WriteLine` is fire-and-forget, so a failed write has nowhere to go: the first failure is logged with `tracing`, and later lines are dropped.

## License

MIT or Apache-2.0, at your option.
