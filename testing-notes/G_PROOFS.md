# G PROOFS — planted-edge recall + latency (mission G, coder lane)

Server: IRIS for UNIX 2025.3 (Build 226U), container `rism-iris`, namespace
USER — the same corpus G_RESEARCH measured (1,577 user classes / 5,916 docs).
Fixture set: the 10 planted edges + decoy in `fixtures-g/` — byte-identical
docs (only the legal `ROUTINE <name>` header line added on the `.mac` carriers;
the research lane's harness normalized the same way, verified 2026-10-15: a
bare-label first line stores as ERROR #16021 and the doc then exists but is
absent from StudioOpenDialog).

## Reproduction

```
# recall matrix + latency one-liner (loads, compiles, queries, deletes,
# verifies census 0 via %Dictionary + StudioOpenDialog):
cargo test --test tool_matrix -- --ignored find_references_live_recall --nocapture --test-threads=1

# baseline workaround on the same docs (per-edge exact/widened search):
# see G_RESEARCH §results; harness = fixtures-g/recall.py

# manual single query once fixtures are loaded:
./target/debug/rism graph refs FProbeG.Target.TgtMethod --format json
./target/debug/rism graph refs '^FProbeG.Mid'           --format json
./target/debug/rism graph refs FProbeG.Target           --format json
./target/debug/rism graph refs FProbeG.Target.TgtMethod --system --format json
```

## Recall vs the G_SPEC §5 table — all gates PASS

| query | required | measured | verdict |
|---|---|---|---|
| `FProbeG.Target.TgtMethod` | E1 E2 E6 resolved | E1 Caller.cls `class_call` L7 · E2 EdgeR.mac `class_call` L7 · E6 Target.cls `self_call` L19 | PASS |
| | E7 E8 resolved | E7 Target.cls `trigger` L25 · E8 Target.cls `computed` L29 (SQL-narrowed carrier — literal search does NOT surface this doc) | PASS |
| | E9 surfaced | Macros.mac `class_call` L3 (macro definition text) + EdgeR.mac `?macro` unresolved | PASS |
| | E4 E5 E10 in unresolved w/ correct hint | E4 Caller.cls L14 `?dynamic_string` ($zobjmethod) · E5 Caller.cls L22 `?dynamic_string` (assembled + `$ZF`) · E10 Sub.cls L8 `?oref_call` | PASS |
| | ≥9/10 docs surfaced | 6/6 fixture docs + Sub.cls consulted; every carrier doc surfaced | PASS |
| | E3 NOT required (method query) | not in member query's set; found by the routine query | n/a |
| `^FProbeG.Mid` | E3 resolved | EdgeR.mac `routine_do` L5 | PASS |
| `FProbeG.Target` | E1 E2 E6 resolved (+super/Sub) | E1 · E2 · E6 `self_call` + Sub.cls `super` edge + Sub `self_call` (inherited `..`) | PASS |
| baseline (recorded, same docs) | 5/10 exact · 9/10 widened | G_RESEARCH: literal `FProbeG.Target.TgtMethod` search surfaced 5/10; widened `TgtMethod` 9/10 (misses E8's doc; decoy lands resolved) | recorded |
| find_references (this ship) | — | **10/10** edges surfaced, 7 resolved + 3 unresolved-with-hint | PASS |
| precision | decoy ONLY unresolved or excluded | decoy EdgeR.mac L11 → `?dynamic_string` unresolved; **0** resolved decoy edges | PASS |

Live matrix as shipped runs the same assertions in-suite
(`find_references_live_recall`, 24/24 passed incl. parallel `--test-threads=8`).

## Latency (project gate: member p95 < 2.0 s)

In-process MCP timings (PROOF-G-TIMING, 3 runs):

| query | run 1 | run 2 | run 3 | p95≈max | gate |
|---|---|---|---|---|---|
| member (×3 targets) | 1036/1131/1273 | 1028/1298/1413 | 1027/1053/1098 | **1413 ms** | PASS <2000 |
| class | 1021 | 964 | 1275 | 1275 ms | PASS |
| routine | 1218 | 1170 | 1176 | 1218 ms | PASS |
| census (`class_members` kinds) | 40 | 39 | 38 | 40 ms | PASS <1000 |
| `--include-system` (report only) | 3855 | 3909 | 3834 | ~3.9 s | no gate claim |

CLI wall time adds ~0.2–0.3 s process start: member 1055–1325, routine
920–973, class 963–1243, system 3896–3967 ms (same 3-run protocol).
The include-system number is slower than G_RESEARCH's 3.09 s measurement
(6-doc sweep now includes `.inc`-carried macro definitions + the same
overlapped narrowing SQL); it is reported honestly and is exactly why
`include_system` stays FALSE by default.

Concurrency note: stage 1 issues the literal search and both embedded-code
SQL narrowing queries (`CompiledProperty.SqlComputeCode`, `CompiledTrigger.Code`)
in parallel; the property scan measured ~0.9 s cold — serialising them pushed
member p95 to ~3.7 s and breached the 2 s gate before this fix.

## Teardown census

After every run (verified twice, incl. the manual proof harness):
`%Dictionary.ClassDefinition WHERE Name [ 'FProbeG'` → 0 rows;
`RoutineMgr_StudioOpenDialog('FProbeG*',…)` → 0 rows. Compiled member
routines (`*.0.obj`/`*.1.int` artifacts) die with the ClassDefinition
delete; the StudioOpenDialog listing lags deletions by ~1 s (cache), so
census checks re-poll rather than read once.

---

## Delta — cold re-proof after the scanner/stage refactor (2026-10-15)

Behavior-neutral rewrites landed since the table above (scanner passes 1–5
split into per-rule helpers, `find_references` stages extracted,
`class_members` census kind queries extracted). Full re-proof from cold:

| row | shipped | this re-run | delta |
|---|---|---|---|
| E1 E2 E6 resolved (member query) | PASS | PASS (6 resolved edges, same docs/lines/via) | 0 |
| E7 E8 resolved (trigger/computed) | PASS | PASS (`trigger` L25, `computed` L29) | 0 |
| E9 surfaced | PASS | PASS (Macros.mac `class_call` L3 + EdgeR `macro` hint) | 0 |
| E4 E5 E10 unresolved w/ hints | PASS | PASS (`dynamic_string` ×2, `oref_call`) | 0 |
| E3 resolved (`^FProbeG.Mid`) | PASS | PASS (`routine_do` L5) | 0 |
| class query: E1 E2 E6 + super | PASS | PASS (+Sub `super` edge, inherited `..`) | 0 |
| decoy: 0 resolved | PASS | PASS (unresolved `dynamic_string` only) | 0 |
| census kinds vs execute_sql | PASS | PASS — 8/8 kinds agree with the Oracle filtered the same Origin rule; `--include-inherited` parent-only 245=245 | 0 |
| member p95 (in-process, 3×3 cold suite) | 1413 ms | 1427 / 1521 / 1636 ms (one session-start cold-cache run at 2111 ms documented in G_ATTACKS G-01) | ≤ gate 2000 |
| CLI warm member / routine / class | 1055–1325 / 920–973 / 963–1243 | 1154–1202 / 1009–1095 / 1078–1101 | ≤ gate |
| teardown census 0 | PASS | PASS (both queries, re-poll) | 0 |
| live matrix 24/24 | PASS | PASS (`--test-threads=8`) | 0 |

Public-corpus precision sweep, crash realism probes, `.inc` storage-quirk
probe, guards-before-HTTP and the 30-tool surface re-count: see
G_ATTACKS.md (same server + corpus).
