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
  wasm-bindgen-cli,
  bench-pkgs,
}: let
  cargo = pkgs.lib.getExe rust-toolchain;
  git = "${pkgs.git}/bin/git";
  java = "${jdk}/bin/java";
  node = "${pkgs.nodejs}/bin/node";
  python = "${pkgs.python3}/bin/python3";
  wasm-bindgen = "${wasm-bindgen-cli}/bin/wasm-bindgen";
  bench-path = pkgs.lib.makeBinPath bench-pkgs;

  # The shell utilities the longer checks use, pinned with everything else.
  utils-path = pkgs.lib.makeBinPath (with pkgs; [
    coreutils
    diffutils
    gnugrep
    procps
  ]);

  # `quint run --mbt` writes full ITF states, which repeat the whole model at
  # every step. The replay (sans-effort-host/tests/spec_traces.rs) compares
  # only what a host can see, so each trace keeps that: the action and its
  # picks, and after it the calls in flight with their outputs, `wakes`, and
  # the stale-reply count. The steps after the verdict (`done`) go, and so do
  # repeats: a small instance has few paths.
  slim-traces = pkgs.writeText "slim-traces.py" ''
    import glob, json, sys

    raw, instance = sys.argv[1], sys.argv[2]
    keep = ("in_flight", "wakes", "stale")


    def slim(state):
        out = {"action": state["mbt::actionTaken"], "picks": state["mbt::nondetPicks"]}
        for name, value in state.items():
            if name.rsplit("::", 1)[-1] in keep:
                out[name.rsplit("::", 1)[-1]] = value
        return out


    def trace(path):
        states = [slim(s) for s in json.load(open(path))["states"]]
        while states and states[-1]["action"] == "done":
            states.pop()
        return states


    paths = sorted(glob.glob(f"{raw}/{instance}_*.itf.json"),
                   key=lambda p: int(p.rsplit("_", 1)[1].split(".")[0]))
    distinct = list({json.dumps(t, sort_keys=True): t for t in map(trace, paths)}.values())
    json.dump({"instance": instance, "traces": distinct}, sys.stdout, separators=(",", ":"))
    print(f"{instance}: {len(distinct)} traces, {sum(map(len, distinct))} steps", file=sys.stderr)
  '';
in {
  "bench:instructions" = cmd "Count instructions and allocations for the core workloads (Gungraun, under Valgrind; Linux)" ''
    set -e
    export PATH="${bench-path}:$PATH"
    ${cargo} bench -p sans-effort-core --bench instructions "$@"
  '';

  "bench:time" = cmd "Time the core workloads and the host table (criterion); pass --save-baseline NAME or --baseline NAME" ''
    set -e
    ${cargo} bench -p sans-effort-core --bench time -- "$@"
    ${cargo} bench -p sans-effort-host --bench table -- "$@"
  '';

  "lint" = cmd "Check formatting, Clippy (all features, and the wasm module on wasm32), and rustdoc (no warnings)" ''
    set -e

    echo "===> Checking formatting..."
    ${cargo} fmt --all --check

    echo ""
    echo "===> Running Clippy (all features)..."
    ${cargo} clippy --workspace --all-targets --all-features -- -D warnings

    echo ""
    echo "===> Running Clippy on the wasm module (wasm32)..."
    # The host Clippy never sees the `#[wasm_bindgen]` code cfg'd for wasm32.
    ${cargo} clippy -p greeter_wasm --target wasm32-unknown-unknown -- -D warnings

    echo ""
    echo "===> Building the docs (no warnings, no broken links)..."
    RUSTDOCFLAGS="-D warnings" ${cargo} doc --workspace --no-deps --all-features

    echo ""
    echo "Done"
  '';

  "test:host" = cmd "Run tests (default and all features) and doc tests" ''
    set -e

    echo "===> Running tests (default features)..."
    ${cargo} test --workspace

    echo ""
    echo "===> Running tests (all features)..."
    ${cargo} test --workspace --all-features

    echo ""
    echo "===> Running doc tests..."
    ${cargo} test --workspace --doc

    echo ""
    echo "Done"
  '';

  "test:no_std" = cmd "Check the core crate (wasm32, thumbv6m) and the demo routine (wasm32) build without std" ''
    set -e

    echo "===> Checking sans-effort-core and the sans-effort facade (no_std: spin lock)..."
    ${cargo} check -p sans-effort-core -p sans-effort --no-default-features --features spin

    echo ""
    echo "===> Checking every feature combination builds (or is refused on purpose)..."
    ${cargo} hack check -p sans-effort-core --feature-powerset --at-least-one-of std,spin
    ${cargo} hack check -p sans-effort --feature-powerset --at-least-one-of std,spin
    ${cargo} hack check -p sans-effort-effects --feature-powerset --at-least-one-of std,spin
    ${cargo} hack check -p sans-effort-host --feature-powerset --at-least-one-of std,spin

    echo ""
    echo "===> Checking the library crates without std (wasm32-unknown-unknown)..."
    ${cargo} check -p sans-effort -p sans-effort-core -p sans-effort-effects -p sans-effort-host --no-default-features --features spin --target wasm32-unknown-unknown

    echo ""
    echo "===> Checking sans-effort-core and the facade (thumbv6m-none-eabi, critical-section)..."
    ${cargo} check -p sans-effort-core -p sans-effort --no-default-features --features critical-section --target thumbv6m-none-eabi

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

  "demo" = cmd "Run the demo routines natively on tokio and on Node, and drive them from Python and Java over the C ABI; transcripts must agree" ''
    # pipefail: each host's output goes through `tee`, and a host that fails —
    # or overruns its time limit — must fail the demo, not just stop early.
    set -eo pipefail

    echo "===> Building the cdylib and the wasm module..."
    ${cargo} build -q -p greeter_cdylib
    demo:wasm

    for variant in "" "--fanout" "--ping-pong" "--front-desk" "--ring" "--journal" "--deadline"; do
      case "$variant" in
        "") script='alice\nbob\nquit\n' ;;
        --fanout) script='bob\ncarol\n' ;;
        --ping-pong) script="" ;;
        --front-desk) script='alice\nbob\ncarol\n' ;;
        --ring) script="" ;;
        --journal) script="" ;;
        --deadline) script="" ;;
      esac

      # A deadline's quick worker beats a 30 s sleep; a host that fails to
      # cancel the abandoned timer waits it out, which the limit catches.
      limit=""
      if [ "$variant" = "--deadline" ]; then limit="timeout 20"; fi

      echo ""
      echo "===> tokio, natively (no driver) $variant"
      printf "$script" | ${cargo} run -q -p greeter_tokio -- $variant | tee /tmp/sans-effort-rust.txt

      echo ""
      echo "===> Python host $variant"
      $limit ${python} demo/driven/python/main.py $variant | tee /tmp/sans-effort-python.txt

      echo ""
      echo "===> Node, natively (no driver) $variant"
      $limit ${node} demo/native/js/main.mjs $variant | tee /tmp/sans-effort-js.txt

      echo ""
      echo "===> Java host (Panama, C ABI) $variant"
      $limit ${java} --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java $variant | tee /tmp/sans-effort-java.txt

      echo ""
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-python.txt
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-js.txt
      diff /tmp/sans-effort-rust.txt /tmp/sans-effort-java.txt
      echo "Transcripts agree $variant"
    done

    echo ""
    echo "===> Python host, a Quiet machine (ticker): only tags 4 and 5 can appear"
    ${python} demo/driven/python/main.py --ticker
  '';

  # Host check: every host, under many seeded, adversarial schedules, must
  # write what tokio writes on an ordinary run. For each seed and each mode:
  # tokio with a worker count from the seed (its own scheduler is the
  # adversary), and Python, Node, and Java with `--seed` (Python reorders
  # machines, defers replies, and resumes spuriously; Node settles answers out
  # of order; Java varies its worker count and delays its asks). A run fails
  # if it exits non-zero or writes anything else; its output is kept as
  # /tmp/sans-effort-stress-<host>-<mode>-<seed>.txt. Every Python run is also
  # recorded and replayed in Rust (`--record`, the `replay` binary): Python is
  # a sequential host, so its runs must replay exactly.
  "demo:stress" = cmd "Run every host under seeded adversarial schedules (default 10 seeds; pass a number); output must match the tokio reference" ''
    set -uo pipefail
    export PATH=${utils-path}:$PATH
    seeds=''${1:-10}

    ${cargo} build -q -p greeter_cdylib -p greeter_tokio || exit 1
    demo:wasm > /dev/null || exit 1

    tokio=target/debug/greeter_tokio
    reference=/tmp/sans-effort-stress-reference.txt
    out=/tmp/sans-effort-stress-out.txt
    log=/tmp/sans-effort-stress-python.log
    failed=0
    mode_failed=0

    # One run: a host's output and exit code against the reference.
    check() {
      local host=$1 code=$2 mode=$3 seed=$4
      if [ "$code" -ne 0 ] || ! cmp -s "$out" "$reference"; then
        local kept="/tmp/sans-effort-stress-$host-$mode-$seed.txt"
        cp "$out" "$kept"
        echo "  FAILED: $host, $mode, seed $seed (exit $code) — output in $kept"
        failed=1
        mode_failed=1
      fi
    }

    for variant in "" --fanout --ping-pong --front-desk --ring --journal --deadline; do
      case "$variant" in
        "") script='alice\nbob\nquit\n' ;;
        --fanout) script='bob\ncarol\n' ;;
        --front-desk) script='alice\nbob\ncarol\n' ;;
        *) script="" ;;
      esac
      mode=''${variant#--}
      mode=''${mode:-greeter}
      mode_failed=0

      printf "$script" | "$tokio" $variant > "$reference" 2> /dev/null || {
        echo "the reference run failed: $mode"
        exit 1
      }

      for seed in $(seq 1 "$seeds"); do
        workers=$((seed % 4 + 1))
        printf "$script" | timeout 60 "$tokio" $variant --workers "$workers" > "$out" 2> /dev/null
        check "tokio-$workers-workers" $? "$mode" "$seed"
        timeout 60 ${python} demo/driven/python/main.py $variant --seed "$seed" --record "$log" > "$out" 2> /dev/null
        check python $? "$mode" "$seed"
        if ! target/debug/replay "$log" "$mode" > "$out" 2>&1; then
          kept="/tmp/sans-effort-stress-replay-$mode-$seed.log"
          cp "$log" "$kept"
          echo "  FAILED: replaying python's run, $mode, seed $seed: $(cut -c1-120 "$out") — log in $kept"
          failed=1
          mode_failed=1
        fi
        timeout 60 ${node} demo/native/js/main.mjs $variant --seed "$seed" > "$out" 2> /dev/null
        check node $? "$mode" "$seed"
        timeout 60 ${java} --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java $variant --seed "$seed" > "$out" 2> /dev/null
        check java $? "$mode" "$seed"
      done
      if [ "$mode_failed" -eq 0 ]; then
        echo "Agree under $seeds seeds: $mode"
      fi
    done

    exit "$failed"
  '';

  # Host checks: routines that misbehave on purpose (`routines::faults`), and
  # what every host must do about them. Not part of `demo`, which shows how
  # routines are written; these check the hosts.
  "demo:faults" = cmd "Check that every host reports a misbehaving routine (a deadlock) the same way, rather than hanging" ''
    set -eo pipefail
    ${cargo} build -q -p greeter_cdylib
    demo:wasm

    # A deadlock: two machines each waiting for the other. The driven hosts
    # report the stall on stderr and exit 1; Node's event loop runs dry and it
    # exits 13 (an unsettled top-level await). tokio has no detector: it would
    # hang, so it is not run. A host that hangs instead hits the time limit.
    echo "===> A deadlock (--deadlock): reported, not hung on"
    set +e
    timeout 20 ${python} demo/driven/python/main.py --deadlock \
      > /tmp/sans-effort-python.txt 2> /tmp/sans-effort-python-err.txt
    python_exit=$?
    timeout 20 ${java} --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java --deadlock \
      > /tmp/sans-effort-java.txt 2> /tmp/sans-effort-java-err.txt
    java_exit=$?
    timeout 20 ${node} demo/native/js/main.mjs --deadlock > /tmp/sans-effort-js.txt 2> /dev/null
    js_exit=$?
    set -e
    cat /tmp/sans-effort-python.txt /tmp/sans-effort-python-err.txt
    if [ "$python_exit $java_exit $js_exit" != "1 1 13" ]; then
      echo "exit codes: Python $python_exit, Java $java_exit, Node $js_exit; expected 1, 1, 13"
      exit 1
    fi
    diff /tmp/sans-effort-python.txt /tmp/sans-effort-java.txt
    diff /tmp/sans-effort-python.txt /tmp/sans-effort-js.txt
    diff /tmp/sans-effort-python-err.txt /tmp/sans-effort-java-err.txt
    echo "Deadlock reported --deadlock"
  '';

  # The host-protocol spec (spec/host_protocol.qnt), model-checked by TLC
  # through Quint: every real instance must pass, and every mistaken host must
  # be caught — those prove the properties can fail. Full output per instance
  # goes to /tmp/sans-effort-spec-<instance>.log.
  #
  # Each `quint verify` starts an Apalache server on port 8822, and does not
  # always stop it; a server left behind makes the next run hang. So every run
  # has a time limit and is followed by stopping the server.
  "spec:check" = cmd "Model-check the host-protocol spec (TLC via Quint): real instances pass, mistaken hosts are caught" ''
    set -uo pipefail
    export PATH=${utils-path}:$PATH
    cd spec || exit 1
    status=0

    stop_server() {
      # `ja[r]`: a pattern that cannot match this script's own command line.
      local server='apalache\.ja[r] server'
      pkill -f "$server" || return 0
      # The JVM takes a moment to exit; the next run must not find the port taken.
      for _ in $(seq 50); do
        pgrep -f "$server" > /dev/null || return 0
        sleep 0.2
      done
      pkill -9 -f "$server" || true
    }

    check() {
      local instance=$1 expect=$2
      local log="/tmp/sans-effort-spec-$instance.log"
      printf '%-26s expect %-10s ' "$instance" "$expect"
      timeout 300 ${quint}/bin/quint verify --backend=tlc --main="$instance" \
        --invariant=safe,ends_right host_protocol.qnt > "$log" 2>&1
      local code=$?
      stop_server
      local states
      states=$(grep -oE '[0-9]+ distinct states found' "$log" | tail -1)
      if [ "$expect" = ok ] && [ "$code" -eq 0 ] && grep -q '^\[ok\]' "$log"; then
        echo "ok        ($states)"
      elif [ "$expect" = violation ] && grep -q '^\[violation\]' "$log"; then
        echo "caught    ($states)"
      else
        echo "FAILED    (exit $code; see $log)"
        status=1
      fi
    }

    stop_server
    for instance in ping_pong ping_pong_sequential deadline panic_wakes deadlock; do
      check "$instance" ok
    done
    for instance in mistaken_ignores_woke mistaken_ignores_closed; do
      check "$instance" violation
    done
    rm -rf _apalache-out
    exit "$status"
  '';

  # Random traces of the spec's instances, which sans-effort-host replays
  # against the real table (tests/spec_traces.rs), saved as spec/traces/*.json.
  # The seed is fixed, so the same spec and Quint draw the same traces:
  # regenerate them when the spec changes. CI checks they are current.
  "spec:traces" = cmd "Draw the traces sans-effort-host replays against the table from the spec (spec/traces/)" ''
    set -euo pipefail
    export PATH=${utils-path}:$PATH
    cd spec
    raw=$(mktemp -d)
    trap 'rm -rf "$raw"' EXIT
    traces=50

    for instance in ping_pong ping_pong_sequential deadline panic_wakes deadlock \
      mistaken_ignores_woke mistaken_ignores_closed; do
      ${quint}/bin/quint run --mbt --main="$instance" --max-steps=60 --seed=1 \
        --max-samples="$traces" --n-traces="$traces" \
        --out-itf="$raw/''${instance}_{seq}.itf.json" host_protocol.qnt > /dev/null
      ${python} ${slim-traces} "$raw" "$instance" > "traces/$instance.json"
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

  "ci:full" = cmd "Run what CI runs, but mutation testing and benchmarks (lint, tests, no_std, typos, deny, demo, host checks, spec)" ''
    set -e

    echo "===> [1/7] Linting..."
    lint

    echo "===> [2/7] Testing..."
    test:host

    echo "===> [3/7] Checking no_std..."
    test:no_std

    echo "===> [4/7] Checking spelling..."
    ${pkgs.typos}/bin/typos

    echo "===> [5/7] Checking licenses and advisories..."
    ${pkgs.cargo-deny}/bin/cargo-deny check

    echo "===> [6/7] Running the demo and the host checks..."
    demo
    demo:faults
    demo:stress 3

    echo "===> [7/7] Model-checking the spec..."
    spec:check

    echo ""
    echo "All CI suites passed"
  '';
}
