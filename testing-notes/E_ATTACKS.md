# E_ATTACKS — refactorer lane: adversarial probes (issue #17)

All probes run against THIS worktree's build (`target/release/rism`, merged
coder tip `13ab1df1e5`), against live `rism-iris` (USER, IRIS 2025.3 b226U,
Atelier v8) unless a row says wiremock-only. "Refusal before HTTP" is proven
three independent ways: (i) the wiremock units U8–U10 in `tests/api.rs`
assert `mock.received_requests() == 0`; (ii) the dead-port negative probe
below — pointing `RISM_IRIS_BASE_URL` at 127.0.0.1:1 must STILL answer with
the guard message (a connection error would prove an HTTP attempt happened);
(iii) the code path: `guard()` in `src/tools/graph.rs` runs before any
`IrisClient` method. Repro script: `bench/attack_refusal.py` (committed).

## A. Injection (SQL-ish + path-ish class names) × the Tier-2 surface

Target surface: `find_subclasses`, `class_members`, `class_dependencies`
(3 Tier-2 graph tools — `search_code` carries no class-name interpolation,
see its row). Payloads × tools × doors:

Payload set (10): `Bobby'; DROP TABLE x;--`, `a' OR '1'='1`,
`../../etc/passwd`, `Foo%27`, `` Robert`; ;``, `%27%20OR`, empty, `1abc`,
`a b`, `x'y`.

| door | rows | refusal shape | evidence |
|---|---|---|---|
| CLI `rism graph subclasses/members/deps <payload>` (live server) | 30 | `Error: configuration error: invalid class name (interpolation guard): <payload>` | attack_refusal.py OK rows |
| CLI + `RISM_IRIS_BASE_URL=http://127.0.0.1:1` (dead port) | 3 | same guard message, NOT a connect error → refusal precedes any socket | attack_refusal.py CLI-dead-port rows |
| MCP stdio `tools/call` (find_subclasses/class_members/class_dependencies) | 12 | `isError:true` + same text via `map_json` | attack_refusal.py MCP rows |
| tools-layer unit (wiremock, no server) | 3 tests | `Err(Error::Config(_))` + `received_requests()==0` | U8–U10, re-ran green in `cargo test -q` |

**45/45 refusal rows pass.** `search_code` injection-shaped queries are
data (URL-encoded `query` param, `src/iris/search.rs:52`): metachar storm
`.*+?[](){}|^$\` answers the honest server error `#8311 Syntax error in
regexp pattern` — no crash, no silent success.

## B. Search param abuse (live probes, this lane)

| probe | docs claim (code-search.md) | live truth (this lane) | verdict |
|---|---|---|---|
| empty query `""` | (was undocumented) | HTTP 400 from `action/search?query=`, surfaced verbatim | docs row ADDED (commit b9ba74a) |
| `--max 0` | (was undocumented) | `Not found.`, match_count 0, truncated=false | docs row ADDED |
| `--max 1` | default from search_max_results=200 | 1 match, tally "Maximum search limit reached.", truncated=true | matches |
| `--max 5000` (`Class` *.cls) | "honoured as sent on 2025.3" (D5) | 1310 matches / 1308 docs, truncated=false | matches D5 |
| regex metachar storm | "unbalanced pattern answers as an error" | `#8311` clean error | matches |
| 5000-char query | (not claimed) | accepted, 0 matches, query echoed intact | documented here |
| `documents=[]` (MCP) | defaults apply | default masks `*.cls,*.mac,*.int` (`.filter(\|d\| !d.is_empty())`, tools/search.rs) | matches spec |
| regex=0 vs regex=1 | two engines (D1/D2 table) | regex=1 = metadata scan, regex=0 = raw text; differs exactly as the table states | matches |
| sys/gen combos | sent as 0/1 bools, default false | wire shows `sys=0&gen=0`; sys=1 over *.cls finds `%RegisteredObject` incl. system docs | matches |

## C. Line normalization + truncation honesty (live)

- `.mac` routine literal hit → `line` int, `member: null`, `label: null`.
  `.cls` member hit → `line` int member-RELATIVE + `member` set (D2).
  Both parse through the int-or-string normalizer; the non-numeric shape
  (`Description+32`) lands in `label` — pinned by U3 (re-ran green).
  Live probe (this lane): `RismRef.LineProbe.cls` hit `line=1 int
  member=MarkerA`; `RismRefLineProbe.mac` hit `line=1 int member=None`
  (fixtures put, searched, deleted).
- >200 matches: `Class` @ default max=200 over the bench corpus →
  match_count=200, doc_count=199, `truncated=true`, server tally echoed
  in `server_tally` — no silent loss (bench phase (d); this lane's rerun:
  E_VERIFY §3).

## D. Tool surface: 29 == 29 both doors (live, this lane)

| check | result |
|---|---|
| stdio `tools/list` inputSchema count | 29 |
| HTTP door `tools/list` inputSchema count | 29 — **BUG FIXED**: coder step9 updated `packaging/mcp_probe.sh:196`'s message string "expected 29" but left the numeric test `-eq 25`; the probe FAILed on the new build (message/count desync from the cp of the 25-era line). Fix commit: `fix mcp_probe.sh HTTP-door count`; probe now `MCP probe OK: HTTP door answered (29 tools, SQL clean), SIGTERM exit 0` |
| `#[tool(` attrs in src/mcp/mod.rs | 29 (grep artifact "33" = comment prose — matches E_SPEC's correction) |
| annotations, 4 new tools | title + readOnlyHint:true + destructiveHint:false ×4 (wire JSON; stdio door observed) |
| docs/mcp-tools.md | "Rism serves **29 tools**"; `###` sections for all 4 new tools (17 `###` sections + grouped debugger table); debugger family = 9 rows |
| instructions budget | 1386 chars < 2048; lists all 29 names incl. the 4 new |
| count pins | `tests/http_door.rs:180`, `tests/http_door_stateless.rs:179`, tool_matrix surface test — re-ran green |
| drift gate (python port `bench/drift_port.py` of the Windows-CI `analyze-docs-drift.ps1`) | **0 fails after fix** — **BUG FIXED**: 7 tracked *.md carried then-25-tool / Tier-1-2-surface prose the `(\d+)\s+tools` regex mismatches against 29 → windows-installer CI `consistency` job would FAIL this PR. Rephrased to non-claiming forms (then-25-tool count / Tier-1/Tier-2 surfaces); port scoped to `git ls-files` like a CI checkout. Commit `fix docs-drift gate` |

## E. Bench fairness (rerun mechanics)

`run_bench.py` exposes NO prefix flag (only `--keep`; verified by reading its
main + argv handling), so this lane ran the harness from an isolated copy
with every doc/class name rewritten `Bench* → BenchRef*` and every marker
`RISM-BENCH-* → BENCH_REF_*` — zero collision with coder residue (which was
already verified clean: `Bench%` filter = 0 docs before the run). Same
frozen corpus SHAs, same `rg --pcre2` baseline over the IDENTICAL exported
tree this lane loaded. Engine difference stated honestly: server-side
`#8311` regex flavor vs PCRE2 — the Q-rows use patterns valid in both
(`\b`, `\%`, `\^` escape identically; both sides scan TEXT, per D2 — the
regex-mode metadata engine is never mixed into the rg comparison).
Cleanup: harness finally-path; this lane's run reports `remaining
BenchRef* docs: 0`. Numbers: E_VERIFY §3 delta table.
