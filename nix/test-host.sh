# Every test on the host: default and all features, the demo vocabulary
# without its table, and the doc tests.

echo "===> Running tests (default features)..."
cargo test --workspace

echo ""
echo "===> Running tests (all features)..."
cargo test --workspace --all-features

echo ""
echo "===> Running the demo routines' and vocabulary's tests without \`table\`..."
# The configuration `test:no_std` builds for wasm32, here with its tests: a
# test that needs the table but is not gated on it fails to compile.
cargo test -p routines -p greeter_boundary --no-default-features --features sans-effort/spin

echo ""
echo "===> Running doc tests..."
cargo test --workspace --doc

echo ""
echo "Done"
