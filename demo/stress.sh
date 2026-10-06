#!/usr/bin/env bash
# Host check: every host, under many seeded, adversarial schedules, must write
# what tokio writes on an ordinary run.
#
#   nix develop --command demo:stress [SEEDS]     (default 10)
#
# For each seed and each mode: tokio with a worker count from the seed (its
# own scheduler is the adversary), and Python, Node, and Java with `--seed`
# (Python reorders machines, defers replies, and resumes spuriously; Node
# settles answers out of order; Java varies its worker count and delays its
# asks). A run fails if it exits non-zero or writes anything else; its output
# is kept as /tmp/sans-effort-stress-<host>-<mode>-<seed>.txt. Every Python
# run is also recorded and replayed in Rust (`--record`, the `replay` binary):
# Python is a sequential host, so its runs must replay exactly. Needs python3,
# node, and java on PATH, and the wasm module built (`demo:wasm`).

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
seeds=${1:-10}

cargo build -q -p greeter_cdylib -p greeter_tokio || exit 1
if [ ! -f demo/native/js/pkg/greeter_wasm.js ]; then
	echo "build the wasm module first: nix develop --command demo:wasm"
	exit 1
fi

tokio=target/debug/greeter_tokio
java_host=(java --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java)
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
	*) script='' ;;
	esac
	mode=${variant#--}
	mode=${mode:-greeter}
	mode_failed=0

	printf "$script" | "$tokio" $variant >"$reference" 2>/dev/null || {
		echo "the reference run failed: $mode"
		exit 1
	}

	for seed in $(seq 1 "$seeds"); do
		workers=$((seed % 4 + 1))
		printf "$script" | timeout 60 "$tokio" $variant --workers "$workers" >"$out" 2>/dev/null
		check "tokio-$workers-workers" $? "$mode" "$seed"
		timeout 60 python3 demo/driven/python/main.py $variant --seed "$seed" --record "$log" >"$out" 2>/dev/null
		check python $? "$mode" "$seed"
		if ! target/debug/replay "$log" "$mode" >"$out" 2>&1; then
			kept="/tmp/sans-effort-stress-replay-$mode-$seed.log"
			cp "$log" "$kept"
			echo "  FAILED: replaying python's run, $mode, seed $seed: $(cut -c1-120 "$out") — log in $kept"
			failed=1
			mode_failed=1
		fi
		timeout 60 node demo/native/js/main.mjs $variant --seed "$seed" >"$out" 2>/dev/null
		check node $? "$mode" "$seed"
		timeout 60 "${java_host[@]}" $variant --seed "$seed" >"$out" 2>/dev/null
		check java $? "$mode" "$seed"
	done
	if [ "$mode_failed" -eq 0 ]; then
		echo "Agree under $seeds seeds: $mode"
	fi
done

exit "$failed"
