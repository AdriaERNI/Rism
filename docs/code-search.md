# Code Search & Reference Graph

Issue #17 ships in tiers. Rism implements Tier 1 and Tier 2 with zero new
endpoints and zero server-side helper code: one Atelier call
(`action/search`) plus `%Dictionary` SQL.

## Tier 1 — `search_code` / `rism search`

Server-side regex/text search over source documents. Query, masks, and
flags ride the wire as-is (URL-encoded); results come back grouped per
document with member/attribute/line attribution, normalized so `line` is
always a number or a label token, never a parse failure.

### Scan semantics (probed on IRIS 2025.3, Atelier v8)

The endpoint has two genuinely different engines and the `regex` flag
selects between them:

| mode | what it scans | finds |
|---|---|---|
| `regex` (default) | class **metadata** — member names, property/parameter/XData names, attribute values (e.g. a property's `Type`) | identifiers, not body text |
| `regex: false` (literal) | raw **source text** of each document (compiled or not — uncompiled docs included) | any literal substring: comments, string literals, code |

Practical rules:

- Search for an **identifier** (method/property/class name): leave regex
  on — results carry exact member attribution.
- Search for **text inside code** (a string literal, a comment): set
  `regex: false` (`--no-regex`). A regex-mode search for body text
  returns `Not found.` — this is the engine, not a bug (it is why the
  bench's recall numbers only make sense against literal mode; see
  [benchmarks](#benchmarks)).
- Literal-mode hits on class members report `line` relative to the
  member's routine section, not the file — treat `member` + `text` as
  the position, not `line`, for `.cls` docs.
- `case_sensitive` / `whole_word` only take effect in literal mode.
- An empty `query` is rejected server-side (HTTP 400 on `action/search`)
  — `search_code` surfaces the error rather than returning everything;
  `max=0` answers `Not found.` with zero matches and `truncated=false`.
- `truncated` is true when the server printed its "Maximum search limit
  reached" notice **or** the client-side match count reached the requested
  `max` — so a query whose true total exactly equals `max` also flags
  `truncated` (conservative; re-run with a higher `--max` to confirm you
  saw everything).
- Default masks are `*.cls`, `*.mac`, `*.int`, `*.inc`. Macro `$def`
  definitions live in `*.inc` and used to be a silent blind spot under the
  old defaults; CSP pages (`*.csp`) stay opt-in — pass
  `--documents '*.cls,*.csp'` to include them. Note: on 2025.3 the doc API
  stores a `.inc` PUT under the `.mac` name (server quirk, verified live);
  scanning both masks covers either spelling. A `Not found.` for a macro
  name with a custom mask that omits both is a mask miss, not an absence.
- Literal mode scans the body sections of each document (methods,
  properties, XData, comments, string literals). UDL **header** lines —
  `Class X Extends Y`, `Include Foo`, `ProcedureBlock`, class keywords —
  are not part of that text; they are searchable in regex (metadata)
  mode via attributes like `Super` and `IncludeCode`.
- Metacharacters: in regex mode your query is a server regex (an
  unbalanced `(` answers as an error); in literal mode `-` and `$` are
  plain text. The bench marker `RISM-BENCH-M-01` therefore needs
  `regex: false`, or escaping (`RISM\-BENCH\-M\-01`) — and escaped
  regex-mode still misses, because the text is a comment.
- The `max` parameter (default 200 via `search_max_results`) is
  honoured as sent on 2025.3; issue #17's "hard cap 200" holds for the
  server's own default when `max` is omitted or ≤ 200. `truncated`
  reflects the server's "Maximum search limit reached" notice.

## Tier 2 — the graph tools

`find_subclasses`, `class_members`, `class_dependencies` read
`%Dictionary.ClassDefinition` + `Compiled*` tables through the existing
`action/query` endpoint. The dictionary's `[` (contains) operator is
substring-based, so every walk re-validates each edge client-side
(`Super` lists split exactly) — the over-match guard is pinned by tests
(`subclasses('BenchGraph.Mi')` returns zero rows, not `BenchGraph.Mid`).
`depends_on` merges the raw `DependsOn` field with the exact `Super`
list: on 2025.3, inheritance-only classes keep `DependsOn` empty even
when their bodies reference the superclass — a superclass IS a
dependency (dictionary semantics, verified live). Names that have no
`%Dictionary.ClassDefinition` row (a typo, or a class ALIAS such as
`%RegisteredObject` — the dictionary keys on the real name
`%Library.RegisteredObject`) answer an honest not-found instead of
misleading zero members; query the canonical name. Class names are
allowlisted (`valid_class_name`) before any request; unsafe names are
refused with zero HTTP calls. Namespaces ride the URL path, so they get
their own allowlist on the same rule (letters/digits/`_`/`-`, optional
leading `%`): an unsafe namespace is refused before any request too.
`used_by` is a graph edge over the dictionary `DependsOn`/`Super` fields
only: 2025.3 does NOT populate `DependsOn` from body references
(`##class(X).M()` calls), so a class used exclusively inside method
bodies is invisible to BOTH directions. For body call sites, the answer
is `find_references` (below) — `used_by` is a fast structural hint, not
impact analysis.

### `find_references` — the body-call-site answer

2025.3 has no server-side cross-reference store (`^XDEF`, `%SyntaxColor`
call graphs — probed, absent), so `find_references` builds edges per
query with parse-on-fetch: a literal `action/search` narrowing pass plus
SQL over the dictionary's embedded-code columns
(`CompiledProperty.SqlComputeCode`, `CompiledTrigger.Code` — the literal
engine surfaces those inconsistently), each candidate doc fetched ONCE,
then a source scanner classifies every call-site shape:

| pattern | example | result |
|---|---|---|
| literal class-method call | `##class(Pkg.Cls).M(...)` | resolved `class_call` |
| self / `..` inside the target's own class (incl. inherited) | `set x = ..M(6)` | resolved `self_call` |
| routine `Do` | `Do ^MyRoutine` (with/without argument) | resolved `routine_do` (against `^Routine` targets) |
| inheritance | `Class D Extends Pkg.Cls` | resolved `super` (dictionary row) |
| trigger / computed-property bodies | `write ##class(Pkg.Cls).M()` in a `%Dictionary` code block | resolved `trigger` / `computed` |
| macro `$def` call sites | `$$$M` (definition scanned too) | resolved when the definition expands to a literal call; `?macro` hint otherwise |
| object-reference dispatch | `o.M()` where `o` may be the target | unresolved, `?oref_call` |
| dynamic dispatch | `$zobjmethod(o,"M")`, assembled `##class("_…` eval'd via `$ZF` | unresolved, `?dynamic_string` |
| comment / string decoy | `write "M is the name we hunt"` | excluded by the string/comment lexer, or `?dynamic_string` when it carries a quoted member name — never a resolved edge |

Measured on the 10-edge planted fixture (`testing-notes/fixtures-g/`):
**10/10 surfaced** — 7 resolved + 3 in `unresolved` with the right hint,
0 resolved false positives — against 5/10 for the exact-literal
`##class(...)` search and 9/10 for the name-word search (G_PROOFS.md has
the repro lines and the latency table).

What stays structurally out of reach, stated honestly:

- **`Do ^` into a computed routine name** (`Do ^rng` / `Do @x`): the
  routine name never appears as literal text anywhere. This is the one
  edge class no fetch-scan can catch; a plain `Do ^Name` IS caught.
- **pure dynamic dispatch where even the member name never appears as
  literal text** (fully assembled `"F"_"oo"` identifiers): reported as
  `unresolved` when a `$zobjmethod`/`$ZF` form is visible, otherwise
  invisible to any text-based tool, ours included.
- `$zf`-evaluated runtime-only dispatch; global/stream **data** and
  production **config rows** (use `execute_sql` over the right
  `%Dictionary`/global surfaces — source search cannot see them).
- cross-namespace references: one namespace per call.
- a persistent incremental index: deliberately not shipped — bounded
  per-query scans measured faster than any checkout-free index build
  path, and a stale index is worse than a fresh scan (revisit only if
  per-query latency fails on real corpora).

## Benchmarks

4 public MIT repos (80 docs — see `bench/CORPUS.md` in the repo), 20
deterministic
markers, run 2026-10-13. Full tables in `bench/RESULTS.md`;
headline numbers (CLI wall ms incl. process start):

| metric | value |
|---|---|
| recall (20 markers, literal mode) | 20/20 = 100% |
| precision (aggregate, decoy-adjusted) | 20/20 = 100% (re-run at PR head; the earlier 20/21 counted a stale container fixture, since deleted — see F_FINDINGS F-02) |
| graph checks vs hand truth | 11/11 = 100% |
| rare-query latency p50/p95 | 289 / 303 ms |
| broad-query latency p50/p95 | 715–760 ms (server-side scan dominates) |
| rg baseline p50/p95 (local tree) | 11–12 / 12–14 ms |
| ratio | rg ≈ 22–60× faster locally; server search needs no checkout |
| truncation cliff (`Class`, *.cls) | default caps at 200 matches; `--max 500` returns 500 — no silent paging |

Reproduce: `cargo build --release && python3 bench/run_bench.py`
(4-minute run; cleans every loaded doc on all exit paths).

## Where to go next

- [MCP Tools](mcp-tools.md) — tool parameters
- [CLI Reference](cli-reference.md) — `rism search` / `rism graph`
- [Configuration](configuration.md) — `search_max_results`, `graph_depth`
