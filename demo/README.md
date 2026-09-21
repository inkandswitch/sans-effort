# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — as one
effect routine, driven four ways that must agree byte for byte.

```
  greeter/      the routine (effects style), its View/Encode, a scripted world,
                and a Rust host as `main.rs`
  cdylib/       the C-ABI skin over effect_routine_host: five #[no_mangle]
                wrappers, three unsafe blocks, no mechanism
  python/       a ctypes host that speaks ABI.md with a byte buffer and no library
  wasm/         the wasm-bindgen skin: the generated class is the handle, typed
                getters replace the codec; holds a Machine directly
  js/           a Node host over that module; `pkg/` is generated
```

Two kinds of skin, on purpose. The C ABI is the _raw_ path: any language
that can `dlopen` and hand over bytes can drive the routine, and the host
writes its own decoder from `ABI.md`. wasm-bindgen is the _generated_ path:
no handle table, no codec, but a per-language toolchain. The routine cannot
tell which it is under.

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter
cargo build -p greeter_cdylib && python3 demo/python/main.py
nix develop --command demo:wasm && node demo/js/main.mjs
nix develop --command demo      # all of the above, and diff the transcripts
```

Add `--fanout` to any host for the variant with two waits per batch.
