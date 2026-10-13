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
dependency (dictionary semantics, verified live). Class names are
allowlisted (`valid_class_name`) before any request; unsafe names are
refused with zero HTTP calls.

## Tier 3 — design only (deferred)

A persistent local xref index (like Prism's `.index` or ctags) was ruled
out for this PR: there is no server xref machinery to read (issue #17)
and building an index duplicates the metadata scan above. Sketch:
hash-checked per-namespace incremental index in the Rism data dir,
populated by a full `action/search` sweep + `class_members` walk, with
the Tier-1/Tier-2 surfaces staying the source of truth on conflict. Not
implemented; no flag, no format.

## Benchmarks

4 public MIT repos (80 docs — see `bench/CORPUS.md` in the repo), 20
deterministic
markers, run 2026-10-13. Full tables in `bench/RESULTS.md`;
headline numbers (CLI wall ms incl. process start):

| metric | value |
|---|---|
| recall (20 markers, literal mode) | 20/20 = 100% |
| precision (aggregate, decoy-adjusted) | 20/21 = 95% |
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
