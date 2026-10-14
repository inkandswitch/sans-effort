# The library crates and the demo routines build without std: on wasm32, on
# thumbv6m with `critical-section`, in every feature combination, and with no
# lock at all as one clear error.

echo "===> Checking sans-effort-core and the sans-effort facade (no_std: spin lock)..."
cargo check -p sans-effort-core -p sans-effort --no-default-features --features spin

echo ""
echo "===> Checking every feature combination builds (or is refused on purpose)..."
for crate in sans-effort-core sans-effort sans-effort-effects sans-effort-host; do
  cargo hack check -p "$crate" --feature-powerset --at-least-one-of std,spin
done

echo ""
echo "===> Checking the library crates without std (wasm32-unknown-unknown)..."
cargo check -p sans-effort -p sans-effort-core -p sans-effort-effects -p sans-effort-host \
  --no-default-features --features spin --target wasm32-unknown-unknown

echo ""
echo "===> Checking sans-effort-core and the facade (thumbv6m-none-eabi, critical-section)..."
cargo check -p sans-effort-core -p sans-effort --no-default-features --features critical-section \
  --target thumbv6m-none-eabi

echo ""
echo "===> Checking the demo routines and their vocabulary crate are no_std too (wasm32)..."
# Neither crate picks a lock (that is the binary's decision), so checking them
# as leaves means standing in for the binary here. No default features: the
# vocabulary's `table` feature (spawning) needs std.
cargo check -p routines -p greeter_boundary --no-default-features --features sans-effort/spin \
  --target wasm32-unknown-unknown

echo ""
echo "===> Without a lock: one error, the one that says which feature to enable..."
# No colour, whatever the environment asks for (CI sets CARGO_TERM_COLOR):
# escape codes inside `error` would hide it from the count.
out=$(cargo check -p sans-effort-core --no-default-features --message-format=short --color=never 2>&1 || true)
# Short format: an error line is `error…`, or `path:line:col: error…`.
errors=$(printf '%s\n' "$out" | grep -cE '(^|: )error(\[|:)' || true)
if ! printf '%s\n' "$out" | grep -q 'needs a lock' || [ "$errors" -ne 2 ]; then
  printf '%s\n' "$out"
  echo "expected only the missing-lock error (and cargo's summary), got $errors errors"
  exit 1
fi

echo ""
echo "Done"
