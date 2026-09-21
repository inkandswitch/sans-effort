# demo

The greeter — prompt, read, look up, pause, greet, count, repeat — as one
effect routine, driven three ways that must agree byte for byte.

```
  greeter/      the routine (effects style), its View/Encode, a scripted world,
                and a Rust host as `main.rs`
  cdylib/       the C-ABI skin over effect_routine_host: five #[no_mangle]
                wrappers, three unsafe blocks, no mechanism
  python/       a ctypes host that speaks ABI.md with a byte buffer and no library
```

```sh
printf 'alice\nbob\nquit\n' | cargo run -p greeter
cargo build -p greeter_cdylib && python3 demo/python/main.py
nix develop --command demo      # both, and diff the transcripts
```

Add `--fanout` to either host for the variant with two waits per batch.
