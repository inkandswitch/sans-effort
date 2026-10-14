# Project-specific commands not covered by nix-command-utils built-in modules.
# CI runs these too (.github/workflows/), in the flake's `ci` shell: what a
# workflow checks is what the command checks, with the same tools.
{
  pkgs,
  system,
  cmd,
  jdk,
  quint,
  rust-toolchain,
  msrv-toolchain,
  wasm-bindgen-cli,
  bench-pkgs,
}: let
  cargo = pkgs.lib.getExe rust-toolchain;
  git = "${pkgs.git}/bin/git";
  wasm-bindgen = "${wasm-bindgen-cli}/bin/wasm-bindgen";
  bench-path = pkgs.lib.makeBinPath bench-pkgs;

  # A command whose script is a file of its own, nix/<file>.sh: plain bash,
  # checked by shellcheck when the shell is built, with `errexit`, `nounset`,
  # and `pipefail` set. Its tools are pinned on `PATH` (`inputs`, ahead of
  # the shell's own, which still supplies the cargo subcommands and the other
  # commands), and the files it needs are named in its environment (`env`).
  script = file: inputs: env: let
    app = pkgs.writeShellApplication {
      name = file;
      runtimeInputs = inputs;
      runtimeEnv = env;
      text = builtins.readFile (./. + "/${file}.sh");
    };
  in ''exec ${pkgs.lib.getExe app} "$@"'';

  # What the scripts' runs need: the demo's hosts, and the spec's checkers.
  hosts = [rust-toolchain pkgs.python3 pkgs.nodejs jdk pkgs.coreutils pkgs.diffutils];
  checkers = [quint jdk pkgs.python3 pkgs.coreutils pkgs.gnugrep pkgs.gnused pkgs.procps];
in {
  "bench:instructions" = cmd "Count instructions and allocations for the core workloads and the handle table (Gungraun, under Valgrind; Linux)" ''
    set -e
    export PATH="${bench-path}:$PATH"
    ${cargo} bench -p sans-effort-core --bench instructions "$@"
    ${cargo} bench -p sans-effort-host --bench instructions "$@"
  '';

  "bench:time" = cmd "Time the core workloads and the host table (criterion); pass --save-baseline NAME or --baseline NAME" ''
    set -e
    ${cargo} bench -p sans-effort-core --bench time -- "$@"
    ${cargo} bench -p sans-effort-host --bench table -- "$@"
  '';

  "lint" = cmd "Check formatting (Rust, Nix), Clippy (all features, and the wasm module on wasm32), rustdoc (no warnings), and the Nix commands' Python (ruff)" (script "lint" [rust-toolchain pkgs.git pkgs.alejandra pkgs.ruff pkgs.findutils] {});

  "test:host" = cmd "Run tests (default and all features, and the demo vocabulary without its table) and doc tests" (script "test-host" [rust-toolchain] {});

  "test:no_std" = cmd "Check the core crate (wasm32, thumbv6m) and the demo routine (wasm32) build without std" (script "test-no-std" [rust-toolchain pkgs.gnugrep] {});

  # loom models of the driver's waker and outbox (sans-effort-core/tests/
  # loom.rs): every interleaving of a wake with a poll, of wakes with each
  # other, and of a tell from another thread with a drain. `--cfg
  # sans_effort_loom` swaps the driver's lock and wake flag for loom's; its
  # own target directory keeps that from rebuilding everything else.
  # The MSRV (Cargo.toml's `rust-version`, which the flake reads to pick this
  # toolchain): the published crates' libraries must build with it, in every
  # configuration a user can choose. Tests and benches are not checked —
  # their dev-dependencies set their own floors. `RUSTC` is set because Cargo
  # runs the `rustc` on `PATH` — the development toolchain's — not the one
  # beside it; and a target directory per version keeps another version's
  # build from passing for this one.
  "test:msrv" = cmd "Check the published crates build with the MSRV toolchain (Cargo.toml's rust-version)" ''
    set -e
    export RUSTC=${msrv-toolchain}/bin/rustc
    version=$("$RUSTC" --version | cut -d' ' -f2)
    export CARGO_TARGET_DIR="target/msrv/$version"
    echo "rustc $version"

    echo "===> All features..."
    ${pkgs.lib.getExe msrv-toolchain} check -p sans-effort-core -p sans-effort-effects -p sans-effort-host \
      -p sans-effort-tokio -p sans-effort --all-features

    echo ""
    echo "===> No std (spin lock)..."
    ${pkgs.lib.getExe msrv-toolchain} check -p sans-effort-core -p sans-effort-effects -p sans-effort-host \
      -p sans-effort --no-default-features --features spin
  '';

  "test:loom" = cmd "Model-check the waker and outbox of the driver under every interleaving (loom)" ''
    set -e
    RUSTFLAGS="--cfg sans_effort_loom" CARGO_TARGET_DIR=target/loom \
      ${cargo} test --release -p sans-effort-core --test loom "$@"
  '';

  "test:props" = cmd "Run property tests with many iterations" ''
    set -e
    export BOLERO_RANDOM_ITERATIONS=100000
    ${cargo} test --workspace --all-features -- --nocapture
  '';

  # Mutation testing: does some test fail when the code is changed? Slow, so
  # not on every push (CI: weekly, and the changed lines of a pull request).
  # A fixed seed keeps bolero's properties drawing the same inputs, so a
  # mutant is caught or missed the same way every run. Survivors are listed
  # in target/mutants/mutants.out/missed.txt; accepted ones are excluded, with
  # reasons, in .cargo/mutants.toml. Exit code 3 means some mutants only timed
  # out: the tests hung rather than passed — a mutant that makes the test
  # runner loop forever — so those count as caught. MUTANTS_JOBS sets the
  # parallelism (default 4; CI's runners have fewer cores).
  "test:mutants" = cmd "Mutation-test the four library crates (~8 min); survivors in target/mutants" ''
    set -e
    export BOLERO_RANDOM_SEED=1
    ${cargo} mutants -p sans-effort-core -p sans-effort-effects -p sans-effort-host -p sans-effort-tokio \
      -j "''${MUTANTS_JOBS:-4}" --output target/mutants "$@" || test $? -eq 3
  '';

  "test:mutants:diff" = cmd "Mutation-test only the lines changed since a base (default origin/main)" ''
    set -e
    export BOLERO_RANDOM_SEED=1
    mkdir -p target
    ${git} diff "''${1:-origin/main}" > target/mutants.diff
    ${cargo} mutants -p sans-effort-core -p sans-effort-effects -p sans-effort-host -p sans-effort-tokio \
      --in-diff target/mutants.diff -j "''${MUTANTS_JOBS:-4}" --output target/mutants || test $? -eq 3
  '';

  "demo:wasm" = cmd "Build the wasm-bindgen module and generate the JS glue into demo/native/js/pkg" ''
    set -e
    ${cargo} build -q -p greeter_wasm --release --target wasm32-unknown-unknown
    mkdir -p demo/native/js/pkg
    ${wasm-bindgen} --target nodejs --out-dir demo/native/js/pkg \
      target/wasm32-unknown-unknown/release/greeter_wasm.wasm
    echo "demo/native/js/pkg ready"
  '';

  "demo" = cmd "Run the demo routines natively on tokio and on Node, and drive them from Python and Java over the C ABI; transcripts must agree" (script "demo" hosts {});

  "demo:stress" = cmd "Run every host under seeded adversarial schedules (default 10 seeds; pass a number); output must match the tokio reference" (script "demo-stress" hosts {});

  "demo:faults" = cmd "Check that every host reports a misbehaving routine (a deadlock) the same way, rather than hanging" (script "demo-faults" hosts {});

  "spec:check" = cmd "Model-check the host-protocol spec (TLC via Quint): safety, and liveness under weak fairness; real instances pass, mistaken hosts are caught" (script "spec-check" checkers {
    QUINT = "${quint}";
    STRICT_FOLDS = "${./strict-folds.py}";
  });

  "spec:traces" = cmd "Draw the traces sans-effort-host replays against the table from the spec (spec/traces/)" (script "spec-traces" checkers {SLIM_TRACES = "${./slim-traces.py}";});

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

  "ci:full" = cmd "Run what CI runs, but mutation testing and benchmarks (lint, tests, loom, no_std, typos, deny, demo, host checks, spec)" (script "ci-full" [pkgs.typos pkgs.cargo-deny] {});
}
