#!/usr/bin/env bash
# Model-check the host-protocol spec with TLC, through Quint: every real
# instance must pass, and every mistaken host must be caught — those prove the
# properties can fail.
#
#   nix develop --command spec:check     (or: bash spec/check.sh, with `quint`
#                                         and Java 17+ on PATH)
#
# Full output per instance goes to /tmp/sans-effort-spec-<instance>.log.
#
# Each `quint verify` starts an Apalache server on port 8822, and does not
# always stop it; a server left behind makes the next run hang. So every run
# has a time limit and is followed by stopping the server.

set -uo pipefail
cd "$(dirname "$0")" || exit 1
quint=${QUINT:-quint}
status=0

stop_server() {
	# `ja[r]`: a pattern that cannot match this script's own command line.
	local server='apalache\.ja[r] server'
	pkill -f "$server" || return 0
	# The JVM takes a moment to exit; the next run must not find the port taken.
	for _ in $(seq 50); do
		pgrep -f "$server" >/dev/null || return 0
		sleep 0.2
	done
	pkill -9 -f "$server" || true
}

check() {
	local instance=$1 expect=$2
	local log="/tmp/sans-effort-spec-$instance.log"
	printf '%-26s expect %-10s ' "$instance" "$expect"
	timeout 300 "$quint" verify --backend=tlc --main="$instance" \
		--invariant=safe,ends_right host_protocol.qnt >"$log" 2>&1
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
