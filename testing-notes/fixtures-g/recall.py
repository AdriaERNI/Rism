#!/usr/bin/env python3
"""Recall measurement: which planted call edges does a search variant find?

Truth = 10 planted edges toward FProbeG.Target (and its routine ^FProbeG.Mid)
in FProbeG.* fixtures in USER on rism-iris. An edge is FOUND when a match in
the named doc contains the edge's distinguishing substring.
"""
import json
import subprocess

BIN = "/home/hermes/Projects/blkpws/Rism/.worktrees/spec/target/release/rism"

TRUTH = {
    "E1 ##class literal in .cls (Caller.Edge1)": ("FProbeG.Caller.cls", "##class(FProbeG.Target).TgtMethod(1)"),
    "E2 ##class literal in .mac (EdgeR)": ("FProbeG.EdgeR.mac", "##class(FProbeG.Target).TgtMethod(3)"),
    "E3 Do ^routine in .mac (EdgeR)": ("FProbeG.EdgeR.mac", "Do ^FProbeG.Mid"),
    "E4 $zobjmethod dynamic name (Caller.Edge4)": ("FProbeG.Caller.cls", '"TgtMethod", 4'),
    "E5 assembled + $ZF eval (Caller.Edge5)": ("FProbeG.Caller.cls", "_cls_\").TgtMethod(5)"),
    "E6 instance self-call .. (Target.MInst)": ("FProbeG.Target.cls", "..TgtMethod(6)"),
    "E7 trigger body call (Target.trg1)": ("FProbeG.Target.cls", ".TgtMethod(7)"),
    "E8 SqlComputeCode call (Target.P9)": ("FProbeG.Target.cls", "= ##class(FProbeG.Target).TgtMethod(8)"),
    "E9 macro definition body (.inc-as-mac)": ("FProbeG.Macros", "TgtMethod(5)"),
    "E10 oref-variable call (Sub.Edge10)": ("FProbeG.Sub.cls", "o.TgtMethod(10)"),
}
DECOY = ("FProbeG.EdgeR.mac", "TgtMethod is the name we are hunting")


def run(args):
    out = subprocess.run([BIN] + args, capture_output=True, text=True, timeout=120)
    if out.returncode != 0:
        return {"results": [], "error": (out.stderr or out.stdout)[:200]}
    return json.loads(out.stdout)


def measure(name, args):
    res = run(args)
    if "error" in res:
        print(f"\n## {name}\n   ERROR {res['error']}")
        return
    hits = [(r["doc"], m.get("text", "")) for r in res.get("results", []) for m in r.get("matches", [])]
    found = {e for e, (d, f) in TRUTH.items() if any((doc == d or doc.startswith(d + ".")) and f in t for doc, t in hits)}
    decoy = any(doc == DECOY[0] and DECOY[1] in t for doc, t in hits)
    print(f"\n## {name}")
    print(f"   matches={len(hits)} docs={res.get('doc_count')} tally={res.get('server_tally')!r} truncated={res.get('truncated')}")
    for e in sorted(TRUTH):
        print(f"   {'FOUND' if e in found else 'MISS '} {e}")
    print(f"   RECALL {len(found)}/{len(TRUTH)} = {100*len(found)//len(TRUTH)}%  decoy-hit(precision cost): {decoy}")


if __name__ == "__main__":
    measure("A workaround: literal exact '##class(FProbeG.Target).TgtMethod'",
            ["search", "##class(FProbeG.Target).TgtMethod", "--no-regex", "--format", "json"])
    measure("B workaround widened: literal 'TgtMethod', default masks",
            ["search", "TgtMethod", "--no-regex", "--format", "json"])
    measure("C workaround widened + .inc mask",
            ["search", "TgtMethod", "--no-regex", "--documents", "*.cls,*.mac,*.int,*.inc", "--format", "json"])
