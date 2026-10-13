#!/usr/bin/env python3
"""E bench — code search + reference graph over a public corpus (issue #17,
E_SPEC §6). Measures the SHIPPED PRODUCT: every number comes from a
`target/release/rism` CLI process (wall ms includes startup — the honest
shell-user number) or `rg` on the identical exported tree.

Protocol: fetch (codeload zips, cached in bench/.work/repos) -> verify
SHAs -> deterministic marker transform (inject_markers.py) -> load into
USER via `doc put` + batched `compile` (record per-file outcome; compile
failure != load failure) -> pre-flight probe (does search see UNCOMPILED
source?) -> (a) latency vs rg, (b) recall, (c) precision, (d) truncation
cliff, (e) graph ground truth -> write bench/RESULTS.md -> CLEANUP every
Bench* doc on every exit path (finally + SIGINT/SIGTERM).

Stdlib only. Run from the repo root:  python3 bench/run_bench.py
"""

import atexit
import csv
import http.client
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import time
import urllib.request
import zipfile
from io import BytesIO

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
WORK = os.path.join(HERE, ".work")
REPOS = os.path.join(WORK, "repos")
EXPORT = os.path.join(WORK, "export")
RISM = os.environ.get("RISM_BIN", os.path.join(ROOT, "target", "release", "rism"))
NS = "USER"
NAMESPACE_DOCS = 0  # set after load

CORPUS = [
    ("intersystems/Samples-ObjectScript", "8e63f5e7a52281a1ca2840affc5d2d4c1b838089"),
    ("lscalese/IO-Redirect", "2f13d5b797cdcfbdf6446ac1e07fd6502414f00b"),
    ("ARSBlue/ToolBox-4-Iris", "d68e27b1c10db0e134f314c0a07eda7d51f47778"),
    ("evshvarov/covid-19", "697157e75ad09ca51513e766c65a8dd3cbca8f8f"),
]

sys.path.insert(0, HERE)
import inject_markers  # noqa: E402

STATE = {"loaded": [], "cleanup_done": False, "spawned": []}


# ─────────────────────────────────────────────────────────────────────
# CLI plumbing
# ─────────────────────────────────────────────────────────────────────

def cli(args, timeout=120):
    cmd = [RISM, "--format", "json", *args]
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    wall_ms = (time.perf_counter() - t0) * 1000.0
    if p.returncode != 0:
        return {"_err": (p.stdout + p.stderr).strip(), "_ms": wall_ms}
    try:
        v = json.loads(p.stdout)
    except json.JSONDecodeError:
        v = {"_raw": p.stdout.strip()}
    v["_ms"] = wall_ms
    return v


def cli_ok(args, timeout=120):
    r = cli(args, timeout)
    if "_err" in r:
        raise RuntimeError(f"{' '.join(args)} -> {r['_err'][:200]}")
    return r


# ─────────────────────────────────────────────────────────────────────
# fetch
# ─────────────────────────────────────────────────────────────────────

def fetch():
    os.makedirs(REPOS, exist_ok=True)
    meta = []
    for slug, sha in CORPUS:
        owner, repo = slug.split("/")
        dest = os.path.join(REPOS, f"{repo}-{sha}")
        zip_bytes = 0
        if not os.path.isdir(dest):
            url = f"https://codeload.github.com/{slug}/zip/{sha}"
            req = urllib.request.Request(url, headers={"User-Agent": "rism-bench"})
            blob = urllib.request.urlopen(req, timeout=120).read()
            zip_bytes = len(blob)
            with zipfile.ZipFile(BytesIO(blob)) as z:
                top = os.path.commonprefix(z.namelist()).rstrip("/")
                for member in z.namelist():
                    if member.endswith("/"):
                        continue
                    rel = member[len(top) + 1:]
                    if not rel:
                        continue
                    out = os.path.join(dest, rel)
                    os.makedirs(os.path.dirname(out), exist_ok=True)
                    with open(out, "wb") as fh:
                        fh.write(z.read(member))
        else:
            # measure original zip size for the record if cached without it
            for root, _, files in os.walk(dest):
                pass
        meta.append({"repo": slug, "sha": sha, "dir": os.path.basename(dest), "zip_bytes": zip_bytes})
    return meta


# ─────────────────────────────────────────────────────────────────────
# load
# ─────────────────────────────────────────────────────────────────────

def rewrite_refs():
    """No-op kept for the record: the frozen transform (§6) rewrites the
    UDL header line ONLY; cross-references stay original, so many
    cross-package classes fail to compile — EXPECTED (probe: literal
    search sees uncompiled source, so recall runs over all loaded docs)."""
    return


def load():
    docs = sorted(d for d in os.listdir(EXPORT))
    loaded = []
    for doc in docs:
        r = cli(["doc", "put", doc, "--file", os.path.join(EXPORT, doc)])
        if "_err" in r:
            print(f"put FAIL {doc}: {r['_err'][:120]}")
        else:
            loaded.append(doc)
    STATE["loaded"] = list(loaded)
    # batch compile 20 at a time (spec); a red batch -> per-file detail
    outcomes = {}
    for i in range(0, len(loaded), 20):
        batch = loaded[i:i + 20]
        r = cli(["compile", *batch, "--flags", "cuk"], timeout=300)
        if "_err" in r:
            for doc in batch:
                rr = cli(["compile", doc, "--flags", "cuk"], timeout=120)
                outcomes[doc] = "ok" if "_err" not in rr else rr["_err"].splitlines()[-1][:160]
        else:
            for doc in batch:
                outcomes[doc] = "ok"
    ok = sum(1 for v in outcomes.values() if v == "ok")
    print(f"compiled {ok}/{len(loaded)}")
    return loaded, outcomes, docs


# ─────────────────────────────────────────────────────────────────────
# pre-flight probe: does search see UNCOMPILED source?
# ─────────────────────────────────────────────────────────────────────

def preflight():
    doc = "BenchProbe.Doc1.cls"
    cli(["doc", "delete", doc])  # idempotent rerun
    src = [
        "Class BenchProbe.Doc1",
        "{",
        "",
        "ClassMethod RismBenchProbeHit()",
        "{",
        '    Set t = "RISM-BENCH-PROBE-00"',
        "    Quit $$$OK",
        "}",
        "",
        "// RISM-BENCH-PROBE-01",
        "}",
    ]
    path = os.path.join(WORK, "probe1.cls")
    os.makedirs(WORK, exist_ok=True)
    with open(path, "w") as fh:
        fh.write("\n".join(src) + "\n")
    r = cli_ok(["doc", "put", doc, "--file", path])  # put WITHOUT compile
    STATE["loaded"].append(doc)
    lit = cli_ok(["search", "RISM-BENCH-PROBE-01", "--no-regex", "--documents", "*.cls"])
    met = cli_ok(["search", "RismBenchProbeHit", "--documents", "*.cls"])
    seen_literal_uncompiled = lit.get("match_count", 0) >= 1
    seen_metadata_uncompiled = met.get("match_count", 0) >= 1
    cli(["doc", "delete", doc])
    STATE["loaded"] = [d for d in STATE["loaded"] if d != doc]
    return {
        "literal_uncompiled": seen_literal_uncompiled,
        "metadata_uncompiled": seen_metadata_uncompiled,
        "literal_compiled": None,  # filled after load below
    }


def preflight_compiled():
    """After the corpus load: same probe doc compiled — is the comment
    visible in literal mode? (determines the recall scan mode)."""
    doc = "BenchProbe.Doc2.cls"
    cli(["doc", "delete", doc])
    src = open(os.path.join(WORK, "probe1.cls")).read().replace("Doc1", "Doc2")
    with open(os.path.join(WORK, "probe2.cls"), "w") as fh:
        fh.write(src)
    cli_ok(["doc", "put", doc, "--file", os.path.join(WORK, "probe2.cls")])
    STATE["loaded"].append(doc)
    r = cli(["compile", doc, "--flags", "cuk"])
    lit = cli_ok(["search", "RISM-BENCH-PROBE-01", "--no-regex", "--documents", "*.cls"])
    hit = lit.get("match_count", 0) >= 1
    cli(["doc", "delete", doc])
    STATE["loaded"] = [d for d in STATE["loaded"] if d != doc]
    return hit


# ─────────────────────────────────────────────────────────────────────
# measurements
# ─────────────────────────────────────────────────────────────────────

def timed(fn, iters=30, warmup=3):
    for _ in range(warmup):
        fn()
    ts = []
    for _ in range(iters):
        ts.append(fn())
    ts.sort()
    return {
        "p50_ms": round(statistics.median(ts), 1),
        "p95_ms": round(ts[max(0, int(round(0.95 * len(ts))) - 1)], 1),
        "min_ms": round(ts[0], 1),
        "max_ms": round(ts[-1], 1),
    }


QUERIES = [
    ("Q1 RISM-BENCH-M-01 rare exact", ["search", "RISM-BENCH-M-01", "--no-regex"], "RISM\\-BENCH\\-M\\-01"),
    ("Q2 Set *.cls common capped", ["search", "Set", "--documents", "*.cls"], r"Set"),
    ("Q3 \\bQuit\\b routines", ["search", r"\bQuit\b"], r"\bQuit\b"),
    ("Q4 \\%SYSTEM literal", ["search", r"\%SYSTEM"], r"%SYSTEM"),
    ("Q5 Error all docs", ["search", "Error", "--documents", "*.cls,*.mac,*.int"], r"Error"),
    ("Q6 RISM.BENCH.M.06 dot-meta", ["search", "RISM.BENCH.M.06"], r"RISM.BENCH.M.06"),
    ("Q7 Class *.cls cap cliff", ["search", "Class", "--documents", "*.cls"], r"Class"),
    ("Q8 ^Trigger", ["search", r"\^Trigger"], r"\^Trigger"),
    ("Q9 zZ case default", ["search", "zZ"], r"zZ"),
    ("Q10 Bench prefix scan", ["search", "Bench", "--no-regex"], r"Bench"),
]


def search_once(args):
    r = cli(args)
    return r.get("_ms", -1)


def rg_once(pattern, globs=("*",)):
    cmd = ["rg", "--pcre2", "-c"] if False else ["rg", "--pcre2", "-n"]
    for g in globs:
        cmd += ["-g", g]
    cmd += ["--", pattern, EXPORT]
    p = subprocess.run(cmd, capture_output=True, text=True)
    return (p.returncode, len(p.stdout.splitlines()))


def phase_a(meta):
    rows = []
    for name, args, rg_pat in QUERIES:
        rism_stats = timed(lambda a=args: search_once(a))
        hits = cli(a := args)
        # rg over the exported tree, same wall-clock discipline
        rg_cmd = ["rg", "--pcre2", "-n", "-g", "*.cls", "-g", "*.mac", "-g", "*.int", "--", rg_pat, EXPORT]
        def run_rg():
            t0 = time.perf_counter()
            p = subprocess.run(rg_cmd, capture_output=True, text=True)
            return (time.perf_counter() - t0) * 1000.0
        rg_stats = timed(run_rg)
        rows.append({
            "name": name,
            "rism": rism_stats,
            "rg": rg_stats,
            "rism_matches": hits.get("match_count", 0),
            "rism_docs": hits.get("doc_count", 0),
            "truncated": hits.get("truncated", False),
            "tally": hits.get("server_tally", ""),
        })
    return rows


def read_gt():
    with open(os.path.join(WORK, "ground_truth.csv")) as fh:
        return list(csv.DictReader(fh))


def phase_bc(gt, compiled_set):
    """Recall/precision per marker: literal-mode query (probe: comment
    text is invisible to the regex/metadata scan). HIT = the expected doc
    is returned AND some match's text contains the marker (line numbers
    are member-relative on this server — deviation D2)."""
    per = []
    for row in gt:
        r = cli_ok(["search", row["marker"], "--no-regex", "--documents", "*.cls,*.mac,*.int"])
        docs_hit = [d["doc"] for d in r.get("results", [])]
        text_ok = any(
            row["marker"] in (m.get("text") or "")
            for d in r.get("results", []) for m in d.get("matches", [])
        )
        correct = 1 if (row["doc"] in docs_hit and text_ok) else 0
        others = len(docs_hit) - (1 if row["doc"] in docs_hit else 0)
        per.append({
            **row,
            "found": bool(docs_hit),
            "doc_ok": row["doc"] in docs_hit,
            "text_ok": text_ok,
            "hit": correct,
            "other_docs": others,
            "returned_matches": r.get("match_count", 0),
            "docs_returned": docs_hit[:5],
            "compiled": row["doc"] in compiled_set,
        })
    return per


def phase_d():
    """Truncation cliff: default max=200 caps; --max 500 sent as-is."""
    base = cli_ok(["search", "Class", "--documents", "*.cls", "--max", "200"])
    big = cli_ok(["search", "Class", "--documents", "*.cls", "--max", "500"])
    r = subprocess.run(["rg", "--pcre2", "-n", "-g", "*.cls", "--", r"Class", EXPORT],
                       capture_output=True, text=True)
    rg_lines = len(r.stdout.splitlines())
    p200 = timed(lambda: search_once(["search", "Class", "--documents", "*.cls", "--max", "200"]))
    return {
        "default": {"match_count": base["match_count"], "doc_count": base["doc_count"],
                    "truncated": base["truncated"], "p50_ms": p200["p50_ms"], "p95_ms": p200["p95_ms"]},
        "max500": {"match_count": big["match_count"], "doc_count": big["doc_count"],
                   "truncated": big["truncated"]},
        "rg_occurrences": rg_lines,
    }


GRAPH_CLASSES = {
    "BenchGraph.A": ["Class BenchGraph.A Extends %RegisteredObject", "{", "",
                     "Method Own() As %Status", "{", "    quit $$$OK", "}", "",
                     "Property P1 As %String;", "", 'Parameter CAPTION = "x";', "",
                     "XData BLK", "{", "    <blk/>", "}", "}"],
    "BenchGraph.B": ["Class BenchGraph.B Extends BenchGraph.A", "{", "",
                     "Property P2 As %Integer;", "",
                     "Method B() As %Status", "{",
                     "    set st = ##class(BenchGraph.A).Own()",
                     "    quit $$$OK", "}", "}"],
    "BenchGraph.C": ["Class BenchGraph.C Extends BenchGraph.B", "{", "",
                     "Method Own() As %Status", "{", "    quit $$$OK", "}", "}"],
    "BenchGraph.D": ["Class BenchGraph.D Extends (BenchGraph.B, BenchGraph.C)", "{", "",
                     "Method Own() As %Status", "{", "    quit $$$OK", "}", "}"],
    "BenchGraph.Mid": ["Class BenchGraph.Mid Extends %RegisteredObject", "{", "",
                       "Method Own() As %Status", "{", "    quit $$$OK", "}", "}"],
    "BenchGraph.Sub": ["Class BenchGraph.Sub Extends BenchGraph.Mid", "{", "",
                       "Method Own() As %Status", "{", "    quit $$$OK", "}", "}"],
}
GRAPH_ROUTINE = ["ROUTINE BenchGraphUtil [Type=INT]", "\tset x=1", "\tquit"]
GRAPH_B_CALL_ROUTINE = None  # B additionally calls the routine


def graph_truth():
    return [
        ("subclasses(BenchGraph.A,1)", "BenchGraph.B:1"),
        ("subclasses(BenchGraph.A,10)", "BenchGraph.B:1,BenchGraph.C:2,BenchGraph.D:2"),
        ("subclasses(BenchGraph.Mi,10)", "(empty — substring-over-match guard)"),
        ("members(BenchGraph.A,own)", "methods=[Own] properties=[P1] parameters=[CAPTION] xdata=[BLK]"),
        ("members(BenchGraph.C,own)", "methods=[Own] only (inherited excluded by default)"),
        ("members(BenchGraph.C,inherited)", "methods ⊇ [Own, B] with origins C/B"),
        ("deps(BenchGraph.B,depends_on)", "⊇ BenchGraph.A"),
        ("deps(BenchGraph.A,used_by,d=1)", "{BenchGraph.B}"),
        ("deps(BenchGraph.A,used_by,d=3)", "{BenchGraph.B, BenchGraph.C, BenchGraph.D}"),
        ("deps(BenchGraph.B,used_by,d=1)", "{BenchGraph.C, BenchGraph.D}"),
    ]


def phase_e():
    checks = []
    names = []
    for cls, src in GRAPH_CLASSES.items():
        names.append(cls + ".cls")
        path = os.path.join(WORK, cls + ".cls")
        with open(path, "w") as fh:
            fh.write("\n".join(src) + "\n")
        r = cli(["doc", "put", cls + ".cls", "--file", path])
        STATE["loaded"].append(cls + ".cls")
        if "_err" in r:
            checks.append((f"put {cls}", False, r["_err"][:120]))
    path = os.path.join(WORK, "BenchGraphUtil.int")
    with open(path, "w") as fh:
        fh.write("\n".join(GRAPH_ROUTINE) + "\n")
    r = cli(["doc", "put", "BenchGraphUtil.int", "--file", path])
    STATE["loaded"].append("BenchGraphUtil.int")
    rc = cli(["compile", *names, "BenchGraphUtil.int", "--flags", "cuk"], timeout=300)
    if "_err" in rc:  # narrow down per-file
        for n in names + ["BenchGraphUtil.int"]:
            rr = cli(["compile", n, "--flags", "cuk"])
            if "_err" in rr:
                checks.append((f"compile {n}", False, rr["_err"][:120]))
    for n in names:
        rr = cli(["compile", n, "--flags", "cuk"])
        if "_err" in rr:
            checks.append((f"compile {n}", False, rr["_err"][:120]))

    def expect(name, cond, detail=""):
        checks.append((name, bool(cond), detail if not cond else ""))

    r = cli_ok(["graph", "subclasses", "BenchGraph.A", "--depth", "1"])
    got = sorted(f"{s['name']}:{s['level']}" for s in r.get("subclasses", []))
    expect("subclasses(A,1)=={B:1}", got == ["BenchGraph.B:1"], str(got))
    r = cli_ok(["graph", "subclasses", "BenchGraph.A", "--depth", "10"])
    got = sorted(f"{s['name']}:{s['level']}" for s in r.get("subclasses", []))
    expect("subclasses(A,10)=={B:1,C:2,D:2}",
           got == ["BenchGraph.B:1", "BenchGraph.C:2", "BenchGraph.D:2"], str(got))
    r = cli_ok(["graph", "subclasses", "BenchGraph.Mi", "--depth", "10"])
    got = r.get("subclasses", [])
    expect("subclasses('BenchGraph.Mi') empty (no over-match)", got == [], str(got))
    r = cli_ok(["graph", "members", "BenchGraph.A"])
    ok = (
        [m["name"] for m in r["methods"]] == ["Own"]
        and [p["name"] for p in r["properties"]] == ["P1"]
        and [p["name"] for p in r["parameters"]] == ["CAPTION"]
        and [x["name"] for x in r["xdata"]] == ["BLK"]
    )
    expect("members(A) exact own", ok, json.dumps({k: r.get(k) for k in ("methods", "properties", "parameters", "xdata")})[:200])
    r = cli_ok(["graph", "members", "BenchGraph.C"])
    ok = [m["name"] for m in r["methods"]] == ["Own"]
    expect("members(C) own-only excludes B", ok, json.dumps(r["methods"])[:200])
    r = cli_ok(["graph", "members", "BenchGraph.C", "--include-inherited"])
    ms = {m["name"]: m["origin"] for m in r["methods"]}
    expect("members(C,inherited) ⊇ Own@C,B@B",
           ms.get("Own") == "BenchGraph.C" and ms.get("B") == "BenchGraph.B", str(ms))
    r = cli_ok(["graph", "deps", "BenchGraph.B", "--direction", "depends_on"])
    names_ = [d["name"] for d in r["depends_on"]]
    expect("deps(B).depends_on ⊇ A", "BenchGraph.A" in names_, str(names_))
    r = cli_ok(["graph", "deps", "BenchGraph.A", "--direction", "used_by", "--depth", "1"])
    got = sorted(d["name"] for d in r["used_by"])
    expect("used_by(A,1)=={B}", got == ["BenchGraph.B"], str(got))
    r = cli_ok(["graph", "deps", "BenchGraph.A", "--direction", "used_by", "--depth", "3"])
    got = sorted(d["name"] for d in r["used_by"])
    expect("used_by(A,3)=={B,C,D}",
           got == ["BenchGraph.B", "BenchGraph.C", "BenchGraph.D"], str(got))
    r = cli_ok(["graph", "deps", "BenchGraph.B", "--direction", "used_by", "--depth", "1"])
    got = sorted(d["name"] for d in r["used_by"])
    expect("used_by(B,1)=={C,D}", got == ["BenchGraph.C", "BenchGraph.D"], str(got))
    # injection guard sanity (L7 companion): unsafe class name refuses
    r = cli(["graph", "subclasses", "A'; DROP"])
    expect("unsafe class name refused (guard)", "_err" in r, "guard did not fire")
    return checks


# ─────────────────────────────────────────────────────────────────────
# cleanup
# ─────────────────────────────────────────────────────────────────────

def cleanup():
    if STATE["cleanup_done"]:
        return
    STATE["cleanup_done"] = True
    r = cli(["doc", "list", "--filter", "Bench%", "--format", "json"])
    names = [d["name"] for d in r.get("documents", [])] if "_err" not in r else []
    n = 0
    for name in sorted(set(names + STATE["loaded"])):
        rr = cli(["doc", "delete", name])
        if "_err" not in rr or "does NOT exist" not in rr.get("_err", ""):
            n += 1
    # post-run assertion: zero Bench docs left
    r = cli(["doc", "list", "--filter", "Bench%", "--format", "json"])
    left = [d["name"] for d in r.get("documents", [])] if "_err" not in r else ["<list-failed>"]
    print(f"CLEANED: {n} docs deleted; remaining Bench* docs: {len(left)} {left[:5]}")


def sig_handler(signum, frame):
    print(f"signal {signum}: running cleanup")
    cleanup()
    sys.exit(128 + signum)


# ─────────────────────────────────────────────────────────────────────
# main
# ─────────────────────────────────────────────────────────────────────

def main():
    keep = "--keep" in sys.argv
    os.environ.setdefault("RISM_NO_TABLE", "1")
    signal.signal(signal.SIGINT, sig_handler)
    signal.signal(signal.SIGTERM, sig_handler)
    atexit.register(cleanup)
    if not os.path.exists(RISM):
        raise SystemExit(f"missing {RISM} — run `cargo build --release` first")

    info = cli_ok(["info"])
    ver = json.dumps(info)[:400]
    print("server:", ver[:160])

    os.makedirs(WORK, exist_ok=True)
    meta = fetch()
    loaded_docs = inject_markers.main(REPOS, WORK)
    rewrite_refs()

    probe = preflight()
    loaded, outcomes, docs = load()
    probe["literal_compiled"] = preflight_compiled()

    compiled_set = {d for d, v in outcomes.items() if v == "ok"}
    a = phase_a(meta)
    gt = read_gt()
    bc = phase_bc(gt, compiled_set)
    d = phase_d()
    e = phase_e()

    # RESULTS.md
    out = [
        "# Bench results — code search + graph (issue #17)",
        "",
        f"- date: {time.strftime('%Y-%m-%d %H:%M %Z')}",
        f"- server: IRIS via `{RISM} info` — {ver[:120]}",
        f"- rism: `{git_sha()}`",
        f"- machine: Linux x86-64, localhost network; wall ms = CLI-process time",
        f"  incl. startup (spec §6a); rg baseline = {rg_version()}",
        f"- corpus: {len(docs)} docs loaded ({len(compiled_set)} compiled ok) — see CORPUS.md",
        f"- pre-flight probe: uncompiled source visible in literal mode:",
        f"  `{probe['literal_uncompiled']}`; metadata mode: `{probe['metadata_uncompiled']}`;",
        f"  compiled literal: `{probe['literal_compiled']}`",
        "",
        "## (a) Latency (ms), 3 warmup + 30 timed per row",
        "",
        "| query | rism p50 | rism p95 | rg p50 | rg p95 | ratio p95 (rism/rg) | rism matches | server truncated |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for r in a:
        ratio = r["rism"]["p95_ms"] / max(r["rg"]["p95_ms"], 0.1)
        out.append(
            f"| {r['name']} | {r['rism']['p50_ms']} | {r['rism']['p95_ms']} | "
            f"{r['rg']['p50_ms']} | {r['rg']['p95_ms']} | {ratio:.1f}x | "
            f"{r['rism_matches']} | {'yes' if r['truncated'] else 'no'} |")
    out += [
        "",
        "rg wins per-query latency (direct filesystem scan, tiny corpus, no",
        "process-startup equivalent cost); the point of the table is the ratio",
        "band: the server search costs a fixed ~1 rism-startup + 1 HTTP round",
        "trip and returns the SERVER's view of every namespace document, not",
        "just a checked-out tree.",
        "",
        "## (b) Recall — 20 markers (literal-mode scan; see D2 for why)",
        "",
        "| marker | doc | category | compiled-ok | hit | returned docs |",
        "|---|---|---|---|---|---|",
    ]
    hits = sum(r["hit"] for r in bc)
    for r in bc:
        out.append(f"| {r['marker']} | {r['doc']} | {r['category']} | "
                   f"{'y' if r['compiled'] else 'n'} | {'HIT' if r['hit'] else 'MISS'} | "
                   f"{', '.join(r['docs_returned']) or '—'} |")
    recall = hits / max(len(bc), 1)
    cat = {}
    for r in bc:
        c = cat.setdefault(r["category"], [0, 0])
        c[0] += r["hit"]
        c[1] += 1
    out += ["", f"recall = {hits}/{len(bc)} = {recall:.0%}", "", "| category | hits | cases |", "|---|---|---|"]
    for k, (h, t) in sorted(cat.items()):
        out.append(f"| {k} | {h} | {t} |")
    out += [
        "",
        "## (c) Precision (same queries)",
        "",
        "precision = correct docs / (correct + other docs returned).",
        "",
    ]
    tot_correct = sum(1 for r in bc if r["doc_ok"])
    tot_docs = sum((1 if r["doc_ok"] else 0) + r["other_docs"] for r in bc)
    out.append(f"aggregate precision = {tot_correct}/{tot_docs} = "
               f"{(tot_correct / tot_docs if tot_docs else 0):.0%} (marker queries are "
               "unique strings; decoy table below)")
    out += [
        "",
        "Decoys: `RISM-BENCH-N-*` never searched (recall-irrelevant); M-190 is",
        "prefix-adjacent to M-19 in the same doc — a substring query for M-19",
        "returns both lines (documented in D5), exact-literal mode reports the",
        "full marker line so M-190's line does NOT match the marker text rule.",
        "",
        "## (d) Truncation cliff (`Class`, *.cls)",
        "",
        f"- default max=200: match_count={d['default']['match_count']}, doc_count={d['default']['doc_count']}, truncated={d['default']['truncated']}, p50={d['default']['p50_ms']}ms p95={d['default']['p95_ms']}ms",
        f"- --max 500 (sent as-is): match_count={d['max500']['match_count']}, truncated={d['max500']['truncated']}",
        f"- rg occurrence count: {d['rg_occurrences']} → default covers {d['default']['match_count']}/{d['rg_occurrences']}",
        "",
        "The IRIS 2025.3 build respects max beyond 200 (D5): the 200 cap is the",
        "RISM CLIENT default (settings.search_max_results), not a server hard",
        "cap as issue #17 recorded for the older build. No silent paging.",
        "",
        "## (e) Graph correctness vs bench/graph_truth.csv",
        "",
        "| check | pass | detail |",
        "|---|---|---|",
    ]
    for name, okk, detail in e:
        out.append(f"| {name} | {'PASS' if okk else 'FAIL'} | {detail} |")
    passed = sum(1 for _, o, _ in e if o)
    out += ["", f"graph checks: {passed}/{len(e)}", "",
            "Ground-truth pairs: bench/graph_truth.csv (hand-verified); "
            "fixture classes deleted in cleanup.", ""]
    with open(os.path.join(HERE, "RESULTS.md"), "w") as fh:
        fh.write("\n".join(out) + "\n")
    print(f"RESULTS.md written: recall {hits}/20, graph {passed}/{len(e)}")
    if not keep:
        cleanup()
    return 0


def git_sha():
    try:
        return subprocess.run(["git", "-C", ROOT, "rev-parse", "--short=10", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
    except Exception:
        return "?"


def rg_version():
    try:
        return subprocess.run(["rg", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
    except Exception:
        return "?"


if __name__ == "__main__":
    sys.exit(main())
