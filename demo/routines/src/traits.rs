//! The demo's own effect traits. `Count` and `Lookup` are specific to the
//! greeter; `Sleep`, `ReadLine`, and `WriteLine` come from
//! `sans-effort-effects`, the standard library, because nearly every routine
//! wants them. One trait per verb, each one thing a routine can do to, or
//! wait on from, its environment.
//!
//! Laid out like a module of the standard library: one module per trait,
//! holding the trait, its reifying impl, and its effect (in `effect`). The
//! impl is written over [`AsCtx`](sans_effort::ctx::AsCtx), so it holds for
//! [`Ctx`](sans_effort::ctx::Ctx) and for any newtype wrapping one. The
//! routines themselves use only the traits.
//!
//! Methods are spelled `fn … -> impl Future<Output = T> + Send`;
//! implementors write `async fn`. Declaring `Send` is what lets a routine
//! generic over its context prove a child it spawns is `Send`. The price is
//! that every context's futures must be `Send`, so an `Rc`-based context
//! cannot implement these traits — pinned in `tests/send.rs`.

pub mod count;
pub mod lookup;
