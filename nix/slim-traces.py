"""`quint run --mbt` writes full ITF states, which repeat the whole model at
every step. The replay (sans-effort-host/tests/spec_traces.rs) needs only
what a host sees, so each step keeps that: its action and picks; for a
call, its output (none if the table refused it) and whether it was stale
or on the wrong worker; for `quiet`, what `wakes` held. The steps after
the verdict (`done`) go, and so do repeats: a small instance has few paths.
"""

import glob
import json
import sys

raw, instance = sys.argv[1], sys.argv[2]


def var(state, name):
    return next(v for k, v in state.items() if k.rsplit("::", 1)[-1] == name)


def grew(before, after, name):
    return int(var(after, name)["#bigint"]) > int(var(before, name)["#bigint"])


def step(before, after):
    picks = {
        k: v["value"]
        for k, v in after["mbt::nondetPicks"].items()
        if v["tag"] == "Some"
    }
    out = {"action": after["mbt::actionTaken"], "picks": picks}
    if out["action"] == "call":
        event = picks["e"]
        target = (
            event["value"] if event["tag"] == "Resume" else event["value"]["#tup"][0]
        )
        for key, flight in var(after, "in_flight")["#map"]:
            if key == target:
                out["output"] = flight
        out["stale"] = grew(before, after, "stale")
        out["wrong_thread"] = grew(before, after, "wrong_thread")
    elif out["action"] == "quiet":
        out["wakes"] = var(before, "wakes")
    return out


def trace(path):
    states = json.load(open(path))["states"]
    steps = [step(b, a) for b, a in zip(states, states[1:])]
    while steps and steps[-1]["action"] == "done":
        steps.pop()
    return steps


paths = sorted(
    glob.glob(f"{raw}/{instance}_*.itf.json"),
    key=lambda p: int(p.rsplit("_", 1)[1].split(".")[0]),
)
distinct = list({json.dumps(t, sort_keys=True): t for t in map(trace, paths)}.values())
json.dump({"instance": instance, "traces": distinct}, sys.stdout, separators=(",", ":"))
print(
    f"{instance}: {len(distinct)} traces, {sum(map(len, distinct))} steps",
    file=sys.stderr,
)
