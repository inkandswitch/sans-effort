# What CI runs, but mutation testing and benchmarks: each step is one of the
# project's commands, or a tool CI runs directly.

echo "===> [1/7] Linting..."
lint

echo "===> [2/7] Testing..."
test:host
test:loom

echo "===> [3/7] Checking no_std..."
test:no_std

echo "===> [4/7] Checking spelling..."
typos

echo "===> [5/7] Checking licenses and advisories..."
cargo-deny check

echo "===> [6/7] Running the demo and the host checks..."
demo
demo:faults
demo:stress 3

echo "===> [7/7] Model-checking the spec..."
spec:check

echo ""
echo "All CI suites passed"
