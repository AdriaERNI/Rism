#!/usr/bin/env python3
"""Rewrite ONLY the Status column of the checklist from results/*.json.

Checklist row lines look like `| <id> | ... | <group> | <STATUS> |`. The row
bodies are never touched. Canonical checklist path is passed explicitly so the
tool can't guess. Any row id present in results/ but with no checklist line is
reported. Usage:
    python3 update_ledger.py --ledger ../../../.live-matrix/L_MATRIX.md [--results results]
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    here = Path(__file__).resolve().parent
    ap.add_argument("--ledger", default=str(here.parents[2] / "../.live-matrix/L_MATRIX.md"))
    ap.add_argument("--results", default=str(here / "results"))
    args = ap.parse_args()

    ledger = Path(args.ledger)
    if not ledger.exists():
        print(f"LEDGER NOT FOUND: {ledger}", file=sys.stderr)
        return 2

    statuses: dict[str, str] = {}
    for f in sorted(Path(args.results).glob("*.json")):
        r = json.loads(f.read_text())
        statuses[r["id"]] = r["status"]

    lines = ledger.read_text().splitlines()
    seen: set[str] = set()
    changed = 0
    out = []
    pat = re.compile(r"^\|\s*([\w-]+)\s*\|")
    for ln in lines:
        m = pat.match(ln)
        if m and m.group(1) in statuses and ln.rstrip().endswith("|"):
            rid = m.group(1)
            cells = ln.split("|")
            if len(cells) >= 3 and cells[-2].strip() in (
                    "TODO", "PASS", "FAIL", "UNKNOWN",
                    "UNKNOWN — probe on first W: B2 run decides whether CR survives storage or folds at read; the probe is put_document CRLF → get_document + execute_command `zwrite` of raw doc"):
                if cells[-2].strip() != statuses[rid]:
                    cells[-2] = f" {statuses[rid]} "
                    changed += 1
                seen.add(rid)
                ln = "|".join(cells)
        out.append(ln)

    ledger.write_text("\n".join(out) + "\n")
    missing = sorted(set(statuses) - seen)
    print(f"updated {changed} statuses; wrote {len(seen)} rows")
    if missing:
        print(f"ids with results but no checklist line: {missing}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
