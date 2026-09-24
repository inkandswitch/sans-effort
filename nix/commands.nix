# Project-specific commands not covered by nix-command-utils built-in modules
{
  pkgs,
  system,
  cmd,
  wasm-bindgen-cli,
}: let
  cargo = "${pkgs.cargo}/bin/cargo";
  java = "${pkgs.jdk25}/bin/java";
  node = "${pkgs.nodejs}/bin/node";
  python = "${pkgs.python3}/bin/python3";
  wasm-bindgen = "${wasm-bindgen-cli}/bin/wasm-bindgen";
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

  "test:no_std" = cmd "Check the core crate (wasm32, thumbv6m) and the demo routine (wasm32) build without std" ''
    set -e

    echo "===> Checking sans-effort (no_std: spin lock)..."
    ${cargo} check -p sans-effort --no-default-features --features spin

    echo ""
    echo "===> Checking every feature combination builds (or is refused on purpose)..."
    ${cargo} hack check -p sans-effort --feature-powerset --at-least-one-of std,spin
    ${cargo} hack check -p sans-effort-effects --feature-powerset --at-least-one-of std,spin
    ${cargo} hack check -p sans-effort-host --feature-powerset --at-least-one-of std,spin

    echo ""
    echo "===> Checking sans-effort, sans-effort-effects, and sans-effort-host without std (wasm32-unknown-unknown)..."
    ${cargo} check -p sans-effort -p sans-effort-effects -p sans-effort-host --no-default-features --features spin --target wasm32-unknown-unknown

    echo ""
    echo "===> Checking sans-effort (thumbv6m-none-eabi, critical-section)..."
    ${cargo} check -p sans-effort --no-default-features --features critical-section --target thumbv6m-none-eabi

    echo ""
    echo "===> Checking the demo routines and their vocabulary crate are no_std too (wasm32)..."
    # Neither crate picks a lock — that is the binary's decision — so checking
    # them as leaves means standing in for the binary here. No default
    # features: the vocabulary's `table` feature (spawning) needs std.
    ${cargo} check -p routines -p greeter_boundary --no-default-features --features sans-effort/spin --target wasm32-unknown-unknown

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

  "demo" = cmd "Run the demo routines natively on tokio and on Node, and drive them from Python and Java over the C ABI; transcripts must agree" ''
    set -e

    echo "===> Building the cdylib and the wasm module..."
    ${cargo} build -q -p greeter_cdylib
    demo:wasm

    for variant in "" "--fanout" "--ping-pong" "--front-desk" "--ring"; do
      case "$variant" in
        "") script='alice\nbob\nquit\n' ;;
        --fanout) script='bob\ncarol\n' ;;
        --ping-pong) script="" ;;
        --front-desk) script='alice\nbob\ncarol\n' ;;
        --ring) script="" ;;
      esac

      echo ""
      echo "===> tokio, natively (no driver) $variant"
      printf "$script" | ${cargo} run -q -p greeter_tokio -- $variant | tee /tmp/sans-effort-rust.txt

      echo ""
      echo "===> Python host $variant"
      ${python} demo/python/main.py $variant | tee /tmp/sans-effort-python.txt

      echo ""
      echo "===> Node, natively (no driver) $variant"
      ${node} demo/js/main.mjs $variant | tee /tmp/sans-effort-js.txt

      echo ""
      echo "===> Java host (Panama, C ABI) $variant"
      ${java} --enable-native-access=ALL-UNNAMED demo/java/Main.java $variant | tee /tmp/sans-effort-java.txt

      echo ""
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-python.txt
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-js.txt
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-java.txt
      echo "Transcripts agree $variant"
    done

    echo ""
    echo "===> Python host, a Quiet machine (ticker): only tags 4 and 5 can appear"
    ${python} demo/python/main.py --ticker
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

    echo "===> [2/7] Running Clippy (all features) and rustdoc (no broken links)..."
    ${cargo} clippy --workspace --all-targets --all-features -- -D warnings
    RUSTDOCFLAGS="-D warnings" ${cargo} doc --workspace --no-deps --all-features

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
