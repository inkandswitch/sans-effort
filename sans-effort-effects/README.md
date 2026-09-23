# sans-effort-effects

> A standard library of common capabilities for `sans-effort` routines

A routine names what it needs as traits; a context decides whether each call is a real future or an effect recorded for a host. Most routines want the same few things, and without this crate every application writes each of them three times: the trait, the effect and its reifying impl, and a native impl. Here the first two are written once.

| Module    | Traits      | Fallible?                       |
|-----------|-------------|---------------------------------|
| `time`    | `Sleep`     | No                              |
| `console` | `WriteLine` | No                              |
| `console` | `ReadLine`  | `Result<String, ReadLineError>` |

Each module holds the trait, its effect structs (`effect::…`), and the trait's impl for `Ctx<E>`, the reifying context. `Ctx` is generic over the host's vocabulary `E`, so one impl serves every application: an application writes its vocabulary enum with a `From` impl per effect it offers, and `Ctx<E>` implements exactly the traits that vocabulary can carry. An application's own capabilities implement their traits for the same `Ctx` through `Ctx::request` and `Ctx::notify`.

A trait returns `Result` exactly when its effect can fail for reasons outside the routine. A fallible effect's reply crosses as `bytes`, encoded with `sans-effort`'s convention for `Result`.

This crate is `no_std` + `alloc`. Its `std` and `spin` features only forward `sans-effort`'s choice of lock; a crate that names the traits and nothing else should depend with `default-features = false`.

## License

MIT or Apache-2.0, at your option.
