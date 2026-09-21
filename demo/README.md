# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — written
once against four capability traits, then run three ways that must agree
byte for byte.

```
  routines/  the routines, one per module: greeter (the conversation), fanout (two
             waits at once), ticker (needs only Clock + Output); traits.rs. no_std.
             Imports Run and join, nothing else.
             Tests: a Recording mock + testing::run_now — no driver, one poll.

  tokio/     the native path. TokioCtx implements the traits with tokio futures;
             tokio::spawn(Greeter::new(ctx).run()). No Driver anywhere.
             Tests: paused clock — three 50 ms pauses cost no wall time.

  wire/      the reifying context. Request structs (Lookup, ReadLine, …); Ctx<E>
             implements each trait for any E: From<Asked<…>>; Full carries all five,
             Quiet only Sleep + Write. View/HostEffect/Encode: the tag table.
             Tests: through a Driver, as data; Greeter under Quiet is a compile_fail.

  cdylib/    the C-ABI skin over effect_routine_host: new/start/reply/free/buf_free,
             three unsafe blocks, no mechanism. The only crate allowing unsafe.
  python/    a ctypes host that speaks ABI.md with a byte buffer and no library.

  wasm/      the wasm-bindgen skin: the generated class is the handle; holds a
             Machine directly; typed getters replace the codec.
  js/        a Node host over that module. pkg/ is generated.
```

Two kinds of foreign skin, on purpose. The C ABI is the _raw_ path: any
language that can `dlopen` and hand over bytes can drive the routine, and
the host writes its own decoder from `ABI.md`. wasm-bindgen is the
_generated_ path: no handle table, no codec, but a per-language toolchain.
The routine cannot tell which it is under — or that it is under anything.

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio          # native
cargo build -p greeter_cdylib && python3 demo/python/main.py       # C ABI
nix develop --command demo:wasm && node demo/js/main.mjs           # wasm-bindgen
nix develop --command demo                                         # all three, diffed
```

`--fanout` on any host runs the two-waits-per-batch variant; `--ticker` on
the Python host drives a `Quiet` machine, which can only ever emit tags 4
and 5.
