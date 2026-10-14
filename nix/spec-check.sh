# The host-protocol spec (spec/host_protocol.qnt), model-checked by TLC
# through Quint: every real instance must pass, and every mistaken host must
# be caught; those prove the properties can fail. Full output per instance
# goes to /tmp/sans-effort-spec-<instance>.log.
#
# Each `quint verify` starts an Apalache server on port 8822, and does not
# always stop it; a server left behind makes the next run hang. So every run
# has a time limit and is followed by stopping the server.
#
# The environment gives QUINT (Quint's store path, for the Apalache jar it
# bundles) and STRICT_FOLDS (nix/strict-folds.py). Failures are collected,
# not fatal: every instance runs.
set +o errexit

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
  timeout 300 quint verify --backend=tlc --main="$instance" \
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

# Liveness under weak fairness: `reaches_verdict`, compiled to TLA+ and run
# on TLC directly, beside Apalache's modules with its folds made strict (see
# nix/strict-folds.py). Through `quint verify` it never finishes.
tla=$(mktemp -d)
trap 'rm -rf "$tla"' EXIT
jars=("$QUINT"/share/quint/apalache-dist-*/apalache/lib/apalache.jar)
jar=${jars[0]}

live() {
  local instance=$1 expect=$2
  local log="/tmp/sans-effort-spec-$instance-live.log"
  printf '%-26s expect %-10s ' "$instance" "$expect"
  timeout 120 quint compile --target=tlaplus --main="$instance" \
    --temporal=reaches_verdict host_protocol.qnt 2> "$log" \
    | sed -n '/^-* MODULE/,$p' > "$tla/$instance.tla"
  stop_server
  printf 'INIT q_init\nNEXT q_step\nPROPERTY q_temporalProps\n' > "$tla/$instance.cfg"
  # The largest instances take a few minutes here; a CI runner is slower.
  (cd "$tla" && timeout 900 java -XX:+UseParallelGC -cp "$jar" tlc2.TLC \
    -workers 2 -metadir "$tla/states/$instance" -config "$instance.cfg" "$instance.tla") \
    >> "$log" 2>&1
  local code=$?
  local states
  states=$(grep -oE '[0-9]+ distinct states found' "$log" | tail -1)
  if [ "$expect" = ok ] && [ "$code" -eq 0 ] && grep -q 'No error has been found' "$log"; then
    echo "ok        ($states)"
  elif [ "$expect" = violation ] && grep -q 'Temporal properties were violated' "$log"; then
    echo "caught    ($states)"
  else
    echo "FAILED    (exit $code; see $log)"
    status=1
  fi
}

stop_server
echo "Safety, and liveness as safety (safe, ends_right):"
for instance in ping_pong ping_pong_sequential deadline panic_wakes deadlock ring front_desk; do
  check "$instance" ok
done
for instance in mistaken_ignores_woke mistaken_ignores_closed mistaken_ignores_pinning \
  mistaken_ends_when_idle mistaken_polls_idle; do
  check "$instance" violation
done

echo "Liveness under weak fairness (reaches_verdict):"
python3 "$STRICT_FOLDS" "$jar" "$tla" || exit 1
# A host that ignores closed frames still finishes; its mistake is a safety
# one, caught above. One that ignores `woke` frames does not, nor does one
# that loses an event to a call on the wrong worker, one that calls a
# deadlock the end, or one that spins on idle machines forever.
for instance in ping_pong ping_pong_sequential deadline panic_wakes deadlock ring front_desk \
  mistaken_ignores_closed; do
  live "$instance" ok
done
for instance in mistaken_ignores_woke mistaken_ignores_pinning mistaken_ends_when_idle \
  mistaken_polls_idle; do
  live "$instance" violation
done

rm -rf _apalache-out
exit "$status"
