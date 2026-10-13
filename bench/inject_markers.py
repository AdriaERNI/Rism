#!/usr/bin/env python3
"""Deterministic 20-marker transformation for the code-search bench
(E_SPEC §6; every path in the frozen table below was validated against
the SHAs in CORPUS.md — a missing target aborts, never silently re-picks).

Copies bench/.work/repos/<repo>-<sha>/ into bench/.work/export/ under the
frozen doc-name transform:  src/IORedirect/Redirect.cls  ->
BenchIOR.Redirect.cls  (`Bench<SHORT>.` + path→dots; SHORT ∈ SOS|IOR|TBX|COV),
rewrites the `Class <Old>` header to the new UDL name, and applies exactly
ONE marker to each of 20 pre-fixed target files (plain list — deterministic,
no RNG). Writes ground_truth.csv (marker,doc,line,category,sha256).

Placement (deviation D4 vs §6 "new final line"): a `//` comment AFTER the
class-closing `}` fails the IRIS parser (#5559, probe 2026-10-13), so the
marker rides as the last line INSIDE the class body (class scope, directly
before the final `}`); routines get `; RISM-BENCH-M-NN` as an EOF comment
after a prepended `ROUTINE <newname> [Type=MAC|INT]` header (raw corpus
routines without the header are rejected #16021 on doc put — probe).
`literal`-category markers are a real `Set t = "RISM-BENCH-M-NN"`
statement inside an injected trivial ClassMethod.

Stdlib only, Python >= 3.9.
"""

import csv
import hashlib
import os
import shutil
import sys

REPOS = {
    "Samples-ObjectScript": "SOS",
    "IO-Redirect": "IOR",
    "ToolBox-4-Iris": "TBX",
    "covid-19": "COV",
}

SRC_EXTS = (".cls", ".mac", ".int")

# (marker, repo-key, path-under-repo-dir, category): 12 plain + 4 literal + 4 regex
MARKERS = [
    ("RISM-BENCH-M-01", "Samples-ObjectScript", "src/ObjectScript/Person.cls", "plain"),
    ("RISM-BENCH-M-02", "Samples-ObjectScript", "src/ObjectScript/RightTriangle.cls", "plain"),
    ("RISM-BENCH-M-03", "Samples-ObjectScript", "src/ObjectScript/Lookup1.cls", "plain"),
    ("RISM-BENCH-M-04", "Samples-ObjectScript", "src/ObjectScript/Examples.cls", "plain"),
    ("RISM-BENCH-M-05", "IO-Redirect", "src/IORedirect/Redirect.cls", "plain"),
    ("RISM-BENCH-M-06", "IO-Redirect", "src/IORedirect/OutputGlobal.cls", "plain"),
    ("RISM-BENCH-M-07", "ToolBox-4-Iris", "arsblue/util/DateTime.cls", "plain"),
    ("RISM-BENCH-M-08", "ToolBox-4-Iris", "arsblue/util/Json.cls", "plain"),
    ("RISM-BENCH-M-09", "ToolBox-4-Iris", "arsblue/io/TempFileCharacter.cls", "plain"),
    ("RISM-BENCH-M-10", "ToolBox-4-Iris", "arsblue/io/File.cls", "plain"),
    ("RISM-BENCH-M-11", "covid-19", "src/evshvarov/USA/Utils.cls", "plain"),
    ("RISM-BENCH-M-12", "covid-19", "src/evshvarov/USA/Covid.cls", "plain"),
    ("RISM-BENCH-M-13", "Samples-ObjectScript", "Installer.cls", "literal"),
    ("RISM-BENCH-M-14", "ToolBox-4-Iris", "arsblue/util/Exec.cls", "literal"),
    ("RISM-BENCH-M-15", "ToolBox-4-Iris", "arsblue/io/Serializable.cls", "literal"),
    ("RISM-BENCH-M-16", "covid-19", "src/evshvarov/USA/Region.cls", "literal"),
    ("RISM-BENCH-M-17", "ToolBox-4-Iris", "arsblue/util/Calendar.cls", "regex"),
    ("RISM-BENCH-M-18", "IO-Redirect", "src/IORedirect/InputStream.cls", "regex"),
    ("RISM-BENCH-M-19", "covid-19", "src/evshvarov/Covid19/WorldKPI.cls", "regex"),
    ("RISM-BENCH-M-20", "Samples-ObjectScript", "src/ObjectScript/DataEntry1.cls", "regex"),
]

# decoys (NOT recall cases): 3 near-miss N strings + M-190 prefix-adjacent
# riding directly ABOVE M-19 in the M-19 carrier (§6).
DECOYS = [
    ("RISM-BENCH-N-21", "ToolBox-4-Iris", "arsblue/io/TempFileBinary.cls", "decoy"),
    ("RISM-BENCH-N-22", "covid-19", "src/evshvarov/USA/City.cls", "decoy"),
    ("RISM-BENCH-N-23", "IO-Redirect", "test/UnitTest/IORedirect/Redirect.cls", "decoy"),
    ("RISM-BENCH-M-190", "covid-19", "src/evshvarov/Covid19/WorldKPI.cls", "adjacent"),
]


def repo_key(dirname):
    for key in REPOS:
        if dirname == key or dirname.startswith(key + "-"):
            return key
    return None


def split_rel(rel):
    """'IO-Redirect-<sha>/src/x.cls' -> ('IO-Redirect', 'src/x.cls')."""
    top, rest = rel.split("/", 1)
    return repo_key(top), rest


def doc_name(key, rest):
    stem, ext = os.path.splitext(rest)
    return f"Bench{REPOS[key]}.{stem.replace('/', '.')}{ext}"


def norm_lines(text):
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    while lines and lines[-1].strip() == "":
        lines.pop()
    return lines


def rewrite_class_header(lines, udl):
    for i, ln in enumerate(lines[:12]):
        s = ln.lstrip()
        if s.startswith("Class "):
            indent = ln[: len(ln) - len(s)]
            rest = s[len("Class "):]
            for sep in (" Extends", " [", "{"):
                if sep in rest:
                    rest = rest[rest.index(sep):]
                    break
            else:
                rest = ""
            lines[i] = indent + f"Class {udl}{rest}"
            return True
    return False


def last_nonblank(lines):
    j = len(lines) - 1
    while j > 0 and lines[j].strip() == "":
        j -= 1
    return j


def insert_comment(lines, marker):
    close = last_nonblank(lines)
    lines.insert(close, f"// {marker}")
    return close + 1  # 1-based


def insert_literal(lines, marker):
    close = last_nonblank(lines)
    block = [
        "",
        "ClassMethod RismBenchProbe() As %Status",
        "{",
        f'\tSet t = "{marker}"',
        "\tQuit $$$OK",
        "}",
        "",
    ]
    lines[close:close] = block
    return close + 4  # the Set line, 1-based


def main(repos_dir, work_dir):
    export = os.path.join(work_dir, "export")
    if os.path.isdir(export):
        shutil.rmtree(export)
    os.makedirs(export)

    all_rels = []
    for root, dirs, files in os.walk(repos_dir):
        dirs.sort()
        for f in sorted(files):
            if f.endswith(SRC_EXTS):
                rel = os.path.relpath(os.path.join(root, f), repos_dir).replace(os.sep, "/")
                key, rest = split_rel(rel)
                # system stubs (%ZSTART.int) cannot carry a Bench package
                # prefix (a mid-name % is not a legal UDL name) — recorded
                # as excluded in CORPUS.md instead of loaded.
                if key and not os.path.basename(rest).startswith("%"):
                    all_rels.append(rel)
    all_rels.sort()
    by_key = {}
    for rel in all_rels:
        k = split_rel(rel)
        by_key[k] = rel

    for m, key, rest, _ in MARKERS:
        if (key, rest) not in by_key:
            raise SystemExit(f"FATAL: marker target missing: {m} -> {key}/{rest}")
    decoys = []
    for m, key, rest, kind in DECOYS:
        if (key, rest) in by_key:
            decoys.append((m, key, rest, kind))
        else:
            print(f"note: decoy target missing, skipped: {m} -> {key}/{rest}")

    marker_by = {(k, r): (m, c) for m, k, r, c in MARKERS}
    decoy_by = {}
    for m, k, r, _ in decoys:
        decoy_by.setdefault((k, r), []).append(m)

    rows, loaded = [], []
    for rel in all_rels:
        key, rest = split_rel(rel)
        ext = os.path.splitext(rest)[1]
        doc = doc_name(key, rest)
        with open(os.path.join(repos_dir, rel), "rb") as fh:
            text = fh.read().decode("utf-8-sig", errors="replace")
        lines = norm_lines(text)
        udl = doc.rsplit(".", 1)[0]
        if ext == ".cls":
            rewrite_class_header(lines, udl)
        else:
            if lines and lines[0].lstrip().upper().startswith("ROUTINE"):
                lines[0] = f"ROUTINE {udl} [Type={ext[1:].upper()}]"
            else:
                lines.insert(0, f"ROUTINE {udl} [Type={ext[1:].upper()}]")
        line_no = ""
        category = "corpus"
        if (key, rest) in marker_by:
            m, category = marker_by[(key, rest)]
            if ext != ".cls" and category == "literal":
                raise SystemExit(f"FATAL: literal marker on routine target {m}")
            if category == "literal":
                line_no = insert_literal(lines, m)
            elif ext == ".cls":
                line_no = insert_comment(lines, m)
            else:
                lines.append(f"; {m}")
                line_no = len(lines)
        for m in sorted(decoy_by.get((key, rest), [])):
            if ext == ".cls":
                insert_comment(lines, m)
            else:
                lines.append(f"; {m}")
        out_text = "\n".join(ln.rstrip() for ln in lines) + "\n"
        with open(os.path.join(export, doc), "w") as fh:
            fh.write(out_text)
        loaded.append(doc)
        if (key, rest) in marker_by:
            m = marker_by[(key, rest)][0]
            rows.append({
                "marker": m,
                "doc": doc,
                "line": line_no,
                "category": category,
                "sha256": hashlib.sha256(out_text.encode()).hexdigest()[:16],
            })
    gt = os.path.join(work_dir, "ground_truth.csv")
    with open(gt, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=["marker", "doc", "line", "category", "sha256"])
        w.writeheader()
        w.writerows(sorted(rows, key=lambda r: r["marker"]))
    print(f"transformed {len(loaded)} docs -> {export}; {len(rows)} markers -> {gt}")
    return loaded


if __name__ == "__main__":
    main(
        sys.argv[1] if len(sys.argv) > 1 else "bench/.work/repos",
        sys.argv[2] if len(sys.argv) > 2 else "bench/.work",
    )
