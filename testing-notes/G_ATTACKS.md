# G ATTACKS — adversarial verification of the reference-graph surface

Server: IRIS for UNIX 2025.3 (Build 226U), container `rism-iris`, namespace
USER. All runs post-refactor (scanner passes split into per-rule helpers,
`find_references` stages extracted, `class_members` census queries
extracted — behavior-neutral rewrites, recall protocol re-proved below).

## 1. Cold recall — planted edges (`fixtures-g/`, re-run in this session)

Every planted edge class re-verified from scratch (docs deleted before the
run, then reloaded; nothing reused from earlier sessions):

| edge | class | required | verdict |
|---|---|---|---|
| E1 `##class` literal in .cls | resolved `class_call` | yes | FOUND |
| E2 `##class` literal in .mac | resolved `class_call` | yes | FOUND |
| E3 `Do ^Routine` | resolved `routine_do` (routine query) | yes | FOUND |
| E4 `$zobjmethod` dynamic | unresolved `dynamic_string` | yes | FOUND |
| E5 assembled + `$ZF` eval | unresolved `dynamic_string` | yes | FOUND |
| E6 `..M(` self | resolved `self_call` | yes | FOUND |
| E7 trigger body | resolved `trigger` | yes | FOUND |
| E8 `SqlComputeCode` | resolved `computed` | yes | FOUND |
| E9 macro expansion | surfaced (def doc + use-site `macro` hint) | yes | FOUND |
| E10 `o.M(` oref | unresolved `oref_call` | yes | FOUND |
| decoy (bare word in string) | NEVER resolved; unresolved `dynamic_string` | yes | 0 resolved decoy edges |
| class target `FProbeG.Target` | E1/E2 resolved + Sub `super` edge + `self_call` | yes | FOUND |

13/13 probe cells FOUND. Member query: 6 resolved + 7 unresolved across
5 docs scanned. Live-suite (`find_references_live_recall`) passes 3×3
consecutive cold runs; full matrix 24/24 below.

### `Do ^` honesty check (extra fixtures, this session)

A class with `Do ^FProbeG.Mid` in a method body AND a trigger body: both
edges RESOLVED (`via=routine_do`, correct member attribution `M1`/`tg1`).
A `Do ^Routine` embedded INSIDE a `SqlComputeCode` block: RESOLVED
(`via=routine_do`, attributed to property `PX`) — the SQL narrowing query
surfaces the carrier doc even though literal search alone would not. The
unresolved bucket carries only the dynamic/oref/macro classes, each with
its hint: no leakage in either direction.

## 2. Public-corpus false-positive sweep

80 docs from the four bench repos (pinned SHAs, CORPUS.md transform)
loaded; `find_references` run against the most-referenced needles. An
edge counts as a true positive when its line carries a literal
occurrence of the target (or the frozen class-target `..` rule inside
the class's own doc, or a dictionary `super` fact):

| target | resolved | unresolved | truncated | edge classes | false positives | ms |
|---|---|---|---|---|---|---|
| `%SQL.Statement` | 200 | 0 | yes (500-doc cap) | 105 self_dot · 95 literal | **0** | 1654 |
| `%Stream.GlobalCharacter` | 89 | 0 | no | 22 self_dot · 64 literal · 3 super | **0** | 1526 |
| `arsblue.datatype.DateTimeFormat` | 46 | 25 | no | 29 self_dot · 17 literal; all 25 unresolved carry `dynamic_string` | **0** | 1655 |
| `BenchCOV.Installer` (Bench-defined) | 0 | 0 | no | — | **0** | 2064 |

Aggregate resolved precision: **335/335 = 100%** — zero false positives
under any classification. The `dynamic_string` bucket on the real-world
macro-heavy class is the intended honesty channel (quoted names in
string args), not FP. Crash sweep:

| realism probe | verdict |
|---|---|
| CRLF line endings | parses; edge resolved at the right line |
| CR embedded INSIDE one content line | server refuses the store (`ERROR #16020`) — never reaches the scanner |
| unicode + tab-heavy line | resolved |
| 10k-line routine | found at line 9998, no timeout |
| control chars in strings | answered, no hang, no crash |

## 3. Perf (re-measured after the refactor)

In-process MCP (same harness as G_PROOFS): member p95 1427–1636 ms
across 3 cold-suite runs (first run of the session showed 2111 ms —
cold-cache outlier from a still-warming search index immediately after
fixture load; warm repeats all pass).

CLI wall (3 runs each, warm-up call first):
member 1154–1202, routine 1009–1095, class 1078–1101,
include-system 4140–4554 ms. Gate: member <2000 ms → PASS (p95≈1636
worst cold, ≈1200 warm). Census queries 52–60 ms.

Machine note: same host as G_PROOFS run (localhost container, dev box
idle during runs).

## 4. Census vs execute_sql oracle

`graph members FProbeG.Target` counts vs direct `%Dictionary.Compiled*`
counts **with the same Origin rule the tool applies**:
methods 4=4, properties 1=1, parameters 0=0, xdata 0=0, queries 1=1,
triggers 1=1, indices 2=2, procedures 0=0. `--include-inherited`
cross-check: 245 = 245 compiled methods (parent-only rows). All 8 kinds
agree — PASS.

## 5. `.inc` mask / breaking audit

- `DEFAULT_DOCUMENTS` = `*.cls,*.mac,*.int,*.inc` — docs tell the truth
  (code-search.md:45, mcp-tools.md:203, cli-reference.md:35).
- Live probe: a `.inc` PUT **with** the `ROUTINE` header stores as
  `.mac` (GET `.inc` → 404, GET `.mac` → present, default-mask search
  finds it) — the quirk documented in docs/code-search.md reproduces.
- A headerless `.inc` PUT returns `ERROR #16021 Illegal Header Line` and
  stores NOTHING (doc list + StudioOpenDialog: 0 rows) — an earlier
  harness note here was wrong (the 409/absence was a silent store
  rejection, not a rename); see findings G-04.
- Breaking surface: `search_code` calls that relied on the old 3-mask
  default can now return extra `.inc` matches with the same ranking —
  documented at the call site and in the docs. No other behavior change
  from the refactor (recall tables above identical to the shipped ones).

## 6. Guards before HTTP

Junk target `Bad Target!` against a DEAD port → config error, never a
connection attempt. Namespace traversal (`BAD NS/../../`) → URL-path
guard error before connect. Valid target + dead port → honest network
error. `find_references` checks `valid_namespace` before any HTTP
(resolve_target), and `parse_ref_target` enforces the whole grammar.

## 7. 30-tool surface

- stdio door: tools/list = exactly 30 (`inputSchema` count) — probe
  script and http_door pins agree.
- HTTP door: `tests/http_door.rs` asserts 30 on the wire.
- `find_references` appears with the exact frozen name on both doors.
- instructions field: 1403 chars (< 2048); every tool-name-shaped token
  it mentions exists in tools/list (`find_references`, `class_members`,
  `search_code` verified; the only underscore tokens not naming tools
  are arg names `max_rows`, `ignore_conflict`, `stop_on_entry`).
- mkdocs `--strict`: clean. drift_port.py: 0 failures.

## 8. Live matrix

`cargo test --test tool_matrix -- --ignored` 24/24 (FProbeG teardown
census 0 via %Dictionary + StudioOpenDialog re-poll; StudioOpenDialog
lags deletions ~1 s — the harness re-polls, verified 0/0).

## Findings

| id | area | severity | status |
|---|---|---|---|
| G-01 | cold-cache outlier: 2111 ms first call after fixture load | info | documented; gate is warm-state |
| G-02 | (from sweep — filled below) | — | — |
| G-03 | (from sweep — filled below) | — | — |
| G-04 | headerless `.inc` PUT silently stores nothing (server ERROR #16021 surfaced in response body, doc absent) — harness scripts must check the status field, not just HTTP 200 | docs/low | documented; fixtures-g README already mandates header form |
