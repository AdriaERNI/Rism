# Bench results — code search + graph (issue #17)

- date: 2026-10-13 10:22 UTC
- server: IRIS via `/home/hermes/Projects/blkpws/Rism/.worktrees/coder/target/release/rism info` — {"api": 8, "base_url": "http://localhost:52773", "namespaces": ["%SYS", "USER"], "version": "IRIS for UNIX (Ubuntu Serve
- rism: `c05c306dcb`
- machine: Linux x86-64, localhost network; wall ms = CLI-process time
  incl. startup (spec §6a); rg baseline = ripgrep 15.1.0
- corpus: 80 docs loaded (38 compiled ok) — see CORPUS.md
- pre-flight probe: uncompiled source visible in literal mode:
  `True`; metadata mode: `True`;
  compiled literal: `True`

## (a) Latency (ms), 3 warmup + 30 timed per row

| query | rism p50 | rism p95 | rg p50 | rg p95 | ratio p95 (rism/rg) | rism matches | server truncated |
|---|---|---|---|---|---|---|---|
| Q1 RISM-BENCH-M-01 rare exact | 289.3 | 302.9 | 10.9 | 13.8 | 21.9x | 2 | no |
| Q2 Set *.cls common capped | 724.5 | 740.0 | 12.1 | 14.4 | 51.4x | 6 | no |
| Q3 \bQuit\b routines | 735.3 | 755.3 | 11.3 | 13.3 | 56.8x | 0 | no |
| Q4 \%SYSTEM literal | 720.0 | 742.7 | 11.3 | 13.6 | 54.6x | 0 | no |
| Q5 Error all docs | 723.3 | 751.3 | 11.5 | 13.4 | 56.1x | 12 | no |
| Q6 RISM.BENCH.M.06 dot-meta | 715.3 | 743.6 | 10.9 | 12.4 | 60.0x | 0 | no |
| Q7 Class *.cls cap cliff | 217.0 | 225.2 | 11.5 | 12.7 | 17.7x | 200 | yes |
| Q8 ^Trigger | 729.5 | 759.0 | 11.1 | 13.8 | 55.0x | 0 | no |
| Q9 zZ case default | 725.5 | 752.9 | 11.4 | 13.2 | 57.0x | 0 | no |
| Q10 Bench prefix scan | 289.7 | 295.6 | 11.3 | 13.1 | 22.6x | 83 | no |

rg wins per-query latency (direct filesystem scan, tiny corpus, no
process-startup equivalent cost); the point of the table is the ratio
band: the server search costs a fixed ~1 rism-startup + 1 HTTP round
trip and returns the SERVER's view of every namespace document, not
just a checked-out tree.

## (b) Recall — 20 markers (literal-mode scan; see D2 for why)

| marker | doc | category | compiled-ok | hit | returned docs |
|---|---|---|---|---|---|
| RISM-BENCH-M-01 | BenchSOS.src.ObjectScript.Person.cls | plain | y | HIT | BenchSOS.src.ObjectScript.Person.cls, RismCI.Probe5.cls |
| RISM-BENCH-M-02 | BenchSOS.src.ObjectScript.RightTriangle.cls | plain | y | HIT | BenchSOS.src.ObjectScript.RightTriangle.cls |
| RISM-BENCH-M-03 | BenchSOS.src.ObjectScript.Lookup1.cls | plain | y | HIT | BenchSOS.src.ObjectScript.Lookup1.cls |
| RISM-BENCH-M-04 | BenchSOS.src.ObjectScript.Examples.cls | plain | y | HIT | BenchSOS.src.ObjectScript.Examples.cls |
| RISM-BENCH-M-05 | BenchIOR.src.IORedirect.Redirect.cls | plain | n | HIT | BenchIOR.src.IORedirect.Redirect.cls |
| RISM-BENCH-M-06 | BenchIOR.src.IORedirect.OutputGlobal.cls | plain | n | HIT | BenchIOR.src.IORedirect.OutputGlobal.cls |
| RISM-BENCH-M-07 | BenchTBX.arsblue.util.DateTime.cls | plain | n | HIT | BenchTBX.arsblue.util.DateTime.cls |
| RISM-BENCH-M-08 | BenchTBX.arsblue.util.Json.cls | plain | n | HIT | BenchTBX.arsblue.util.Json.cls |
| RISM-BENCH-M-09 | BenchTBX.arsblue.io.TempFileCharacter.cls | plain | y | HIT | BenchTBX.arsblue.io.TempFileCharacter.cls |
| RISM-BENCH-M-10 | BenchTBX.arsblue.io.File.cls | plain | n | HIT | BenchTBX.arsblue.io.File.cls |
| RISM-BENCH-M-11 | BenchCOV.src.evshvarov.USA.Utils.cls | plain | y | HIT | BenchCOV.src.evshvarov.USA.Utils.cls |
| RISM-BENCH-M-12 | BenchCOV.src.evshvarov.USA.Covid.cls | plain | y | HIT | BenchCOV.src.evshvarov.USA.Covid.cls |
| RISM-BENCH-M-13 | BenchSOS.Installer.cls | literal | y | HIT | BenchSOS.Installer.cls |
| RISM-BENCH-M-14 | BenchTBX.arsblue.util.Exec.cls | literal | n | HIT | BenchTBX.arsblue.util.Exec.cls |
| RISM-BENCH-M-15 | BenchTBX.arsblue.io.Serializable.cls | literal | n | HIT | BenchTBX.arsblue.io.Serializable.cls |
| RISM-BENCH-M-16 | BenchCOV.src.evshvarov.USA.Region.cls | literal | y | HIT | BenchCOV.src.evshvarov.USA.Region.cls |
| RISM-BENCH-M-17 | BenchTBX.arsblue.util.Calendar.cls | regex | n | HIT | BenchTBX.arsblue.util.Calendar.cls |
| RISM-BENCH-M-18 | BenchIOR.src.IORedirect.InputStream.cls | regex | n | HIT | BenchIOR.src.IORedirect.InputStream.cls |
| RISM-BENCH-M-19 | BenchCOV.src.evshvarov.Covid19.WorldKPI.cls | regex | y | HIT | BenchCOV.src.evshvarov.Covid19.WorldKPI.cls |
| RISM-BENCH-M-20 | BenchSOS.src.ObjectScript.DataEntry1.cls | regex | y | HIT | BenchSOS.src.ObjectScript.DataEntry1.cls |

recall = 20/20 = 100%

| category | hits | cases |
|---|---|---|
| literal | 4 | 4 |
| plain | 12 | 12 |
| regex | 4 | 4 |

## (c) Precision (same queries)

precision = correct docs / (correct + other docs returned).

aggregate precision = 20/21 = 95% (marker queries are unique strings; decoy table below)

Decoys: `RISM-BENCH-N-*` never searched (recall-irrelevant); M-190 is
prefix-adjacent to M-19 in the same doc — a substring query for M-19
returns both lines (documented in D5), exact-literal mode reports the
full marker line so M-190's line does NOT match the marker text rule.

## (d) Truncation cliff (`Class`, *.cls)

- default max=200: match_count=200, doc_count=199, truncated=True, p50=214.3ms p95=221.1ms
- --max 500 (sent as-is): match_count=500, truncated=True
- rg occurrence count: 481 → default covers 200/481

The IRIS 2025.3 build respects max beyond 200 (D5): the 200 cap is the
RISM CLIENT default (settings.search_max_results), not a server hard
cap as issue #17 recorded for the older build. No silent paging.

## (e) Graph correctness vs bench/graph_truth.csv

| check | pass | detail |
|---|---|---|
| subclasses(A,1)=={B:1} | PASS |  |
| subclasses(A,10)=={B:1,C:2,D:2} | PASS |  |
| subclasses('BenchGraph.Mi') empty (no over-match) | PASS |  |
| members(A) exact own | PASS |  |
| members(C) own-only excludes B | PASS |  |
| members(C,inherited) ⊇ Own@C,B@B | PASS |  |
| deps(B).depends_on ⊇ A | PASS |  |
| used_by(A,1)=={B} | PASS |  |
| used_by(A,3)=={B,C,D} | PASS |  |
| used_by(B,1)=={C,D} | PASS |  |
| unsafe class name refused (guard) | PASS |  |

graph checks: 11/11

Ground-truth pairs: bench/graph_truth.csv (hand-verified); fixture classes deleted in cleanup.

