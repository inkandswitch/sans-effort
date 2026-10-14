# The demo routines natively on tokio and on Node, and driven from Python and
# Java over the C ABI: in every mode, the four transcripts must agree. Each
# host's output goes through `tee`, so `pipefail` (which the wrapper sets)
# makes a host that fails, or overruns its time limit, fail the demo.

echo "===> Building the cdylib and the wasm module..."
cargo build -q -p greeter_cdylib
demo:wasm

for variant in "" --fanout --ping-pong --front-desk --ring --journal --deadline; do
  case "$variant" in
    "") script='alice\nbob\nquit\n' ;;
    --fanout) script='bob\ncarol\n' ;;
    --front-desk) script='alice\nbob\ncarol\n' ;;
    *) script="" ;;
  esac
  args=()
  if [ -n "$variant" ]; then args=("$variant"); fi

  # A deadline's quick worker beats a 30 s sleep; a host that fails to cancel
  # the abandoned timer waits it out, which the limit catches.
  limit=()
  if [ "$variant" = --deadline ]; then limit=(timeout 20); fi

  echo ""
  echo "===> tokio, natively (no driver) $variant"
  printf '%b' "$script" | cargo run -q -p greeter_tokio -- "${args[@]}" | tee /tmp/sans-effort-rust.txt

  echo ""
  echo "===> Python host $variant"
  "${limit[@]}" python3 demo/driven/python/main.py "${args[@]}" | tee /tmp/sans-effort-python.txt

  echo ""
  echo "===> Node, natively (no driver) $variant"
  "${limit[@]}" node demo/native/js/main.mjs "${args[@]}" | tee /tmp/sans-effort-js.txt

  echo ""
  echo "===> Java host (Panama, C ABI) $variant"
  "${limit[@]}" java --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java "${args[@]}" \
    | tee /tmp/sans-effort-java.txt

  echo ""
  diff /tmp/sans-effort-rust.txt /tmp/sans-effort-python.txt
  diff /tmp/sans-effort-rust.txt /tmp/sans-effort-js.txt
  diff /tmp/sans-effort-rust.txt /tmp/sans-effort-java.txt
  echo "Transcripts agree $variant"
done

echo ""
echo "===> Python host, a Quiet machine (ticker): only tags 4 and 5 can appear"
python3 demo/driven/python/main.py --ticker
