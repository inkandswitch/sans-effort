# Project-specific commands not covered by nix-command-utils built-in modules
{
  pkgs,
  system,
  cmd,
}: let
  cargo = "${pkgs.cargo}/bin/cargo";
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

  "test:no_std" = cmd "Check no_std compatibility (wasm32, thumbv6m)" ''
    set -e

    echo "===> Checking (no_std, no default features)..."
    ${cargo} check --workspace --no-default-features

    echo ""
    echo "===> Checking (wasm32-unknown-unknown)..."
    ${cargo} check --workspace --no-default-features --target wasm32-unknown-unknown

    echo ""
    echo "===> Checking (thumbv6m-none-eabi)..."
    ${cargo} check --workspace --no-default-features --target thumbv6m-none-eabi

    echo ""
    echo "Done"
  '';

  "test:props" = cmd "Run property tests with many iterations" ''
    set -e
    export BOLERO_RANDOM_ITERATIONS=100000
    ${cargo} test --workspace --all-features proptests -- --nocapture
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

  "ci:full" = cmd "Run full CI (fmt, clippy, all-features, no_std, typos, deny)" ''
    set -e

    echo "===> [1/6] Checking formatting..."
    ${cargo} fmt --all --check

    echo "===> [2/6] Running Clippy (all features)..."
    ${cargo} clippy --workspace --all-targets --all-features -- -D warnings

    echo "===> [3/6] Testing (all features)..."
    ${cargo} test --workspace --all-features

    echo "===> [4/6] Checking no_std..."
    test:no_std

    echo "===> [5/6] Checking spelling..."
    typos

    echo "===> [6/6] Checking licenses and advisories..."
    ${cargo} deny check

    echo ""
    echo "All CI suites passed"
  '';
}
