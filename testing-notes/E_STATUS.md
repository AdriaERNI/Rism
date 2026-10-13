# E_STATUS — architect lane: PR-READY certification (mission E, issue #17)

Branch `swarmforge-architect`, merged chain: specifier 87b6b1f → coder
13ab1df → refactorer 5e4f65d07a (merged here at c2365018ba). Base 75272e8
(v0.2.2). All gates re-run independently in THIS worktree; bench run #3 is
this lane's own (`bench_arch_` prefix, release binary built here).

## 1. Contract diff vs E_SPEC.md (frozen surface)

| surface | E_SPEC | HEAD | verdict |
|---|---|---|---|
| tool names | search_code, find_subclasses, class_members, class_dependencies | identical (src/mcp/mod.rs:321-375) | EXACT |
| arg fields | §1 tables (SearchCodeArgs 9 fields; FindSubclassesArgs/ClassMembersArgs/ClassDependenciesArgs) | field-for-field match incl. `#[serde(deny_unknown_fields)]` ×4, `super` rename on GraphNode, enums `MembersKind{All,Methods,Properties,Parameters,XData}` / `DependencyDirection{DependsOn,UsedBy,Both}` snake_case wire | EXACT |
| response shapes | §1 JSON blocks | match (field names + nullability; `line` u32\|null + `label` companion; `counts` object; both dep arrays always present) | EXACT |
| tool count | 25 → 29 | `#[tool(` count = 29; wiremock/live pins updated (http_door.rs:180 = 29, http_door_stateless.rs:179 = 29, background_jobs.rs:49 = 29, tool_matrix surface = 29) | EXACT |
| annotations | title + read_only_hint + destructive_hint=false ×4 | present on all 4 stubs | EXACT |
| wire (search) | §2: params emitted per Some-ness, query+documents URL-encoded, ns via api_url (%→%25), `{}`/`[]`/absent → 0, str → InvalidEnvelope, tally = last "Found " console line, truncation marker constant | src/iris/search.rs matches rule-for-rule (verified by read: bool_flag omission logic, urlencoding on query + joined masks, TRUNCATION_MARKER, parse arms) | EXACT |
| graph SQL | §3 contains-walks + Origin filter + dep classifier | shapes byte-match; ONE additive column: deps `SELECT DependsOn, Super` (D3) | EXACT + D3 |
| CLI | §4 grammar: `rism search <PATTERN> [--documents --no-regex --system --generated --case-sensitive --whole-word --max --namespace]`; `rism graph subclasses/members/deps` with `--depth/--kind/--include-inherited/--direction`; both wildcard arms | src/cli/mod.rs:58-85, 199-231; to_search/to_graph + wildcard arms in both matchers (lines 425-426, 600-601); kind/direction accept snake_case + kebab | EXACT |
| settings | §5: search_max_results=200, graph_depth=10, RISM_SEARCH_MAX_RESULTS / RISM_GRAPH_DEPTH, garbage-keeps-default failure path | src/settings.rs:73-75, 105-106, 202-214 (parse_env_u32 same pattern as terminal_max_output_chars) | EXACT |

Deviations D1–D6 (E_BUILD §2, all probe-backed): **none touches the frozen
wire surface** — D1/D2 are test-assertion adaptations to the live regex-vs-
literal scan semantics; D3 adds one SELECT column with an unchanged response
shape and is mandated by §3's own "Super edge counts as a dependency" rule;
D4/D6 are bench-fixture mechanics; D5 documents that 2025.3 honours max>200
while the contract ("sent as-is") holds unchanged. All justified; ledger
status: accepted, no re-litigation needed.

## 2. Completeness diff

`git diff --stat 75272e8..HEAD`: 46 files, +5273/−57. Every changed file is
inside the expected surface: src (names.rs, search.rs, graph.rs, testing.rs
move, mod.rs wiring, mcp/mod.rs, cli/mod.rs, cli/render.rs, iris/search.rs,
iris/mod.rs export, main.rs, settings.rs) + tests (api.rs U1–U16,
tool_matrix.rs L1–L6 + 29-surface, count pins ×3 files) + docs (8 files +
mkdocs.yml nav) + bench/ (7 files incl. the refactorer's attack_refusal.py +
drift_port.py) + packaging/mcp_probe.sh + .gitignore + AGENTS/README +
testing-notes. **Cargo.toml/Cargo.lock untouched — zero new deps** (task
sign-off item: none needed; urlencoding was already a dependency).

## 3. Gate sweep (architect worktree, 2026-10-13)

| gate | command | result |
|---|---|---|
| fmt | `cargo fmt --check` | exit 0 |
| clippy host | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| clippy 1.99.0 | `rustup run 1.99.0 cargo clippy --all-targets -- -D warnings` | 0 warnings |
| unit + wiremock | `cargo test -q` | all binaries `ok, 0 failed` (83 lib + 23 api + 7 http_door + 3 stateless + 2 cli_transport + 1 background_jobs(+17 ign); the one ERROR log line is the connection-close race test's expected stderr) |
| live `--ignored` | `cargo test --test tool_matrix -- --ignored --test-threads=1` | `23 passed; 0 failed` in 10.7 s (IRIS 2025.3, container healthy) |
| probe | `PATH=target/release:$PATH bash packaging/mcp_probe.sh` | 4 legs OK: stdio 29 tools, background stream, extension-key, **`HTTP door answered (29 tools, SQL clean)`**, exit 0 |
| docs | `uvx --from mkdocs-material mkdocs build --strict` | built, 0 warnings/errors |
| drift (port) | `python3 bench/drift_port.py` | **DRIFT FAILURES: 0** |
| instructions budget | live `initialize` read-back | 1386 chars < 2048, all 29 names incl. the 4 new |

## 4. Bench — architect final run + three-run comparison

This lane's run: isolated scratch copy (`$SCRATCH/bench_arch/`), mechanical
rewrite `Bench* → BenchArch*` / `RISM-BENCH-* → RISM-BENCH-ARCH-*` (same
documented override method the refactorer used; the in-repo harness is
byte-identical to HEAD), `RISM_BIN=<architect worktree>/target/release/rism`
(release build of 5e4f65d+merge, this worktree), exit 0. Cleanup:
`CLEANED: 87 docs deleted; remaining BenchArch* docs: 0`; independent
post-check: `Bench%`=0, `BenchRef%`=0, `RismCI%`=0 (only the documented
`arsblue.datatype.DateTimeFormat` original-name remnant remains — not a
bench fixture, E_VERIFY §3.1).

| metric | run #1 coder (RESULTS.md @ c05c306dcb) | run #2 refactorer (BenchRef, E_VERIFY §3) | run #3 architect (BenchArch, this lane) | agreement |
|---|---|---|---|---|
| (b) recall | **20/20 = 100%** | 20/20 | **20/20 = 100%** | IDENTICAL |
| (c) precision | 20/21 = 95% (stale `RismCI.Probe5` FP) | **20/20 = 100%** (fixture deleted, B3) | **20/20 = 100%** | AGREES — run #1 delta = container hygiene, not product (the FP doc no longer exists; corpus-only precision was always 20/20 per E_BUILD §3) |
| (d) cliff max=200 | 200 matches / 199 docs, truncated=True | same | **200 / 199, truncated=True** | IDENTICAL |
| (d) --max 500 | 500, truncated=True | same | **500, truncated=True** | IDENTICAL |
| (d) rg occurrences | 481 | 481 | **481** | IDENTICAL |
| (e) graph | 11/11 | 11/11 | **11/11** | IDENTICAL |
| pre-flight probes | T/T/T | T/T/T | T/T/T | IDENTICAL |
| corpus load | 80 docs (38 compiled-ok) | 80 (24) | 80 (24) | docs count IDENTICAL; compiled-ok column is environment-state-sensitive (E_VERIFY §3.1 root-cause: stale original-name dictionary registrations from earlier probe loads; graded metrics unaffected) |
| (a) rism p50 band | 289–760 ms | 287–747 ms | 340–1026 ms | latency band drifts with server load (allowed); ratios same shape (rare queries ~1.3× broad-floor, rg 11–14 ms) |

Every graded metric (recall, precision, cliff, graph) agrees across all
three runs once the container is clean; no metric moved in a way that
changes recall/precision. Committed `bench/RESULTS.md` stays run #1
canonical; docs/code-search.md §Benchmarks matches it field-for-field
(recall 20/20, precision 20/21 with the decoy note, cliff, 11/11, latency
rows) and matches this lane's run on every metric modulo the documented
precision-hygiene and latency-band deltas above.

## 5. Security read of the new code paths (this lane, by reading)

- **Guard on every interpolation**: `guard(&args.class)?` is the FIRST
  statement of every Tier-2 entry point (src/tools/graph.rs:90, 290, 454) —
  before `ns_of`, before any client call. Every frontier name re-validated
  (`valid_class_name(f)` at :105, :582) before re-interpolation into
  `Super [ '{f}'` / `DependsOn [ '{f}' OR Super [ '{f}'`; server-sourced
  rows that fail the predicate are dropped with `tracing::warn!`, never
  interpolated. Dependency entries classified through
  valid_class_name/valid_identifier before surfacing (U13).
- **Encoding**: `action/search` interpolates nothing — query and
  `,`-joined masks go through `urlencoding::encode` (iris/search.rs:57-63);
  namespace through `api_url()` (%→%25, unit-tested). No other wire
  surface in the new code.
- **SQL strings**: all five frozen shapes byte-match §3 (U11/U12/U14 pin
  them); the only dynamic parts are guarded names. `parent = '{C}'` exact
  equality — no over-match vector.
- **no unwrap/expect**: every `unwrap()` in the new files is inside
  `#[cfg(test)]` blocks (verified by grep + line-position check);
  production paths use `unwrap_or_default`/`and_then` on Values. Crate
  clippy gates `unwrap_used = "deny"`, `expect_used = "deny"` (Cargo.toml)
  pass under BOTH toolchains (§3).
- **Error paths**: guard errors are `Error::Config` with the payload echoed
  in the message — it is an input-echo on the caller's OWN string before
  any interpolation, identical to the existing run_tests pattern; the dead-
  port probe (E_ATTACKS §A) proves refusal precedes any socket. depth=0 →
  Config error, >64 → clamp, 256-query frontier budget → truncated, never
  a fan-out DoS. Line normalization degrades (label field) instead of
  erroring; unparsable result shapes → InvalidEnvelope (fail-closed).
- **45/45 injection refusals** across CLI/MCP/dead-port/wiremock doors
  (E_ATTACKS §A), re-pinned green in this lane's `cargo test -q` (U8–U10
  `received_requests == 0`).

## 6. Docs triple-check

- 29 counts: README ×2 lines, AGENTS ×2, index ×3, ai-agents ×2,
  cli-reference ×2, getting-started, linux-packages, mcp-tools header — all
  29; probe script + 3 test pins + instructions all 29. `drift_port.py`
  (the Windows-CI `analyze-docs-drift.ps1` gate) **0 fails**.
- docs/mcp-tools.md: 4 new `###` sections (search_code / find_subclasses /
  class_members / class_dependencies) present — drift gate satisfied.
- mkdocs.yml nav includes `Code Search: code-search.md`; `mkdocs build
  --strict` clean.
- Tier 3 section reads "## Tier 3 — design only (deferred)" — no
  implementation claimed (parser/rusqlite sketch, explicitly not built).
- code-search.md §Benchmarks numbers == bench/RESULTS.md == the run table
  in §4 (precision decoy note explains the one documented delta).
- instructions field 1386 < 2048 (live read-back, §3).

## 7. UNRESOLVED

(empty)

## 8. Verdict

**PR-READY.** Contract surface EXACT vs E_SPEC (deviations D1–D6 documented
+ justified, none wire-visible); completeness surface clean, zero new deps;
all gates green on both toolchains; 23 live tests green; probe 29-tools
both doors; drift 0; third independent bench run agrees with runs #1–#2 on
every graded metric (recall 20/20, precision 20/20 corpus-honest, graph
11/11, cliff 200/199 + 500/rg-481); security guards proven on every
ingress path; docs consistent. Container left clean: 0 Bench*/BenchRef*/
BenchArch*/RismCI* docs; all lane-spawned processes exited (bench PID
149489 reaped; only the shared rism-iris container and pre-existing host
processes remain).
