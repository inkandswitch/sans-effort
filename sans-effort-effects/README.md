# sans-effort-effects

> A standard library of common capabilities for `sans-effort` routines

A routine names what it needs as traits; a context decides whether each call is a real future or an effect recorded for a host. Most routines want the same few things, and without this crate every application writes each of them three times: the trait, the effect and its reifying impl, and a native impl. Here the first two are written once.

| Module    | Traits      | Fallible?                       |
|-----------|-------------|---------------------------------|
| `time`    | `Sleep`     | No                              |
| `console` | `WriteLine` | No                              |
| `console` | `ReadLine`  | `Result<String, ReadLineError>` |

Each module holds the trait, its effect structs (`effect::…`), and the trait's impl for the reifying context `Ctx<E>`. `Ctx` is generic over the host's vocabulary `E`, so one impl serves every application: an application writes its vocabulary enum with a `From` impl per effect it offers, and `Ctx<E>` implements exactly the traits that vocabulary can carry. An application's own capabilities implement their traits the same way, through `Ctx::request` and `Ctx::notify`.

The impls are written over `AsCtx` — anything that can be viewed as a `Ctx` — which `Ctx<E>`, references, `Box`, `Rc`, and `Arc` implement. A newtype over `Ctx` implements it with one method and gets every capability, which is how a crate reifies a capability trait it does not own: the orphan rule forbids `impl TheirTrait for Ctx<E>`, but allows it on a local newtype. Nothing else needs `AsCtx`: routines name capabilities, and native contexts implement them directly.

A trait returns `Result` exactly when its effect can fail for reasons outside the routine. A fallible effect's reply crosses as `bytes`, encoded with `sans-effort`'s convention for `Result`.

This crate is `no_std` + `alloc`. Its `std` and `spin` features only forward `sans-effort`'s choice of lock; a crate that names the traits and nothing else should depend with `default-features = false`.

## License

MIT or Apache-2.0, at your option.
