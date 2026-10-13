# E_PLAN — file map + build order (mission E, issue #17 Tiers 1+2)

Contract: `E_SPEC.md` (frozen). This file: where each change lands, in
which order, and the risks. Lane: coder → tester → reviewer → docs.

## §A New files

| path | contents | spec ref |
|---|---|---|
| `src/iris/search.rs` | `search(client, ns, params) -> Envelope`-level call + `SearchEnvelope` parse fn `parse_search_result(env, requested_max)`; qs building via documents.rs:50-61 idiom | §2 |
| `src/tools/search.rs` | `SearchCodeArgs`, `SearchResult` structs + `search_code()` thin adapter | §1, §2 |
| `src/tools/graph.rs` | `FindSubclassesArgs`/`ClassMembersArgs`/`ClassDependenciesArgs` + result structs + 3 fns with BFS walks on `sql::query` | §1, §3 |
| `src/tools/names.rs` | `pub(crate) fn valid_class_name` + `valid_identifier` moved from testing.rs | §3 |
| `bench/run_bench.py` | orchestrator: fetch→transform→inject→load→probe→measure→report→cleanup | §6 |
| `bench/inject_markers.py` | deterministic marker/decoy injector + ground_truth.csv | §6 |
| `bench/graph_truth.csv` | hand-written inheritance ground truth | §6(e) |
| `bench/README.md` | run instructions + prerequisites | §6 |
| `bench/CORPUS.md` | seed table (4 repos, SHAs from §6) + per-run fetch log | §6 |
| `docs/code-search.md` | search+graph guide, bench numbers, §Tier-3 DESIGN-ONLY placeholder | TASK.md |

## §B Modified files

| path | change |
|---|---|
| `src/iris/mod.rs` | `pub mod search;` |
| `src/tools/mod.rs` | `mod names;` (pub(crate) items) + `pub mod graph; pub mod search;` |
| `src/tools/testing.rs` | delete fn bodies, add `use crate::tools::names::{valid_class_name, valid_identifier};` |
| `src/mcp/mod.rs` | 4 `#[tool]` stubs (annotations per §1), imports; list_tools paths unchanged (router auto) |
| `src/cli/mod.rs` | `Commands::Search{...}`, `Commands::Graph(GraphCommands)` |
| `src/main.rs` | dispatch arms (next to Sql) → tools fns; `--format` parity via render helpers |
| `src/settings.rs` | 2 fields + Default + env chain (apply_env) |
| `tests/api.rs` | U3–U5, U7, U8–U10, U11–U14 wiremock tests |
| `tests/tool_matrix.rs` | L1–L6 live cases + fixtures in cleanup matrix |
| `tests/http_door.rs` (+stateless if pinned) | 25→29 count |
| `README.md`, `docs/{ai-agents,cli-reference,getting-started,index,mcp-tools,configuration}.md` | 25→29 everywhere + new tool sections + settings keys + nav entry for code-search.md |
| `mkdocs.yml` | nav: Code search & references |
| `AGENTS.md` | counts (grep `25`) |
| `.gitignore` | `bench/.work/` |

## §C Build order (coder checkpoints — commit each row)

1. names.rs move + settings keys — gates stay green (pure refactor).
2. iris/search.rs + tools/search.rs + U1–U7.
3. mcp stub + CLI search arm + L-skeleton.
4. tools/graph.rs (+ iris reuses sql::query — no new iris module needed) + U8–U14.
5. graph mcp stubs ×3 + CLI graph arms.
6. docs pass (mcp-tools ×4, code-search.md w/ Tier-3 stub section, counts).
7. bench/ scripts (own commits; run once against dev container; RESULTS.md + numbers into docs).
8. full gate sweep: fmt, clippy -Dwarnings (+1.99 if installed), cargo test -q, `cargo test -- --ignored` against rism-iris, mkdocs --strict. Evidence tails → testing-notes/E_STATUS.md.

## §D Test matrix

See E_SPEC §7 (U1–U16, L1–L7) — one row per cargo test name; tester lane
adds nothing unnamed in the spec without a ledger note.

## §E Risks

1. **`[` contains over-match** — every walk exact-filters client-side (§3); regression = L3 diamond + U11's multi-super row.
2. **Server without action/search** (old IRIS) → 404 → clear HttpStatus error; docs state min server (issue text: verified on 2025.3; v2+ has search).
3. **bench compile failures** — §6 pre-flight probe decides compiled-only vs all-docs recall; results state the split honestly.
4. **Bench pollutes USER** — cleanup on every exit path + `docnames Bench%` post-check; worst case (killed run) rerun is idempotent (deletes leftovers first).
5. **max>200 client pass-through** — pinned by U5/L2; if server ever pages, the cliff test (d) catches it.
6. **Instructions-field budget** — 4 more tools lengthen list_tools descriptions; keep each description ≤~200 chars; CI probe measures.
7. **rmcp annotations drift** — mcp_probe presence gate expects exact titles; use `title` = short human name, frozen: "Search Code", "Find Subclasses", "Class Members", "Class Dependencies".
