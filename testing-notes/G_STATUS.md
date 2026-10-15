# G STATUS — certification verdict (reference graph + partials)

Mission: close issue #17's documented gaps (call-graph blind spot + partial
member kinds) on `feat/reference-graph`. Evidence trail: G_RESEARCH.md
(10 candidates probed, each with a live verdict) → G_SPEC.md (frozen
contract, chosen path matches the ranking rule server > parse > index) →
G_PROOFS.md (recall table + latency) → G_ATTACKS.md (adversarial re-proof).
This file carries the final independent certification sweep + verdict.

Server for all probes: IRIS for UNIX 2025.3 (Build 226U), container
`rism-iris`, namespace USER, corpus 5,165 docs / ~1,577 user classes.
Binary under certification: `cargo build --release` at HEAD (4 m 27 s,
under the 5-minute build gate).

## 1. Decision trail — PASS

Every G_RESEARCH candidate carries a verdict (objectscript-core REFUTED
in-tree on the C-dep rule; CompiledCode REFUTED; SyntaxColor REFUTED for
token-class purposes; server xref endpoints CONFIRMED absent). No
server-side cross-reference exists on 2025.3, so parse-on-fetch with
server-side SQL narrowing is the correct rung of the ranking ladder; the
baseline workaround (literal search) measured 5/10–9/10 recall, the ship
10/10. Fallback documented. Verdict consistent.

## 2. Independent recall probes (fresh fixtures, this sweep) — PASS

Fresh `FProbeGar.*` docs (unique package, nothing reused from earlier
sessions; loaded, compiled, queried, deleted). 9 planted cells:

| edge | required | measured | verdict |
|---|---|---|---|
| `##class` literal in .cls | resolved `class_call` | Caller.cls Edge1 L7 | FOUND |
| `##class` literal in .mac | resolved `class_call` | Mid.mac L3 | FOUND |
| `Do ^` in a method body (routine query) | resolved `routine_do` | Target.cls MDo L19 | FOUND |
| `Do ^` chained routine-to-routine | resolved `routine_do` | Mid.mac L5 → `^FProbeG_ar.Deep` | FOUND |
| `$zobjmethod` dynamic | unresolved `dynamic_string` | Caller.cls Edge4 L14 | FOUND |
| `o.M()` oref | unresolved `oref_call` | Sub.cls Edge5 L8 | FOUND |
| `..M()` self | resolved `self_call` | Target.cls MInst L13 | FOUND |
| trigger body | resolved `trigger` | Target.cls trg1 L26 | FOUND |
| `SqlComputeCode` body | resolved `computed` | Target.cls P9 L30 | FOUND |

Class-target query additionally surfaced the dictionary `super` edge
(Sub extends Target) + the inherited self-call. 9/9, incl. the required
one unresolved-dynamic and one `Do ^` cell. Teardown verified: 0 rows in
`%Dictionary.ClassDefinition`, 0 docs under `FProbe*`/`Bench*`/probe names.

## 3. Precision spot-check (public-corpus target, this sweep) — PASS

`find_references "%Stream.GlobalCharacter"` on the live system corpus:
79 resolved edges (54 `class_call` literal, 22 `self_call` inside the
class's own docs — the frozen class-target rule, 3 `super` dictionary
rows), 0 unresolved, truncated=false. Every resolved edge's line carries
the literal target or is a frozen-rule (self/super) hit: **0 false
positives**. Agrees with the G_ATTACKS aggregate (335/335).

## 4. Perf — independent 3×3 (in-process MCP, cold suite runs) — PASS

| query | run 1 | run 2 | run 3 | gate |
|---|---|---|---|---|
| member p95 | 1635 | 1501 | 1254 ms | <2000 PASS |
| member raw runs | 1188/1196/1635 | 1203/1277/1501 | 1157/1185/1254 | — |
| class | 1145 | 1475 | 1407 ms | PASS |
| routine | 1410 | 1344 | 1296 ms | PASS |
| census (`class_members`) | 52 | 58 | 53 ms | <1000 PASS |
| `--include-system` (report only) | 4502 | 4623 | 4367 ms | no claim |

Worst-case p95 1635 ms — inside the 2 s gate and inside the attack lane's
1427–1636 band. The G-01 cold-cache outlier (2111 ms, first call right
after fixture load) stays info-only: not reproduced in any of the three
cold suite runs here.

## 5. Census vs SQL oracle (independent kind) — PASS

`graph members %Library.JDBCCatalog --kind queries` = 25 vs
`SELECT COUNT(*) FROM %Dictionary.CompiledQuery WHERE parent=… AND
Origin=…` = 25. Exact match on the fresh class chosen for this sweep
(different class than the attack lane's fixture check).

## 6. Guards before HTTP — PASS

- `graph refs "Bad Target!"` against a DEAD port → `configuration error:
  invalid target …` with no connection attempt.
- `--namespace 'BAD NS/../../'` → `configuration error: invalid namespace
  (URL path guard)` before any request.
- Valid target + dead port → honest `network error`. Guard ordering
  confirmed: parse/validate strictly precedes HTTP.

## 7. Surface count — PASS with one doc-wording blemish (see §11)

- Live stdio `tools/list` on the release binary: exactly **30** tools,
  `find_references` present with the frozen name, all names distinct.
- HTTP door: probe script asserts 30 on the wire (passes; note the script
  reads `rism` from PATH — point it at the certified binary).
- `tests/http_door.rs` pins 30 (`inputSchema` count); probe script 30 on
  both doors; docs/mcp-tools.md, README.md, docs/index.md, AGENTS.md:10
  all state 30.
- Instructions field: 1403 chars (< 2048); every tool-name-shaped token
  in it resolves to a real tool (only arg names `max_rows`,
  `ignore_conflict`, `stop_on_entry` carry underscores outside the tool
  set). Confirmed against the live handshake output.

## 8. Gates full sweep — ALL PASS

| gate | result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -Dwarnings` (host) | 0 warnings |
| `rustup run 1.99.0 cargo clippy --all-targets -- -Dwarnings` | clean |
| `cargo test -q` (all suites, non-ignored) | 100+32+7+3+2+1+… ok, 0 failed |
| `cargo test --test tool_matrix -- --ignored` (live, --test-threads=8) | 24/24 ok |
| `mkdocs build --strict` (mkdocs-material) | clean, no warnings |
| `packaging/mcp_probe.sh` (stdio + HTTP doors, vs release binary) | OK both |
| `python3 bench/drift_port.py` | DRIFT FAILURES: 0 |
| `cargo build --release` time | 4 m 27 s (< 5 min) |
| container residue | 0 (FProbe*/Bench*/RismCI*/probe docs; see §10) |

## 9. Breaking-change audit vs `development` 75272e8 — PASS

Diff surface (`git diff --stat development...HEAD`): src/{cli/mcp/tools/
iris/settings/main} + tests + docs + bench + testing-notes/{G_*,fixtures-g}
+ .gitignore + packaging/mcp_probe.sh — expected only. **Cargo.lock and
Cargo.toml: zero diff** — the "no new deps" claim holds (std-only scanner,
`forbid(unsafe_code)` untouched).

Behavior deltas (both frozen in G_SPEC §4 with PRO/CON):
1. `find_references` + `rism graph refs` — net-new, no compat surface.
2. `DEFAULT_DOCUMENTS += *.inc` — `search_code`/`rism search` may return
   extra `.inc` matches, same ranking. Documented honestly in
   docs/code-search.md (default-masks bullet, with the 2025.3
   `.inc`→`.mac` doc-API storage quirk), docs/mcp-tools.md:203 and
   docs/cli-reference.md:35. The `.inc` PUT quirk matches today's live
   behavior (header-ful PUT stores as `.mac`; headerless PUT stores
   nothing — G-04).
3. `class_members` census kinds — additive JSON fields only, no renames.

## 10. Container hygiene — CLEAN

Two fixtures leaked from a crashed earlier run (timestamps 09:05 and
09:23, before this sweep) were found and deleted during certification:
`RismCI.Search12448520003.cls`, `RefProbe.mid.mac`. After removal: doc
list under `FProbe*`/`Bench*`/`RismCI*`/probe filters = 0,
`%Dictionary.ClassDefinition` residue = 0, no stray listener processes
(only the container's own port mappings), no test/rism processes left.
My fresh `FProbeGar` set was deleted with census re-poll verified 0/0.

## 11. UNRESOLVED (info only — none ship-blocking)

- **G-01** (info): cold-cache outlier 2111 ms on the very first call
  after a fixture load; not reproduced in 3×3 cold runs this sweep.
- **G-04** (docs/low): headerless `.inc` PUT stores nothing (server
  surfaces ERROR #16021 in the body) — harness scripts must check status.
- **AGENTS.md:56 says "29 `#[tool]` names"** where the truth is 30 (the
  line at AGENTS.md:10 and every other count surface says 30). One-word
  doc fix; the file is agent-protected and the in-lane edit needs
  interactive approval, so it is left for the PR author rather than
  forced. No behavior, test, probe, or docs-site surface depends on it.

## Verdict: PR-READY

All gates green on the release binary; recall, precision, perf, census,
guards, surface counts, and the breaking-change trail independently
re-proven this sweep. The single open item is the stale numeral at
AGENTS.md:56 (info).

## Public summary (ships with the PR — no internal vocabulary)

> **Find every call site, including the ones the old search could not
> see.** This release adds `find_references` (MCP) and `rism graph refs`
> (CLI): point them at a class, a `Class.Method`, or a `^Routine` and
> they return every body call site — literal `##class(…)` calls in
> classes and routines, self-calls (including inherited ones),
> `Do ^Routine` edges, inheritance rows, trigger and computed-property
> bodies, and macro-expanded calls. Dynamic dispatch that cannot be
> resolved statically (`$zobjmethod`, assembled names, object-reference
> calls) is reported in an honest `unresolved` bucket with a hint
> instead of being silently dropped. On a 10-edge planted fixture the
> tool surfaces 10/10 references where a literal string search surfaced
> 5/10; a 335-edge public-corpus sweep resolved zero false positives.
> Typical queries answer in 1.2–1.6 s against a 5,000-document
> namespace.
>
> `class_members` now lists queries, triggers, indices, and SQL
> procedures alongside the existing methods/properties/parameters
> (additive JSON; counts included), and namespace-wide census one-liners
> are documented. Macro `$def` definitions are no longer a search blind
> spot: `*.inc` joins the default document masks for `search_code` /
> `rism search`, so previously-invisible macro definitions now appear in
> results (same ranking, extra matches — note this if you parse search
> output). CSP pages remain opt-in via `--documents`. The tool surface
> grows from 25 to 30; every existing wire name is unchanged.
