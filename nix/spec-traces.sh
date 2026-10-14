# Random traces of the spec's instances, which sans-effort-host replays
# against the real table (tests/spec_traces.rs), saved as spec/traces/*.json.
# The seed is fixed, so the same spec and Quint draw the same traces:
# regenerate them when the spec changes. CI checks they are current.
#
# The environment gives SLIM_TRACES (nix/slim-traces.py).

cd spec
raw=$(mktemp -d)
trap 'rm -rf "$raw"' EXIT

for instance in ping_pong ping_pong_sequential deadline panic_wakes deadlock ring front_desk \
  mistaken_ignores_woke mistaken_ignores_closed mistaken_ignores_pinning \
  mistaken_ends_when_idle mistaken_polls_idle; do
  # The largest instances get more: each line printed below says how many
  # distinct states an instance's traces visit, to set beside the count
  # `spec:check` prints.
  case "$instance" in
    ping_pong | ring | front_desk) traces=200 ;;
    *) traces=50 ;;
  esac
  quint run --mbt --main="$instance" --max-steps=60 --seed=1 \
    --max-samples="$traces" --n-traces="$traces" \
    --out-itf="$raw/${instance}_{seq}.itf.json" host_protocol.qnt > /dev/null
  python3 "$SLIM_TRACES" "$raw" "$instance" > "traces/$instance.json"
done
