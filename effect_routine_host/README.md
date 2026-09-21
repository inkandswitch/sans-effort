# effect_routine_host

> The host side of an effect routine, for hosts that cannot hold a Rust value

A typed `Machine` over any `Drive`, a byte layer that decodes one input
record and encodes the resulting effects, a handle table steppable from any
thread, and panic isolation. Safe Rust throughout; the application adds the
`extern "C"` skin. `ABI.md` at the repository root is the contract.

See the [workspace README](../README.md).
