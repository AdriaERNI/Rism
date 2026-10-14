# F_STATUS — verdict, PR #22 adversarial review (task rism-pr22-review-f)

Architect lane. Merged chain: specifier 32ed2f5 (F_PLAN 96 rows) → coder 2d3dfec
(F_MATRIX M/H/P/B rows + F_FINDINGS F-01..F-10) → refactorer 8ffcffe (26 rows +
fix round: code 2c3d795, docs 6e7e715, test pin 1e73b48). This file: second-pass
verification of every findings row, matrix coverage audit, final gate sweep, the
explicit answer to "can the index find everything needed?", and the verdict.

Binary under second-pass: this worktree `target/release/rism` rebuilt at merged
HEAD 8ffcffe (rism 0.2.2, includes the fix round). Server: docker rism-iris
(2025.3 b226U), USER + %SYS. Architect fixtures: `FProbeAr.*`/`FProbe.Ar*`, all
deleted second-pass — verified `doc list --filter FProbe%` == 0 docs AND
`ClassDefinition WHERE Name LIKE 'FProbe%'` == 0 rows; `Bench%` == 0; no 31xxx
listeners; no stray rism processes.

## §1 Findings closure — second-pass evidence (this lane, live)

| id | sev | status claimed | architect second-pass | evidence (executed here) |
|---|---|---|---|---|
| F-01 | HIGH | doc-fixed(6e7e715) | CLOSED | docs/mcp-tools.md:257 now says "IMPACT HINT, not complete" + `##class(X)` literal workaround; re-ran the shape live: `graph deps FProbeAr.A --direction used_by` → [FProbeAr.Child] (Super edge only) while the body-caller FProbeAr.Caller is found ONLY by `search '##class(FProbeAr.A)' --no-regex` → exactly the documented split. Docs ↔ behavior parity: word-for-word match. |
| F-02 | MED | doc-fixed(e25353f coder) | CLOSED (accepted residual) | grep: docs/code-search.md headline = "20/20 = 100% (re-run at PR head; the earlier 20/21 counted a stale container fixture…)". Residual: bench/RESULTS.md:73 still prints 20/21 — it is a GENERATED artifact of the stale-fixture run; next clean re-run overwrites it (run_bench.py regenerates the file); refactorer's "generated files not hand-edited" rationale verified against the generator. |
| F-03 | MED | fixed(2c3d795) | CLOSED | live: `graph members %RegisteredObject` → `class '%RegisteredObject' not found … canonical name, e.g. %Library.RegisteredObject` rc=1 (was: misleading all-zeros rc=0); canonical still 19/3/1/0; typo name on subclasses/deps → same honest error; NEGATIVE case right too: existing-but-empty class → zeros rc=0 (no false not-found). Refactorer's "alias SQL rewrite impossible" claim INDEPENDENTLY VERIFIED here: `Alias` column → `SQLCODE -29 Field 'ALIAS' not found` on BOTH ClassDefinition and CompiledClass; `ClassDefinition WHERE Name='%RegisteredObject'` → 0 rows. Alias-name behavior further proven on a real alias fixture (FProbeAr.Base [Alias=FProbeArAlias]): alias name refused honest-not-found, canonical correct. MCP door: same error text via tools/call. |
| F-04 | LOW | fixed(2c3d795) | CLOSED | live: `search Class --documents ''` → rc=0, `documents` echo = `['*.cls','*.mac','*.int']` (default masks, no HTTP 400); MCP `documents:[""]` → same fallback, results byte-shape equal to default search. |
| F-05 | LOW | fixed(2c3d795) | CLOSED | live: `--help` = "Max matches (default: settings `search_max_results` = 200)"; tools/list `max_results.description` = "…= 200; newer servers honour higher values…" (no "hard-caps at 200" anywhere on the wire); behavior parity: `--max 500` → match_count 500 + server limit notice. tools/list still 29 entries. |
| F-06 | MED | doc-fixed(6e7e715) | CLOSED | code-search.md bullet present (masks + `--documents '*.inc'`/`'*.csp'`). Parity re-proved on the server's REAL corpus: macro `EnsAlertErrNoDefaultNotificationOperation` (defined only in EnsAlertErrors.inc) → default masks `Not found.` (mask miss, exactly as docs now say), `--documents '*.inc'` → 1 hit in EnsAlertErrors.inc; `*.csp` mask → 2 hits (M-16 shape). |
| F-07 | MED | doc-fixed(6e7e715) | CLOSED | mcp-tools.md now names Queries/Triggers/Indices/Procedures as NOT listed + workarounds. Parity re-proved: `graph members` on a class with `Query Q1` → output keys are exactly methods/parameters/properties/xdata (no queries key), oracle CompiledQuery row exists, and the documented workaround `search 'Query Q1' --no-regex` finds it (member=Q1). |
| F-08 | LOW | doc-fixed(6e7e715) | CLOSED | code-search.md header-line caveat present ("UDL header lines … not part of that text; searchable in regex (metadata) mode via … IncludeCode"); coder's live rows (literal `Include X` = Not found, regex metadata attr=IncludeCode hits) match the wording — verified in F_MATRIX M-11/M-12 oracles; own fixture re-run consistent. |
| F-09 | MED | fixed(2c3d795)+docs | CLOSED — namespace vector fully shut | live refusal BEFORE any HTTP on ALL four surfaces (search CLI, `graph subclasses`, `graph deps`, MCP tools/call): `A' OR 1=1--`, `../../`, `USER&x=1`, `%`, `%41`, `''`, `x y` → `configuration error: invalid namespace (URL path guard)`, rc=1; DEAD-PORT parity (`RISM_IRIS_BASE_URL=http://127.0.0.1:1`) prints the IDENTICAL guard error = proof of zero HTTP; regressions: %SYS members 19/3/1/0 ok, USER echo ok; docs (code-search.md + mcp-tools.md) now state the namespace allowlist rule explicitly. Unit pin namespace_allowlist_shape runs green in cargo test. |
| F-10 | LOW | doc-fixed(6e7e715) | CLOSED | RESULTS.md + run_bench.py generator both say "2 HTTP round trips (auth + action; proxy-verified)" + machine-specific ratio note — consistent with B-07 proxy evidence; wording parity verified by reading the shipped files. |
| D-01 | LOW | wontfix | ACCEPTED | mutation-tested (F_MATRIX §4b): delta only on DIRTY trees; CI checkouts are clean by construction; py is the local mirror. Reason holds. |
| D-02 | LOW | wontfix | ACCEPTED | mutation-tested: both gates fail (loud), py fails cleaner; no CI correctness gap. Reason holds. |

**No CRITICAL/HIGH open. All ten F rows closed with second-pass evidence; two
D rows wontfix with mutation-tested reasons.**

## §2 Matrix coverage audit

F_PLAN totals 96 rows (33 M + 18 H + 12 P + 11 B + 7 D + 15 DC). Row-count
against the executed tables: M 33/33, H 18/18, P 12/12, B 11/11, D 6/6 verdicted
(+D-07 is the process rule, proven by the gate self-trip), DC 15/15 — **96/96
rows carry a live result; zero BLOCKED, zero 'assumed/per docs' verdicts**
(grep-verified: the only 'BLOCKED' token is the legend comment). M-18's
IMPOSSIBLE is a derived conclusion explicitly built on executed M-05/M-09/M-17
rows — honest 'no', documented as such.

Architect spot-re-runs (5, unique `FProbeAr_`/`FProbe.Ar*` fixtures, cleaned
after): M-02 (inherited members — 18 methods vs own 1, origin semantics incl.
the `origin:""` rows F-07b documents == oracle without Origin filter ✓), M-09
(generated `.1.int` routine invisible by default, visible with `--generated` ✓),
M-16 (`*.csp` mask → 2 hits 1 file ✓), M-29 (max 0→Not found/trunc false, 1→1
trunc true, 500→500 honored ✓), H-07 (`.*+?[](){}` → #8311, `(` → #8316 honest
server regex errors ✓). Plus the §1 fix-path probes (F-01/F-03/F-04/F-05/F-06/
F-07/F-09 re-executed). Every spot result matched the recorded row.

## §3 Gate sweep on the merged review branch (all executed this lane @ 8ffcffe)

| gate | result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -Dwarnings` (host 1.97.0) | ok |
| `rustup run 1.99.0 cargo clippy --all-targets -- -Dwarnings` | ok |
| `cargo test -q` | 123 unit+wiremock tests pass, 0 fail (incl. U19–U21 fix pins + namespace_allowlist_shape; live suite correctly ignored) |
| live `cargo test --test tool_matrix -- --ignored --test-threads=1` | **23/23 ok** (serialized, shared container healthy) |
| `uvx --from mkdocs-material mkdocs build --strict` | built, 0 warnings/errors |
| `packaging/mcp_probe.sh` (branch release binary on PATH) | **29 tools both doors** — stdio "MCP probe OK: 29 tools…", HTTP door "29 tools, SQL clean, SIGTERM exit 0"; NOTE: running it with the stale `~/.local/bin/rism` 0.2.1 on PATH counts 28 — the packaged-binary contract is satisfied by the branch binary; not a branch defect |
| `python3 bench/drift_port.py` | **0 fails**, exit 0 |

## §4 Bench triple-state coherence

Published (E-era, 3 runs): recall 20/20, graph 11/11, cliff 200/199+500. Coder
re-run at PR head (B-01): recall 20/20 MATCH, graph 11/11 MATCH, cliff MATCH,
precision 20/20 vs published 20/21 — root-caused to the deleted stale
`RismCI.Probe5.cls` fixture (F-02), headline docs corrected with provenance.
Latency deltas re-measured higher (contention) and honestly framed as
machine-specific after F-10. Methodology audit rows (B-02/03/05) found NO
self-proving loop, NO vacuous-precision hit, strict==loose scoring. Coherent.

## §5 Verdict — "can the index find everything needed?"

Across the 33-row discovery matrix (each row executed live against IRIS 2025.3):
**FOUND as documented: 11 · PARTIAL (needs a specific flag/mask/workaround): 15 ·
BLIND/IMPOSSIBLE: 7** — and every blind item has either a documented workaround
or an honest "server-side impossible" answer.

- Class structure: fully found (members incl. inherited, subclass trees with
  honest `truncated`, diamond/cycle-safe BFS). Dictionary aliases/typos now
  answer honest not-found instead of misleading zeros (fixed in this review).
- Call/reference graph: **THE headline blind spot — body call sites.** On 2025.3
  `DependsOn` is not populated from `##class(X).M()` calls, so `used_by`/
  `depends_on` are impact HINTS, not completeness. Workaround that closes it in
  practice: `search '##class(X)' --no-regex` (literal scan) — now documented in
  both docs pages. Full cross-reference is Tier-3, honestly deferred (no server
  xref machinery exists).
- Where's-the-file blind spots are mask-level, not engine-level: `$def` macro
  definitions (.inc) and CSP pages (.csp) ARE scanned when you pass
  `--documents '*.inc'`/`'*.csp'` — default masks cls/mac/int answer `Not found.`
  for them; now documented with exact commands.
- Member-table gaps: Queries/Triggers/Indices/Procedures are NOT in
  `class_members` (4 tables only) — findable via literal search or
  `execute_sql`; exclusion list + workarounds now documented.
- Truly impossible via this surface: global/stream DATA content (M-17/M-18),
  production config ROWS (M-33; XData source blocks ARE searchable),
  namespace-wide member census (M-08 → `execute_sql` on Compiled*). All honest
  no's with the SQL escape hatch named.
- System content: %SYS works with `--system`; generated code needs `--generated`;
  both documented and live-verified.
- Cap/truncation: `truncated` never lies (server notice OR client rule, explained);
  the old "server hard-caps at 200" text was wrong for 2025.3 and is corrected.

**Answer: yes for the source-code + structure questions an agent asks day-to-day
(every need in the matrix resolves found or documented-partial); the one honest
structural blind spot is call-graph references — Tier-3 by design — with a
working literal-search workaround that the docs now state where developers look.**

## §6 Verdict line

**PR-APPROVE.** UNRESOLVED: empty (F-01..F-10 closed; D-01/D-02 wontfix-LOW with
mutation-tested reasons; no open items above LOW).

### Review comment for PR #22

```markdown
## Review — PR #22 (code search + reference graph)

**Scope & method.** All 35 commits (ffd3580 vs development) reviewed: every src
hunk read line-by-line; 96 executed verification probes against a live IRIS
2025.3 server (USER + %SYS) — a 33-row discovery-completeness matrix, 18
hostile-input cases (SQL/path/param injection at both the CLI and MCP doors, with
a dead-port oracle proving refused inputs make zero HTTP requests), 12
CLI↔MCP parity checks (17/18 byte-equal results; the one diff is an
"Error: " prefix), a full bench re-run, and a docs-vs-behavior audit of every
published claim.

**What the feature delivers.** The tool surface grows to 29 MCP tools; the
discovery matrix says the index finds class members/trees (incl. inherited,
diamond-safe BFS with honest `truncated` flags), code text across compiled *and*
uncompiled docs, metadata attributes, labels with true line numbers, %SYS and
generated code behind explicit flags. 11/33 needs resolve as fully
found-as-documented, 15 resolve with a flag/mask/workaround, 7 are honest
"server-side impossible" answers.

**Known limits (now documented where developers look).**
- `used_by`/`depends_on` walk `DependsOn`/`Super` dictionary edges only; on
  2025.3 those fields do not record method-body call sites, so a class used
  only via `##class(X).M()` is invisible to the graph. Pair with
  `search '##class(X)' --no-regex` for complete impact analysis.
- Macro `$def` definitions and CSP pages are scanned only when you pass
  `--documents '*.inc'` / `'*.csp'` (default masks are cls/mac/int).
- `class_members` lists the four Compiled class-member tables; Queries,
  Triggers, Indices and Procedures need literal search or `execute_sql`.
- Global/stream *data* content is out of reach of the search endpoint by design.

**Findings fixed in this review** (all verified live, before/after):
- Unknown or alias class names (e.g. `%RegisteredObject`) returned misleading
  all-zero member counts; they now answer an honest not-found naming the dictionary,
  pointing to the canonical name (aliases have no SQL-visible column to
  rewrite against on 2025.3 — probed).
- The `namespace` parameter now has the same pre-flight allowlist guard as
  class names (refused before any HTTP request on all four surfaces; hostile
  vectors previously reached the wire, benign but unguarded).
- `--documents ''` from the CLI caused a server HTTP 400; empty document
  strings now fall back to the default masks.
- Stale "server hard-caps at 200" wording in the tool schema/help/comments
  corrected — 2025.3 honours higher values; `truncated` semantics explained
  (server notice OR client match-count rule; never lies).
- Docs corrected: bench precision headline 20/21 → 20/20 (the one miss was a
  stale fixture since deleted); `used_by` scoped as an impact hint; member-table
  exclusions listed; UDL header-line search caveat; "2 HTTP round trips" bench
  framing with machine-specific ratio note.

**Gate summary.** `cargo fmt` clean; clippy `-Dwarnings` green on the current
stable and pinned 1.99.0 toolchains; 123 unit/wiremock tests green; live contract
suite 23/23 against the server; MCP probe reports 29 tools on both the stdio and
HTTP doors; docs-drift gate 0 failures; `mkdocs build --strict` clean; full bench
re-run agrees with published recall (20/20), graph (11/11) and truncation-cliff
numbers. All test fixtures removed and verified against the server dictionary.

**Recommendation: approve.** One structural blind spot (call-graph references) is an
explicitly deferred Tier-3 item with a working literal-search workaround, and it
is now stated honestly in the docs.
```

## CLOSE (chain close-out, specifier re-check at merged 7cfcb85)

- Merge check: architect tip 7cfcb85 fast-forwarded onto the specifier lane; tree
  contains the full chain (F_PLAN 96 rows -> coder 2d3dfec -> refactorer 8ffcffe -> architect 7cfcb85).
- Spot-check 1 (this file): UNRESOLVED empty, PR-APPROVE line present, fenced
  public review-comment block exists and is free of internal process terms
  (one-word sanitization applied here: "Verdict:" -> "Recommendation:" in the
  block's closing sentence, per the F_PLAN D-07 writing rule).
- Spot-check 2 (F_FINDINGS): all 12 rows closed with reasons - F-01..F-10
  fixed/doc-fixed citing commits 2c3d795/6e7e715/e25353f, D-01/D-02 wontfix with
  mutation-tested reasons.
- Spot-check 3 (F_MATRIX): 95 executed table rows (M 33, H 18, P 12, B 11,
  DC 15, D 6) + D-07 process rule proven by the gate self-trip = 96/96 against
  the F_PLAN assignments; zero BLOCKED, zero assumed verdicts.
- Chain state: closed. No pushes; PR comment to be posted by the human/coordinator
  from the fenced block above.
