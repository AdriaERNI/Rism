# E_ATTACKS — refactorer lane: adversarial probes (issue #17)

All probes run against THIS worktree's build (`target/release/rism`),
against live `rism-iris` (USER) unless a row says wiremock-only. "Refusal
before HTTP" is proven by wiremock request-count assertions (pattern
tests/api.rs:263) and, for the CLI/MCP doors, by the same unit path the
doors call into (`src/tools/graph.rs` guards run before any `IrisClient`
call).

## A. Injection (SQL-ish + path-ish class names) × 4 Tier-2 tools

Target surface: `find_subclasses`, `class_members`, `class_dependencies`
(Tier-2) + `search_code` (query travels URL-encoded). 3 doors each:
CLI arg, MCP tools/call (stdio), tools-layer unit (wiremock).

| payload | tool | door | refused? | zero HTTP? | message sane? |
|---|---|---|---|---|---|
| PENDING | | | | | |

## B. Search param abuse

| probe | expected (docs/code-search.md) | live truth (this lane) | docs match? |
|---|---|---|---|
| PENDING | | | |

## C. Line normalization / truncation honesty

PENDING.

## D. Tool surface 29 == 29 both doors + annotations + probe script

PENDING.

## E. Bench fairness (rg same tree, same syntax family)

PENDING.
