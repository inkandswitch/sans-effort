# effect_routine

> Host-driven async coroutines whose every wait is a typed effect

The mechanism: `Run`, `Post` (`tell`/`ask`), `ReplyHandle<T>`, `Driver`,
`join`, the reply menu, and the `wire` traits an effect type implements to
be shown to a host that cannot hold a Rust value. `no_std` + `alloc`.

See the [workspace README](../README.md) and the crate docs.
