# Project-specific commands not covered by nix-command-utils built-in modules
{
  pkgs,
  system,
  cmd,
}: let
  cargo = "${pkgs.cargo}/bin/cargo";
  node = "${pkgs.nodejs}/bin/node";
  python = "${pkgs.python3}/bin/python3";
  wasm-bindgen = "${pkgs.wasm-bindgen-cli}/bin/wasm-bindgen";
in {
  "test:host" = cmd "Run tests and doc tests" ''
    set -e

    echo "===> Running tests..."
    ${cargo} test --workspace

    echo ""
    echo "===> Running doc tests..."
    ${cargo} test --workspace --doc

    echo ""
    echo "Done"
  '';

  "test:no_std" = cmd "Check the core crate for no_std targets (wasm32, thumbv6m)" ''
    set -e

    echo "===> Checking effect_routine (no_std, no default features)..."
    ${cargo} check -p effect_routine --no-default-features

    echo ""
    echo "===> Checking effect_routine (wasm32-unknown-unknown)..."
    ${cargo} check -p effect_routine --no-default-features --target wasm32-unknown-unknown

    echo ""
    echo "===> Checking effect_routine (thumbv6m-none-eabi, critical-section)..."
    ${cargo} check -p effect_routine --no-default-features --features critical-section --target thumbv6m-none-eabi

    echo ""
    echo "Done"
  '';

  "test:props" = cmd "Run property tests with many iterations" ''
    set -e
    export BOLERO_RANDOM_ITERATIONS=100000
    ${cargo} test --workspace --all-features -- --nocapture
  '';

  "demo:wasm" = cmd "Build the wasm-bindgen module and generate the JS glue into demo/js/pkg" ''
    set -e
    ${cargo} build -q -p greeter_wasm --release --target wasm32-unknown-unknown
    mkdir -p demo/js/pkg
    ${wasm-bindgen} --target nodejs --out-dir demo/js/pkg \
      target/wasm32-unknown-unknown/release/greeter_wasm.wasm
    echo "demo/js/pkg ready"
  '';

  "demo" = cmd "Drive the greeter from Rust, Python (C ABI), and JS (wasm-bindgen); transcripts must agree" ''
    set -e

    echo "===> Building the cdylib and the wasm module..."
    ${cargo} build -q -p greeter_cdylib
    demo:wasm

    for variant in "" "--fanout"; do
      if [ -z "$variant" ]; then
        script='alice\nbob\nquit\n'
      else
        script='bob\ncarol\n'
      fi

      echo ""
      echo "===> Rust host $variant"
      printf "$script" | ${cargo} run -q -p greeter -- $variant | tee /tmp/effect-routine-rust.txt

      echo ""
      echo "===> Python host $variant"
      ${python} demo/python/main.py $variant | tee /tmp/effect-routine-python.txt

      echo ""
      echo "===> JS host $variant"
      ${node} demo/js/main.mjs $variant | tee /tmp/effect-routine-js.txt

      echo ""
      diff /tmp/effect-routine-rust.txt /tmp/effect-routine-python.txt
      diff /tmp/effect-routine-rust.txt /tmp/effect-routine-js.txt
      echo "Transcripts agree $variant"
    done
  '';

  "ci:quick" = cmd "Run quick CI checks (fmt, clippy, test)" ''
    set -e

    echo "===> Checking formatting..."
    ${cargo} fmt --all --check

    echo "===> Running Clippy..."
    ${cargo} clippy --workspace --all-targets -- -D warnings

    echo "===> Running tests..."
    ${cargo} test --workspace

    echo ""
    echo "Done"
  '';

  "ci:full" = cmd "Run full CI (fmt, clippy, all-features, no_std, typos, deny, demo)" ''
    set -e

    echo "===> [1/7] Checking formatting..."
    ${cargo} fmt --all --check

    echo "===> [2/7] Running Clippy (all features)..."
    ${cargo} clippy --workspace --all-targets --all-features -- -D warnings

    echo "===> [3/7] Testing (all features)..."
    ${cargo} test --workspace --all-features

    echo "===> [4/7] Checking no_std..."
    test:no_std

    echo "===> [5/7] Checking spelling..."
    typos

    echo "===> [6/7] Checking licenses and advisories..."
    ${cargo} deny check

    echo "===> [7/7] Running the demo (Rust, Python, JS)..."
    demo

    echo ""
    echo "All CI suites passed"
  '';
}
