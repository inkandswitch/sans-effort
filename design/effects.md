# A standard library of effects

> [!NOTE]
> _Status:_ `sans-effort-effects` exists with `time`, `console`, `Ctx`, and `AsCtx`; `sans-effort-tokio` exists with `TokioClock`, `TokioInput`, `TokioOutput`, and `TokioCtx`; `Decode` and the `Result`/`Option` encoding are in core. `channel` and its tokio side are planned.

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
| `sans-effort-effects` | Per module: the trait, its effect structs (`effect::…`), and the reifying `Ctx<E>` impl. No tags     | `no_std` |
| `sans-effort-tokio`   | One component per capability — `TokioClock` (`Sleep`), `TokioInput<R>` (`ReadLine`), `TokioOutput<W>` (`WriteLine`) — and `TokioCtx<R, W>` built from them; later, a channel registry           | `std`    |

Modules in `sans-effort-effects`:

| Module    | Traits                                        |
|-----------|-----------------------------------------------|
| `time`    | `Sleep`                                       |
| `console` | `ReadLine`, `WriteLine`                       |
| `channel` | `Open`, `Post`, `Receive`, `Spawn`, `Sender<M>`, and `Receiver<M>` — see [`channels`](channels.md) |

The vocabulary enum and its tags stay the application's. A host decides what it offers; the stdlib only makes the offer cheap to write.

## The reifying context ships once

`Ctx<E>` is generic over the vocabulary, so one impl per trait covers every application:

```rust
impl<E: From<Asked<time::effect::Sleep>>> time::Sleep for Ctx<E> {
    async fn sleep(&self, d: Duration) {
        self.request(time::effect::Sleep(d)).await;
    }
}
```

An application writes its vocabulary — eventually with a derive — and both interpreters already exist:

```rust
enum Full {
    Sleep(Asked<time::effect::Sleep>),
    ReadLine(Asked<console::effect::ReadLine>),
    WriteLine(console::effect::WriteLine),
    Lookup(Asked<effect::Lookup>),  // the application's own capability
}
```

Each module names its effect structs in an `effect` submodule: `time::Sleep` is the trait, `time::effect::Sleep` what it records.

### Application capabilities live with their traits

An application's own capabilities implement their traits the same way, through `Ctx::request` and `Ctx::notify` — enough to add a capability without handing out the outbox. The orphan rule allows `impl Lookup for Ctx<E>` only in the crate that defines `Lookup` or the one that defines `Ctx`, so the impl lives with the trait: an application lays out its capabilities the way the stdlib lays out a module — trait, effect, and reifying impl together — and its vocabulary crate holds only the vocabularies. In the demo, `routines::traits` holds `Count` and `Lookup` with their effects and impls; the routines themselves still use only the traits.

### `AsCtx`: newtypes over `Ctx`

Every reifying impl — the stdlib's and, by convention, an application's — is written over `AsCtx`, not `Ctx<E>` itself:

```rust
pub trait AsCtx { type Vocabulary; fn ctx(&self) -> &Ctx<Self::Vocabulary>; }

impl<C: AsCtx> Sleep for C where C::Vocabulary: From<Asked<time::effect::Sleep>> { … }
```

`Ctx<E>` implements `AsCtx`, and so do references, `Box`, `Rc`, and `Arc` to anything that does. A newtype over `Ctx` implements it with one method and gets every capability — which is how a crate reifies a capability trait it does not own: the orphan rule forbids `impl TheirTrait for Ctx<E>`, but allows it on a local newtype. Without `AsCtx`, that newtype would have to forward every stdlib trait by hand.

It is opt-in. Routines name capabilities; `Ctx<E>` already implements `AsCtx`; native contexts implement capabilities directly and are unaffected. The price: a type that implements `AsCtx` cannot also implement a stdlib capability by hand, and the stdlib cannot add blanket forwarding impls for `&T` or `Box<T>` on its traits — those would overlap. Forwarding through `AsCtx` for references and smart pointers covers the same ground for reifying contexts.

### Native contexts forward

The native side has no equivalent of `AsCtx`. Blanket impls of the stdlib's traits can only live in the stdlib crate, and the stdlib's blanket is already over `AsCtx`. So a native context that needs capabilities of its own — the demo's `Count` and `Lookup` on tokio — is a type the application owns, wrapping `TokioCtx` and forwarding the stdlib capabilities in one line each:

```rust
struct DemoCtx<R, W> { tokio: TokioCtx<R, W>, greeted: AtomicU64 }

impl<R, W> Sleep for DemoCtx<R, W> {
    async fn sleep(&self, d: Duration) { self.tokio.sleep(d).await }
}
// … ReadLine, WriteLine likewise; Count and Lookup directly.
```

`sans-effort-tokio` offers one component per capability (`TokioClock`, `TokioInput`, `TokioOutput`) as well as the assembled `TokioCtx`, so a context can take just what it needs — a clock and output for a routine that never reads, or a paused clock with an in-memory output in a test. References and smart pointers do not forward capabilities for native contexts (that forwarding goes through `AsCtx`), so a routine owns its native context; a test that wants the output back shares the _writer_, not the context. If forwarding grows painful as the stdlib grows, a forwarding macro or `#[capability]`-style generation is the next step.

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

- _Channel messages._ Nearly always enums with fields, often carrying senders:

  ```rust
  enum Counter { Incr(u64), Get { reply_to: channel::Sender<u64> } }
  ```

- _Structured replies to ordinary requests._ `Stat(path) → Metadata`, `HttpGet(url) → Response`, `Lookup(name) → Option<String>` — there is no reply kind for "absent".
- _Fallible effects._ Every effect so far is infallible from the routine's side. Real ones fail: a missing file, a reset connection. The honest signature is `async fn read(&self, path) -> Result<Vec<u8>, IoError>`, and a `Result` can only cross as `bytes`.

### All channel messages are bytes on the wire

A message that is just a `String` could, in principle, cross as the `str` kind. It does not, because the host routes messages without looking at them. If a body could be any of four kinds, the host would have to know which kind each receiver expects. With every body as `bytes`, routing is `post(channel, body) → reply(receive_id, body)`, whatever the types.

The simple cases cost nothing: core implements `Encode` and `Decode` for `String`, `u64`, `()`, and `Vec<u8>`. Under tokio none of this happens: messages are values in a channel.

## Where this sits

The capabilities style is the _tagless-final_ style with the representation fixed to `impl Future`. The traits are the algebra, a routine is a term abstract in its interpreter, `TokioCtx` is the evaluating interpreter, and `Ctx<E>` is the reifying one — it recovers the initial, tagged encoding (the vocabulary enum). A smaller vocabulary is a smaller algebra. This is why the effects style (a routine emitting a tagged enum) and the capabilities style share one mechanism: initial and final encodings convert into each other.

## Fallible where the world can fail

A trait returns `Result` exactly when its effect can fail for reasons outside the routine. `read_line` can: input ends, or the stream breaks. Today the demo fakes end of input by answering `"quit"`, which is a protocol hidden in a string. `sleep` cannot fail in any way a routine could act on, so it stays infallible.

```rust
pub trait ReadLine { async fn read_line(&self) -> Result<String, ReadLineError>; }
pub trait Sleep    { async fn sleep(&self, d: Duration); }
```

Each module has its own small error enum, marked `#[non_exhaustive]` so a new failure is not a breaking change. On the wire a fallible reply crosses as `bytes`. Core fixes one encoding for `Result<T, E>` and `Option<T>` — a `0` or `1` tag, then the value — so every host writes them the same way.

## Open questions

- Which modules beyond `time`, `console`, and `channel`: random numbers? logging? A file system?
