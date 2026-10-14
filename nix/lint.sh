# Formatting (Rust, Nix), Clippy (all features, and the wasm module on
# wasm32), rustdoc without warnings, and the Nix commands' Python. The Nix
# commands' shell scripts are checked by shellcheck when the shell is built.

echo "===> Checking formatting..."
cargo fmt --all --check
# The flake's `formatter` (`nix fmt`), run directly on every tracked Nix
# file: quiet when they pass, and naming the ones that do not.
if ! git ls-files -z '*.nix' | xargs -0 alejandra --check --quiet; then
  git ls-files -z '*.nix' | xargs -0 alejandra --check || true
  echo "Run \`nix fmt\` to format them."
  exit 1
fi

echo ""
echo "===> Running Clippy (all features)..."
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo ""
echo "===> Running Clippy on the wasm module (wasm32)..."
# The host Clippy never sees the `#[wasm_bindgen]` code cfg'd for wasm32.
cargo clippy -p greeter_wasm --target wasm32-unknown-unknown -- -D warnings

echo ""
echo "===> Building the docs (no warnings, no broken links)..."
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

echo ""
echo "===> Building each published crate's docs alone, with its default features (as docs.rs does)..."
for crate in sans-effort-core sans-effort-effects sans-effort-host sans-effort-tokio sans-effort; do
  RUSTDOCFLAGS="-D warnings" cargo doc -q -p "$crate" --no-deps
done

echo ""
echo "===> Linting the Nix commands' Python (ruff)..."
ruff check nix/*.py
ruff format --check nix/*.py

echo ""
echo "Done"
