# A standard library of effects

> [!NOTE]
> _Status:_ planned. Today the five capabilities below live in `demo/`.

`Sleep` and console I/O are things nearly every routine wants, and messaging other routines is close behind. Today a user who wants `Sleep` writes it three times:

```text
  the trait                     trait Sleep { async fn sleep(&self, d: Duration); }
  the request + reifying impl   struct Sleep(Duration): Request
                                impl<E: From<Asked<Sleep>>> Sleep for Ctx<E>
  the native impl               impl Sleep for TokioCtx
```

The demo already holds all three rows for five capabilities, filed under `demo/`. Only two of them (`Count`, `Lookup`) are specific to the demo. This document is about moving the rest out, so they are written once.

## Layout

The shape is `embedded-hal`'s: one crate of traits, implementations in separate crates, and applications generic over the traits.

| Crate                 | Contents                                                                                               | Target   |
|-----------------------|--------------------------------------------------------------------------------------------------------|----------|
| `sans-effort`         | The mechanism, unchanged, plus `Decode` and `select`                                                    | `no_std` |
| `sans-effort-effects` | Per module: the trait, its request struct(s), and the reifying `Ctx<E>` impl. No tags                  | `no_std` |
| `sans-effort-tokio`   | `TokioCtx`, implementing every trait in `sans-effort-effects` natively; the actor `Registry`           | `std`    |

Modules in `sans-effort-effects`:

| Module    | Traits                                        |
|-----------|-----------------------------------------------|
| `time`    | `Sleep`                                       |
| `console` | `ReadLine`, `WriteLine`                       |
| `actor`   | `Post`, `Receive`, `Spawn`, `Me`, and `Address<M>` — see [`actors`](actors.md) |

The vocabulary enum and its tags stay the application's. A host decides what it offers; the stdlib only makes the offer cheap to write.

## The reifying context ships once

`Ctx<E>` is generic over the vocabulary, so one impl per trait covers every application:

```rust
impl<E: From<Asked<time::Sleep>>> time::Sleep for Ctx<E> {
    async fn sleep(&self, d: Duration) {
        self.request(time::Sleep(d)).await
    }
}
```

An application writes its vocabulary — eventually with a derive — and both interpreters already exist:

```rust
enum Full {
    Sleep(Asked<time::Sleep>),
    ReadLine(Asked<console::ReadLine>),
    WriteLine(console::WriteLine),
    Lookup(Asked<Lookup>),          // the application's own capability
}
```

An application's own capabilities implement their traits for the same `Ctx<E>`. That needs `Ctx` to expose `request` and `notify` — enough to add a capability, without handing out the outbox.

## Why a crate, and not a feature

The traits are small, `no_std`, and have no dependencies, so the usual reason for a separate crate — keeping heavy things out — does not apply. Three other reasons do:

- _Semver._ The mechanism should be close to frozen: `Run`, `Driver`, `ReplyHandle`, and the reply menu have not moved since they were first built. A library of traits is the opposite. It grows modules, and signatures get argued over. `embedded-hal`'s traits, not its mechanism, took a breaking release to settle. A change to `actor::Spawn` must not force a major release of the mechanism.
- _The mechanism stays opinion-free._ "The library is the mechanism" remains literally true of `sans-effort`, and this stdlib sits on the same footing as anyone else's.
- _The tokio crate must be separate anyway._ A `tokio` feature on the `no_std` crate would, by feature unification, pull `std` into every crate in a workspace that enables it — including `thumbv6m` builds.

## Why "effects", and not "caps"

"Caps" reads as _object capabilities_. The design is object-capability-shaped, but the name would promise more than bytes on a wire can deliver (see [`capabilities`](capabilities.md)). The crate holds effect definitions — the request structs — as well as the traits, so "effects" is accurate. "Capabilities style" stays the name of the _style_.

## `Decode`

`Encode` writes a host's view of an effect as bytes. Until now nothing needed the reverse: replies were one of four kinds (`str`, `u64`, `unit`, `bytes`) and the host layer read them by hand. `Decode` is the reverse:

```rust
pub trait Decode: Sized {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError>;
}
```

It returns `Result` rather than `Option`: parse, don't validate. `Reader`'s primitive methods stay `Option` and lift with `?`. With both directions in place, `decode(encode(x)) == x` becomes a property test for every message type.

### When the four reply kinds are not enough

`ABI.md` already says "a richer reply crosses as `bytes` and is decoded on the routine's side". `Decode` gives that a type. It is needed for:

- _Actor messages._ Nearly always enums with fields, often carrying addresses:

  ```rust
  enum Counter { Incr(u64), Get { reply_to: Address<u64> } }
  ```

- _Structured replies to ordinary requests._ `Stat(path) → Metadata`, `HttpGet(url) → Response`, `Lookup(name) → Option<String>` — there is no reply kind for "absent".
- _Fallible effects._ Every effect so far is infallible from the routine's side. Real ones fail: a missing file, a reset connection. The honest signature is `async fn read(&self, path) -> Result<Vec<u8>, IoError>`, and a `Result` can only cross as `bytes`.

### All actor messages are bytes on the wire

A message that is just a `String` could, in principle, cross as the `str` kind. It does not, because the host routes messages without looking at them. If a body could be any of four kinds, the host would have to know which kind each receiver expects. With every body as `bytes`, routing is `post(to, body) → reply(receive_id, body)`, whatever the types.

The simple cases cost nothing: core implements `Encode` and `Decode` for `String`, `u64`, `()`, and `Vec<u8>`. Under tokio none of this happens: messages are values in a channel.

## Where this sits

The capabilities style is the _tagless-final_ style with the representation fixed to `impl Future`. The traits are the algebra, a routine is a term abstract in its interpreter, `TokioCtx` is the evaluating interpreter, and `Ctx<E>` is the reifying one — it recovers the initial, tagged encoding (the vocabulary enum). A smaller vocabulary is a smaller algebra. This is why the effects style (a routine emitting a tagged enum) and the capabilities style share one mechanism: initial and final encodings convert into each other.

## Open questions

- Should stdlib traits return `Result` (`read_line() -> Result<String, ConsoleError>`)? Changing this after release is exactly the churn the crate split absorbs, but it is still cheaper to decide first.
- Which modules beyond `time`, `console`, and `actor`: random numbers? logging? A file system?
