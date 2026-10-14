"""TLC, which `spec:check` runs on the spec's liveness property, needs
Apalache's two folds strict. As shipped they pass the accumulator on
unevaluated, and TLC re-evaluates such an argument at every use inside
ENABLED, which weak fairness needs: exponential in the spec's 8-step
`run_to_wait`, so the check never finished. Binding the accumulator in a
set comprehension evaluates it once. Writes `Apalache.tla` (patched) and
`Variants.tla` from Apalache's jar into a directory; refuses to run if
Apalache's text changed and a patch no longer matches.
"""

import sys, zipfile

jar, out = sys.argv[1], sys.argv[2]
modules = zipfile.ZipFile(jar)


def module(name):
    return modules.read("tla2sany/StandardModules/" + name).decode()


PATCHES = [
    (r"""ApaFoldSeqLeft(__Op(_,_), __v, __seq) ==
    IF __seq = <<>>
    THEN __v
    ELSE ApaFoldSeqLeft(__Op, __Op(__v, Head(__seq)), Tail(__seq))""",
     r"""ApaFoldSeqLeft(__Op(_,_), __v, __seq) ==
    IF __seq = <<>>
    THEN __v
    ELSE CHOOSE __r \in { ApaFoldSeqLeft(__Op, __acc, Tail(__seq)) :
                          __acc \in { __Op(__v, Head(__seq)) } } : TRUE"""),
    (r"""         ApaFoldSet(__Op, __Op(__v,__w), __T)""",
     r"""         CHOOSE __r \in { ApaFoldSet(__Op, __acc, __T) :
                       __acc \in { __Op(__v, __w) } } : TRUE"""),
]

apalache = module("Apalache.tla")
for lazy, strict in PATCHES:
    if apalache.count(lazy) != 1:
        sys.exit("Apalache.tla has changed: a fold to make strict was not found")
    apalache = apalache.replace(lazy, strict)
open(out + "/Apalache.tla", "w").write(apalache)
open(out + "/Variants.tla", "w").write(module("Variants.tla"))
