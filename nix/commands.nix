# Project-specific commands not covered by nix-command-utils built-in modules
{
  pkgs,
  system,
  cmd,
}: let
  cargo = "${pkgs.cargo}/bin/cargo";
  python = "${pkgs.python3}/bin/python3";
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

  "demo" = cmd "Drive the greeter from Rust and from Python over the C ABI; transcripts must agree" ''
    set -e

    echo "===> Building the cdylib..."
    ${cargo} build -p greeter_cdylib

    echo ""
    echo "===> Rust host"
    printf 'alice\nbob\nquit\n' | ${cargo} run -q -p greeter | tee /tmp/effect-routine-rust.txt

    echo ""
    echo "===> Python host"
    ${python} demo/python/main.py | tee /tmp/effect-routine-python.txt

    echo ""
    diff /tmp/effect-routine-rust.txt /tmp/effect-routine-python.txt && echo "Transcripts agree"
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

    echo "===> [7/7] Running the demo..."
    demo

    echo ""
    echo "All CI suites passed"
  '';
}
