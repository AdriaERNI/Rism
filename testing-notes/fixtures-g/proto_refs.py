#!/usr/bin/env python3
"""Parse-on-fetch prototype: fetch candidate docs (literal-search narrowing),
run dep-free regex edge extraction, measure recall on the planted fixture set.

This mirrors the G_SPEC candidate algorithm so the frozen spec's recall claim
is measured, not theorised.
"""
import base64
import json
import re
import urllib.request
from urllib.parse import quote

BASE = "http://localhost:52773/api/atelier/v8"
AUTH = base64.b64encode(b"_SYSTEM:SYS").decode()

CLS = "FProbeG.Target"
MTH = "TgtMethod"

TRUTH = {
    "E1 ##class literal (Caller)": (CLS, MTH),
    "E2 ##class literal (EdgeR.mac)": (CLS, MTH),
    "E3 Do ^FProbeG.Mid (EdgeR.mac)": ("ROUTINE", "FProbeG.Mid"),
    "E4 $zobjmethod name-string (Caller)": (CLS, MTH),
    "E5 assembled + $ZF (Caller)": (CLS, MTH),
    "E6 ..M self-call (Target)": (CLS, MTH),
    "E7 trigger body (Target)": (CLS, MTH),
    "E8 SqlComputeCode (Target)": (CLS, MTH),
    "E9 macro body (FProbeG.Macros)": (CLS, MTH),
    "E10 oref call (Sub)": (CLS, MTH),
}

PATTERNS = [
    # resolved: exact ##class(C).M(
    ("resolved_class_method", re.compile(r"##class\(\s*([\w.%]+)\s*\)\s*\.\s*([%A-Za-z][\w%]*)\s*\(")),
    # resolved: ..M( self/inherited inside class bodies (class-local)
    ("resolved_self", re.compile(r"(?<![.\w])\.\.([%A-Za-z][\w%]*)\s*\(")),
    # resolved: Do ^routine / $$(^routine)
    ("resolved_routine", re.compile(r"\bDo\s+\^([\w.]+)")),
    # unresolved: "M" as a string argument (zobjmethod/zobjclass/dynamic)
    ("unresolved_namestring", re.compile(r"\$zobj\w*\([^)]*,\s*\"([%\w]+)\"")),
    # unresolved: method name inside any quoted string (eval patterns)
    ("unresolved_instring", re.compile(r"\"[^\"]*" )),
]


def get(path):
    r = urllib.request.Request(BASE + path, headers={"Authorization": "Basic " + AUTH})
    try:
        with urllib.request.urlopen(r, timeout=90) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        return json.loads(e.read() or b"{}")


def sql(q, rows=500):
    d = post("/USER/action/query", {"query": q, "maxRows": rows})
    return d["result"].get("content", [])


def post(path, body):
    r = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        method="POST",
        headers={"Content-Type": "application/json", "Authorization": "Basic " + AUTH},
    )
    return json.loads(urllib.request.urlopen(r, timeout=90).read())


def main():
    # stage 1 — candidate narrowing: literal search for the method name
    needle = MTH if MTH else CLS
    d = get(f"/USER/action/search?query={quote(needle, safe='')}"
            f"&documents={quote('*.cls,*.mac,*.int,*.inc', safe='')}&regex=0&max=500")
    hit_docs = []
    for line in d.get("console", []):
        m = re.match(r"^([^(]+)\(", line)
        if m and "(" in line and "Searching" not in line and "Found" not in line and "not found" not in line:
            doc = m.group(1)
            if doc.startswith("FProbeG"):
                hit_docs.append(doc)
    print("candidate docs from search:", sorted(set(hit_docs)))

    # stage 1b — SQL narrowing for embedded-code columns (same-target)
    sql_docs = set()
    for r in sql("SELECT DISTINCT parent FROM %Dictionary.CompiledProperty WHERE SqlComputeCode LIKE '%" + MTH + "%'"):
        sql_docs.add(r["parent"] + ".cls")

    found = {}
    unresolved = []
    # stage 2 — fetch + parse each candidate
    for doc in sorted(set(hit_docs) | sql_docs):
        c = get(f"/USER/doc/{quote(doc, safe='')}")
        content = c.get("result", {}).get("content")
        if not isinstance(content, list):
            continue
        text = "\n".join(content)
        for name, rx in PATTERNS:
            for mm in rx.finditer(text):
                if name == "unresolved_instring":
                    # any string literal containing the target tokens = unresolved dynamic candidate
                    frag = mm.group(0)
                    if MTH in frag or CLS in frag:
                        unresolved.append((doc, frag[:80]))
                        found.setdefault(f"{name}@{doc}", 0)
                    continue
                if name == "resolved_class_method":
                    if mm.group(1).lower() == CLS.lower() and mm.group(2).lower() == MTH.lower():
                        found["E1-like:##class"] = True
                elif name == "resolved_self":
                    if mm.group(1).lower() == MTH.lower() and doc.startswith(CLS):
                        found["E6-like:self"] = True
                elif name == "resolved_routine":
                    found["E3-like:routine:" + mm.group(1)] = True
                elif name == "unresolved_namestring":
                    if mm.group(1).lower() == MTH.lower():
                        found["E4-like:dynstring"] = True

    print("resolved+unresolved buckets:", json.dumps({k: v for k, v in found.items()}, default=str, indent=1)[:700])
    print("unresolved string candidates:", len(unresolved))
    for u in unresolved[:10]:
        print("   ", u)

    # map to edges
    edges = {}
    edges["E1/E2 ##class literals"] = "E1-like:##class" in found
    edges["E6 .. self"] = "E6-like:self" in found
    edges["E3 Do ^Mid"] = "E3-like:routine:FProbeG.Mid" in found
    edges["E4 $zobjmethod"] = "E4-like:dynstring" in found
    dyn = any(MTH in frag for doc, frag in unresolved)
    edges["E5/E9 dynamic-in-string (unresolved class)"] = dyn
    print("\nEDGE TABLE")
    for k, v in edges.items():
        print(f"  {'FOUND' if v else 'MISS '} {k}")


if __name__ == "__main__":
    main()
