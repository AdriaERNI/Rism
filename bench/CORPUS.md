# Bench corpus — public ObjectScript repos (E_SPEC §6)

First benchmark in this repo (issue #17). All four repos were
license+size verified live by the specifier lane via `gh api` on
2026-10-12 (all MIT, <100 ObjectScript source files each). The coder
re-fetched each SHA at run time via codeload (`https://codeload.github.com/<owner>/<repo>/zip/<sha>`)
and records the per-run additions below. Corpus bytes are NOT committed —
everything under `bench/.work/` is gitignored.

| repo | URL | SHA | license | src files (.cls/.mac/.int) | zip bytes | tree bytes | fetch date |
|---|---|---|---|---|---|---|---|
| intersystems/Samples-ObjectScript | https://github.com/intersystems/Samples-ObjectScript | 8e63f5e7a52281a1ca2840affc5d2d4c1b838089 | MIT | 11 | 23,613 | 47,041 | 2026-10-13 |
| lscalese/IO-Redirect | https://github.com/lscalese/IO-Redirect | 2f13d5b797cdcfbdf6446ac1e07fd6502414f00b | MIT | 10 (+1 excluded dup? no: 10 loaded) | 26,079 | 80,647 | 2026-10-13 |
| ARSBlue/ToolBox-4-Iris | https://github.com/ARSBlue/ToolBox-4-Iris | d68e27b1c10db0e134f314c0a07eda7d51f47778 | MIT | 30 (+2 `%ZSTART`/`%ZSTOP` stubs excluded — see Load) | 120,782 | 391,358 | 2026-10-13 |
| evshvarov/covid-19 | https://github.com/evshvarov/covid-19 | 697157e75ad09ca51513e766c65a8dd3cbca8f8f | MIT | 29 | 4,063,357 | 14,996,406 | 2026-10-13 |

Specifier-file-count note: the §6 table lists 11 src files for
IO-Redirect; live count at the SHA is 10 (.cls×9 + src/test.mac) + 1
dup-name test class — the file-count delta is the specifier counting the
`.inc` (9 cls + 1 mac + 1 inc). Nothing else deviates.

## Transform (frozen, §6)

- doc name: `<path under repo>/Foo/Bar.cls` → `Bench<SHORT>.<Foo.Bar>.cls`
  with SHORT ∈ {SOS (Samples-ObjectScript), IOR (IO-Redirect),
  TBX (ToolBox-4-Iris), COV (covid-19)}; the `Class` header line's UDL
  name is rewritten to match. Cross-references to OTHER classes keep the
  ORIGINAL names (deviation note in E_BUILD: the spec's "package prefix
  ONLY" transform — full-corpus reference rewriting would be a second,
  riskier transform; unresolvable references simply fail to compile and
  are recorded as such, which §6 explicitly tolerates).
- routines get a synthetic `ROUTINE <newname> [Type=MAC|INT]` first line:
  the raw corpus .int files have no header line at all (they arrive from
  Studio archives); the Atelier doc-put endpoint rejects a headerless
  routine ("#16021 Illegal Header Line" — probe 2026-10-13).
- two `%ZSTART`/`%ZSTOP` system stubs (ToolBox) are EXCLUDED from the
  load: a mid-name `%` is not a legal UDL component, so they cannot carry
  the Bench prefix (§6's transform contract wins over §6's count — the
  exclusion is listed here per the "record per-file" rule).
- `.inc` include files (9 across the corpus) are not loadable documents
  (they are Studio include text, no cat); not loaded, not counted.

## Markers (§6, deterministic)

20 markers `RISM-BENCH-M-01..20` — one per pre-fixed target file
(frozen table in `inject_markers.py`, every path re-validated against
the tree at the SHA; a missing path ABORTS the run). Placement: last
line INSIDE the class body (a comment after the class-closing `}` does
not parse — #5559 — deviation D4); `literal`-category markers are a
`Set t = "..."` inside an injected trivial ClassMethod; routines carry
`; RISM-BENCH-M-NN` as an EOF comment. Decoys: `RISM-BENCH-N-21..23` +
`RISM-BENCH-M-190` directly above M-19 in the same file. Ground truth:
`bench/.work/ground_truth.csv` (machine-derived).

## Probe (§6 load reality-check) — 2026-10-13, IRIS 2025.3, USER

- put WITHOUT compile: literal-mode (`regex=0`) search sees the
  uncompiled source; regex-mode (metadata) search sees member names of
  UNCOMPILED classes too (the metadata scan parses UDL, not compiled
  artifacts). → recall runs over ALL loaded docs; per §6 the compiled
  split is printed in RESULTS.md numbers.
- compile outcome is recorded per file in the RESULTS run log.
