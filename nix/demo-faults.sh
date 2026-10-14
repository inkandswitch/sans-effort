# Host checks: routines that misbehave on purpose (`routines::faults`), and
# what every host must do about them. Not part of `demo`, which shows how
# routines are written; these check the hosts.

cargo build -q -p greeter_cdylib
demo:wasm

# A deadlock: two machines each waiting for the other. The driven hosts
# report the stall on stderr and exit 1; Node's event loop runs dry and it
# exits 13 (an unsettled top-level await). tokio has no detector: it would
# hang, so it is not run. A host that hangs instead hits the time limit.
echo "===> A deadlock (--deadlock): reported, not hung on"
set +o errexit
timeout 20 python3 demo/driven/python/main.py --deadlock \
  > /tmp/sans-effort-python.txt 2> /tmp/sans-effort-python-err.txt
python_exit=$?
timeout 20 java --enable-native-access=ALL-UNNAMED demo/driven/java/Main.java --deadlock \
  > /tmp/sans-effort-java.txt 2> /tmp/sans-effort-java-err.txt
java_exit=$?
timeout 20 node demo/native/js/main.mjs --deadlock > /tmp/sans-effort-js.txt 2> /dev/null
js_exit=$?
set -o errexit
cat /tmp/sans-effort-python.txt /tmp/sans-effort-python-err.txt
if [ "$python_exit $java_exit $js_exit" != "1 1 13" ]; then
  echo "exit codes: Python $python_exit, Java $java_exit, Node $js_exit; expected 1, 1, 13"
  exit 1
fi
diff /tmp/sans-effort-python.txt /tmp/sans-effort-java.txt
diff /tmp/sans-effort-python.txt /tmp/sans-effort-js.txt
diff /tmp/sans-effort-python-err.txt /tmp/sans-effort-java-err.txt
echo "Deadlock reported --deadlock"
