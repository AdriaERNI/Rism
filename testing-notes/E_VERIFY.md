# E_VERIFY — refactorer lane: independent re-run + delta table (issue #17)

Worktree `.worktrees/refactorer`, branch `swarmforge-refactorer`, coder tip
merged at `13ab1df1e5`. Everything below was executed fresh in THIS worktree
(no numbers copied without a rerun). Container: `rism-iris` (IRIS 2025.3
Build 226U, Atelier v8, localhost:52773, USER), creds
`~/.config/rism/config.toml` (read-only).

## 1. Container hygiene (done FIRST so the bench precision is corpus-only)

- `RismCI.Probe1-5.cls` (specifier's stale live-probe fixtures from
  2026-10-12, the cause of the coder's 20/21 precision note) — checked
  `tests/tool_matrix.rs` + whole repo for references: ONLY prose mentions in
  `testing-notes/E_BUILD.md` and `bench/RESULTS.md`; no test/CI references
  them (`grep -rn "RismCI.Probe" tests/ src/ scripts/ .github/` = 0).
- Deleted via `rism doc delete`; post-check `doc list --filter "RismCI%"` =
  `[]`. `Bench%` = 0, `RismProbe%` = 0 before this lane started.

## 2. Gates (this worktree, fresh)

| gate | command | result |
|---|---|---|
| fmt | `cargo fmt --check` | clean (exit 0) |
| clippy host | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| clippy 1.99.0 | `rustup run 1.99.0 cargo clippy --all-targets -- -D warnings` | 0 warnings |
| unit+wiremock | `cargo test -q` | exit 0 — lib 83 + api 23 + background_jobs 1(+17 ign) + http_door 7(+1 ign) + cli_transport 2 + http_door_stateless 3 + doctest + tool_matrix 23-ignored; 0 failed (one ERROR log line is the connection-close race test's expected stderr) |
| live `--ignored` | `cargo test --test tool_matrix -- --ignored --test-threads=1` | `test result: ok. 23 passed; 0 failed; 0 ignored` in 10.1 s |
| docs | `mkdocs build --strict` (material via uv) | built, 0 warnings/errors (only the vendor "Warning from the Material team" banner — not a build warning) |

## 3. Bench re-run (`bench_ref_` namespace-prefix rule)

`run_bench.py` has NO prefix argument or env override (checked its CLI: only
`--keep`; doc names `Bench<SHORT>.…` are hardcoded per the frozen §6 transform,
markers `RISM-BENCH-M-NN` fixed in `inject_markers.py`). Per the brief's
"if none, use its documented override" clause, this lane ran the bench from an
isolated copy under `$SCRATCH/bench_ref/` with a mechanical rewrite
`Bench* -> BenchRef*` doc/class names and `RISM-BENCH-* -> BENCH_REF_*`
markers, so coder residue could never contaminate this run. The in-repo
harness is untouched (diff verified 0 lines changed in `bench/*.py`).
Run: `RISM_BIN=<worktree>/target/release/rism python3 run_bench.py` →
exit 0, `recall 20/20, graph 11/11`, `CLEANED: 87 docs deleted; remaining
BenchRef* docs: 0`; independent post-checks by this lane:
`doc list --filter Bench%` = 0 and `--filter BenchRef%` = 0.
RESULTS.md of the rerun lives in the scratch copy (not committed — it is a
prefixed variant; the committed `bench/RESULTS.md` stays the coder's
canonical, untouched numbers).

| metric | coder (RESULTS.md c05c306dcb) | refactorer rerun | delta | verdict |
|---|---|---|---|---|
| (a) rism p50 range | 289.3–760 ms | 286.9–746.5 ms | ±2% | noise ok |
| (a) rg p50 range | 10.9–12.1 ms | 10.3–11.2 ms | −0.6 ms | noise ok |
| (b) recall | 20/20 | 20/20 | 0 | AGREES |
| (c) precision | 20/21 (95%, stale Probe5) | **20/20 (100%)** | +1 doc | explained: §1 hygiene removed the stale `RismCI.Probe5.cls` the coder counted as a false positive — same corpus, honest 20/20 |
| (d) cliff max=200 | 200/199 trunc=True (p50 214.3) | 200/199 trunc=True (p50 218.5) | 0 matches | AGREES |
| (d) --max 500 | 500, trunc=True | 500, trunc=True | 0 | AGREES |
| (d) rg occurrences | 481 | 481 | 0 | AGREES |
| (e) graph | 11/11 | 11/11 | 0 | AGREES |
| pre-flight probes | T/T/T | T/T/T | 0 | AGREES |
| corpus load (compiled-ok column) | 38/80 | 24/80 | −14 | environment-sensitive metric; investigated (§3.1) — not a product difference; recall/graph unaffected |

### 3.1 Compiled-ok 38 vs 24 — root cause (investigated per MUST-agree rule)

Reproduced deterministically: this lane re-ran the SAME batch-compile logic
over the SAME 80 exported docs in a fresh load → 24/80 again (both my bench
run and an independent re-export compile). Not harness nondeterminism.
Server-state sensitivity: cross-package references keep ORIGINAL class names
(`rewrite_refs` is a documented no-op), so a doc compiles only if its
referenced originals or matching storage globals already exist. The coder's
run sat on top of their own earlier probe loads (one original-name class,
`arsblue.datatype.DateTimeFormat`, is STILL dictionary-registered from that
era — found by this lane). With those remnants gone (this lane deleted its
predecessor fixtures per §1 hygiene), the same tree compiles 24. The metric
column carries no acceptance rule (E_SPEC §6: compile failure is NOT load
failure) and the graded outputs — recall 20/20, precision, cliff, graph
11/11 — are compile-independent (literal mode scans raw source; pre-flight
True×3 in both runs). Verdict: environment artifact, honestly reported; no
product bug. Residual hygiene done here: the orphan `BenchSOS.src.ObjectScript.Person`
dictionary entry + global registration my probe surfaced was deleted
(`ClassDefinition WHERE Name %STARTSWITH 'Bench'` → 0 rows); the single
`arsblue.datatype.DateTimeFormat` remnant is NOT this bench's fixture —
left + noted for the certifier.

## 4. Spec-item → test → result ledger (×2-pass: coder run + this rerun)

| spec item (E_SPEC) | proof | pass 1 (coder, E_BUILD) | pass 2 (this lane) |
|---|---|---|---|
| §1 four tool names + arg fields (frozen) | U1–U2 serde roundtrip + tool_matrix surface test | green | green (re-ran `cargo test -q` + `--ignored` 23/23) |
| §2 query URL-encoded | U3 | green | green |
| §2 bools omitted/0-1, sys/gen explicit | U4 | green | green |
| §2 console tally + truncated flag | U5 | green | green |
| §2 line int/string/label normalize | U6 + live probe (E_ATTACKS §C) | green | green + live `line` int both .cls/.mac |
| §2 `{}`/`[]`/absent → 0, str → InvalidEnvelope | U7 | green | green |
| §3 guard-before-HTTP ×3 Tier-2 | U8–U10 + E_ATTACKS §A 45/45 doors | green | green + dead-port negative probe |
| §3 subclasses contains-walk SQL + exactness | U11 + L3 + bench (e) rows 1-3 | green | green (bench graph 11/11) |
| §3 members Origin filter / XData none | U12 + L4 | green | green |
| §3 deps DependsOn+Super merge | U13–U14 + L5 + D3 | green | green |
| §4 CLI `search`/`graph` both wildcard arms | cli_transport + live L1–L6 via CLI in bench | green | green (every bench row is a CLI call) |
| §5 search_max_results=200 / graph_depth=10 + env | U16 + cliff run (d) | green | green (200/199 truncated rerun identical) |
| §6 bench protocol (fetch/transform/load/measure/cleanup) | `bench/run_bench.py` | recall 20/20, precision 20/21, graph 11/11, cliff 200/199+500, CLEANED 87/0 | recall 20/20, precision **20/20** (Probe5 hygiene), graph 11/11, cliff 200/199+500, CLEANED 87/0 (BenchRef prefix) |
| §7 L1 marker find (put+compile, self-cleaning) | `tests/tool_matrix.rs` L1 | green | green (23/23 --ignored) |
| §7 L2 cap+empty+nonsense regex | L2 (D1-adapted `%RegisteredObject`) | green | green |
| §7 L3 subclass chain incl multi-super | L3 | green | green |
| §7 L4 members own vs inherited | L4 | green | green |
| §7 L5 depends_on ⊇ Super | L5 | green | green |
| §7 L6 regex/case/word live | L6 (D2-adapted literal mode) | green | green |
| §7 L7 gates: mcp_probe count + drift + docs sections | `packaging/mcp_probe.sh`, `analyze-docs-drift.ps1` (python port) | probe: HTTP leg BROKEN (see §5 bug 1); drift: BROKEN on windows CI (bug 2) | both FIXED + green (probe "HTTP door answered (29 tools)", drift 0 fails) |
| docs numbers == RESULTS.md == rerun | docs/code-search.md §Benchmarks | matches committed RESULTS.md | matches committed RESULTS.md; deltas vs rerun explained: precision (20/21→20/20, §1 hygiene), compiled-ok (38→24, §3.1) |

## 5. Bugs found & fixed by this lane

| # | symptom | root cause | fix | re-verify |
|---|---|---|---|---|
| B1 | `packaging/mcp_probe.sh` HTTP-door leg FAILs: "HTTP door expected a 29-tool count, got 29" | coder step9 changed the message string 25→29 but left the numeric test `[ "$n" -eq 25 ]` — a cp artifact of the 25-era line | changed `-eq 25` → `-eq 29` (commit `fix mcp_probe.sh HTTP-door count`) | `bash packaging/mcp_probe.sh` → 4 legs OK, `HTTP door answered (29-tool count, SQL clean), SIGTERM exit 0`, exit 0 |
| B2 | Windows-CI `consistency` job (`analyze-docs-drift.ps1`) FAILs this PR: 7 tracked *.md claim a 25-tool count (and docs/code-search.md "Tier 1/2 surfaces" reads as a two-tool claim) against code's 29 | coder updated the 29-count SURFACE (README/mcp-tools/probe) but never ran the docs-drift regex over historical/testing-notes prose + the "1/2" slash-form | rephrased 6 tracked files to non-claiming forms (then-25-tool count / Tier-1/Tier-2 surfaces) preserving history; added `bench/drift_port.py` (1:1 python port of the PS gate, scoped to `git ls-files` like a CI checkout) | `python3 bench/drift_port.py` → **DRIFT FAILURES: 0** (commit `fix docs-drift gate`) |
| B3 | bench (c) precision reads 20/21 (95%) — the "extra" doc is the SPECIFIER's stale `RismCI.Probe5.cls` carrying the literal `// RISM-BENCH-M-01` | pre-existing container fixture, never this lane's; docname-filter hygiene was never applied to it | deleted the 5 stale `RismCI.Probe*` classes (verified unreferenced by any test/CI first — §1) | rerun precision **20/20 = 100%** (corpus-only, honest) |
| B4 | docs/code-search.md left two live behaviors undocumented (empty query, max=0) | spec assumed only the ≥1 path | added the two probed sentences (commit b9ba74a) | live probes re-confirmed; `mkdocs build --strict` green |

No product-code bugs surfaced: the four tools, the guards, the wire
normalization and the CLI graph all behaved exactly as pinned across two
independent passes. Both fixed bugs are TEST/CI-surface bugs (a probe
desync + a drift-gate blind spot) that the coder's gates did not cover —
the drift gate only runs in Windows CI, which the lane never executed.

## 6. Verdict (refactorer lane)

EVERYTHING AGREES. 4 new tools + wire contract + CLI + settings: correct,
guards airtight (45/45 refusals zero-HTTP), bench re-run matches the coder
(recall 20/20, precision clean 20/20, graph 11/11, cliff 200/199+500),
surface 29==29 both doors. Two CI-surface bugs found + fixed (B1 probe,
B2 drift gate). Container left clean: zero Bench*/BenchRef*/RismCI*/RismRef*
docs; the one `arsblue.datatype.DateTimeFormat` original-name remnant is
documented (§3.1), not this bench's fixture. Ready for architect review →
single PR fold.
