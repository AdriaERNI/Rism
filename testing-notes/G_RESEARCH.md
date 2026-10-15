# G_RESEARCH — mechanisms for the call-graph + partials gaps (PR #22)

Every candidate: source URL + claim + LIVE probe verdict (CONFIRMED /
REFUTED / UNTESTABLE-HERE) + fit. Probes run 2026-10-14 against the shared
CI container `rism-iris` (IRIS for UNIX 2025.3 Build 226U, Atelier API v8),
namespace `USER`, `_SYSTEM` creds, endpoints `action/query`, `action/search`,
`doc/{name}`, `action/compile`, and `rism exec` (terminal WS).
Ranking rule (constitution): server-mechanism > parse-on-fetch (dep-free) >
persistent index; applied per gap.

## Fixtures + baseline recall (measured, not assumed)

10 planted edges toward `FProbeG.Target::TgtMethod` / `^FProbeG.Mid`, loaded
via `doc` PUT + `action/compile?flags=cku`, then removed (verified: 0 rows in
`ClassDefinition LIKE 'FProbeG%'`, 0 in `StudioOpenDialog('FProbeG*')`):

| edge | form | doc |
|---|---|---|
| E1 | `set t = ##class(FProbeG.Target).TgtMethod(1)` | FProbeG.Caller.cls |
| E2 | `set y = ##class(FProbeG.Target).TgtMethod(3)` | FProbeG.EdgeR.mac |
| E3 | `Do ^FProbeG.Mid` (routine-target edge) | FProbeG.EdgeR.mac |
| E4 | `set r = $zobjmethod(obj, "TgtMethod", 4)` | FProbeG.Caller.cls |
| E5 | name assembled from strings, `do $ZF(-4,1,cmd)` | FProbeG.Caller.cls |
| E6 | `Quit ..TgtMethod(6)` (self/inherited) | FProbeG.Target.cls |
| E7 | trigger body `write ##class(...).TgtMethod(7)` | FProbeG.Target.cls |
| E8 | `SqlComputeCode = {...TgtMethod(8)}` property ext | FProbeG.Target.cls |
| E9 | macro body `Set Definition $$$X = {##class(...)...}` | FProbeG.Macros(.mac) |
| E10 | oref-variable `set r = o.TgtMethod(10)` | FProbeG.Sub.cls |
| decoy | the bare word inside a `write "..."` comment-like string | FProbeG.EdgeR.mac |

Baseline recall of the CURRENT documented workaround
(`search '<literal>' --no-regex`, `rism` 0.2.2 @ 11c071b):

| variant | query | recall | notes |
|---|---|---|---|
| A exact-qualified | `##class(FProbeG.Target).TgtMethod` | **5/10** (E1 E2 E7 E8 E9) | misses ..-self, oref, dynamic, Do^ |
| B widened word | `TgtMethod` (default masks *.cls,*.mac,*.int) | **9/10** (all but E3) | E3 is a ROUTINE-target edge — a method-reference query cannot contain it by construction; 1 decoy hit costs precision |
| C widened + `*.inc` | same + `--documents '*.cls,*.mac,*.int,*.inc'` | 9/10 | identical on this fixture because the doc API stored the `.inc` as `.mac` (see quirk below) |

Headline: **the ##class-literal workaround recall is 50% exact / 90%
name-word on a 10-edge fixture set; the only structurally-uncoverable class
of misses is pure dynamic dispatch where even the member name never appears
as literal text (E5 was found textually — its string contains the name).**
Fixture corpus + measurement script: `.worktrees/spec` scratch session;
recall logic to be re-implemented as a bench (G_SPEC §6).

## Candidate-by-candidate

### 1. crates.io `objectscript-core` (tree-sitter parser + DependencyGraph)

- URL: https://crates.io/crates/objectscript-core — v0.1.9, 2026-10-02,
  **MIT**, author Hannah Kimura (InterSystems). No repo/homepage metadata.
- URL: https://docs.rs/crate/objectscript-core/latest — claim: full semantic
  model, `dependency_tracker::DependencyGraph` (caller→callee petgraph,
  `all_ancestors`/`closest_definers`), `override_index`, bulk (rayon) +
  incremental indexing, definition/implementation queries. CONFIRMED by
  reading the shipped source (crate tarball: 17k LoC, README module table).
- The `.crate` tarball's `.cargo_vcs_info.json` points at repo path
  `objectscript-lsp/crates/objectscript-core` — the monorepo is NOT public
  on GitHub (`api.github.com/repos/intersystems/objectscript-lsp` → 404,
  live-probed 2026-10-14).
- Grammar: https://github.com/intersystems/tree-sitter-objectscript — C,
  MIT, active (pushed 2026-10-09). `tree-sitter-objectscript` crate deps
  include **`cc ^1.0` (build)** → the grammars are C code compiled at build.
- Fit verdict: **REFUTED as an in-tree substrate.**
  1. It pulls `tree-sitter` + 3 grammar crates that compile C — breaks the
     stated design constraint "pure-Rust deps, no unsafe, no C".
  2. Its API is file/URL-workspace oriented (`ProjectState`/`walkdir`);
     server-sourced content would need an adapter layer anyway.
  3. `tower-lsp`, `petgraph`, `rayon`, `evalexpr`, `parking_lot` — five
     direct deps Rism does not need for one query.
  4. Live build test: `cargo add objectscript-core@0.1.9` in a clean crate
    built OK (2m03s, host has gcc) — feasibility yes, policy no.
  Worth recording: this crate proves the TREE-SITTER route exists
  (InterSystems' own objectscript-lsp uses it); if a heavier parser-based
  design is ever revisited, this is the substrate, not a new parser.

### 2. `%Dictionary.CompiledMethod.Rtn` / `RtnLabel` as a body source

- Claim (research lead / docs): compiled routine/label fields may expose
  method bodies as queryable text.
- Probe: `SELECT TOP 5 Name, RtnLabel FROM %Dictionary.CompiledMethod`
  → `RtnLabel` = `Label^Routine.N` pointers (195,902 non-null rows), no body
  text; field `Rtn` does not exist (`SQLCODE -29`, live).
- **REFUTED** as body source. USEFUL for: mapping member → compiled routine
  name (see 2b).

### 2b. Compiled class routines `Class.N.int` ARE readable docs

- Probe: `GET doc/FProbeG.Target.1.int` → 1000+ lines including the
  materialized `SqlComputeCode` (`Set %val = ##class(FProbeG.Target).TgtMethod(8)`),
  trigger bodies, and UDL comment-wrapped user method bodies. `action/search`
  with `gen=1` scans them (Found 2000+… capped; 0.29 s with max=2000).
- **CONFIRMED.** Fit: a completeness backstop (macro-expanded call sites
  exist ONLY in .int), but full of generated noise (`^FProbeG.TargetD`
  global refs from storage code, $zu(115,4)…) — pollutes precision.
  Decision: expose via `include_generated` flag (default **false**).

### 3. `%Dictionary.CompiledCode`

- Probe: `SELECT ... FROM %Dictionary.CompiledCode` → `SQLCODE -30 Table not
  found`; no such class in `CompiledClass`. **REFUTED** (issue #17 lead
  re-check: this table does not exist on 2025.3; the body-bearing idea in the
  lead note came from other products' dictionaries).

### 4. `%Dictionary.CompiledUDLText`

- Probe: `SELECT COUNT(*) FROM %Dictionary.CompiledUDLText` → **0** in USER;
  columns (Category/Content/TextType) exist. **CONFIRMED EMPTY** — matches
  issue #17; the property-UDL-text table exists for Property members only.
  Not a census shortcut for methods; SQL census below is the body-free path.

### 5. `%Library.SyntaxColor` (the %SyntaxColor lead)

- Claim A (issue #17): not installed on Community. **REFUTED** —
  `%Library.SyntaxColor` + `SyntaxColorReader` compile classes exist
  (`SELECT Name FROM %Dictionary.CompiledClass WHERE Name LIKE '%SyntaxColor%'`
  → both rows) and `Color()` runs: HTML mode and **CSV mode** both produced
  output live (`COS,<type>,<token>` lines; `sc=1`).
- Claim B (IDC post, community.intersystems.com "Caché Studio: find and
  record all global variables…"): global refs appear as token class **COS 12**.
  **REFUTED on 2025.3** for our purpose: in the live CSV of
  `Do ^FProbeG.Mid` / `set x=##class(FProbeG.Target).TgtMethod(1)` /
  `write ..TgtMethod(6)`, tokens carry types 0/1/3/4/20 only
  (`COS,0,^`,`COS,0,FProbeG`; `COS,4,..` marks the self-oref; keywords 3;
  no 12 anywhere). The class is a syntax-highlight DLL front-end, not an
  xref; nothing it reports that a line-scanner can't.
- Fit: **rejected for references** (no resolution power). Recorded: `COS,4,..`
  confirms `..` is lexically distinct — a plain-text parser sees the same.
  URL: https://community.intersystems.com/post/cach%C3%A9-studio-how-find-and-record-all-global-variables-referenced-routine-or-collection

### 6. vscode-objectscript documentIndex / language-server xrefs

- Claim (research lead): documentIndex is CLIENT-side workspace index
  only; the language server resolves via %Dictionary SQL +
  `RoutineMgr_StudioOpenDialog` table functions.
- Probe (server): `GET USER/xref`, `USER/action/xref`,
  `USER/Graph/Nodes` → **404 all** (re-check of issue #17: CONFIRMED absent).
- `%Library.RoutineMgr_StudioOpenDialog` table function **CONFIRMED**:
  lists cls/mac/int/inc/csp by mask+flags (`*.cls` → %Api.Atelier.cls;
  USER counts: 11 .mac + 152 .int + 250 .inc user docs, 5,916 docs total
  incl. system). This is the doc-name census partial — no content.
- `documentIndex` in the extension does not build xrefs server-side
  (lead; consistent with the 404s). **No server xref machinery exists.**

### 7. Legacy Studio xref globals (`^XDEF`, `^%CC`)

- Probe: `write $data(^XDEF), $data(^%CC)` (USER) → `0`, `0`.
- **CONFIRMED absent** (never built; legacy Studio-era artifact). Building
  them would need closed compiler hooks — dead end, same verdict as #17.

### 8. `%Studio.Project` search internals (FindInFiles*, searchItem*)

- Probe: method census lists `FindInFiles`, `FindInFilesRegex`,
  `FindInProject`, `searchItem`, `searchClassNode` (+`%Studio.WebServerClass`
  absent) — **CONFIRMED these exist**. They ARE the engine behind
  `action/search` (same text scan). No xref/xcall semantics.
  Fit: no new capability; documents that Tier-1's scan is the vendor engine.

### 9. $SYSTEM.OBJ / other server "graph" APIs

- `SELECT Name FROM %Dictionary.CompiledClass WHERE Name LIKE '%XRef%'` →
  only %TSQL/%XSQL message-xref utilities (not code xrefs). No
  `$SYSTEM.OBJ.XRef`-style method found in `%Dictionary.CompiledMethod` for
  `%SYSTEM.OBJ`. **No hidden server call-graph API on 2025.3.**

### 10. Partials — census via SQL (server mechanism)

All live-verified on 2025.3 (USER + SYS):

| need | mechanism | probe result |
|---|---|---|
| Queries per class | `%Dictionary.CompiledQuery` (Name, FormalSpec, SqlName, SqlView, SqlViewName, Type) | FProbeG.Target→QAll row ok; 722 rows namespace-wide join in 0.16 s |
| Triggers | `%Dictionary.CompiledTrigger` (Name, Event, Code, CodeMode, Foreach, UpdateColumnList; `Time` is a keyword → SQL needs the storage column `_Time`, verified: `SELECT _Time AS Tm` works, plain `Time` → SQLCODE -12) | trg1 row + Code text `write ##class(...)` CONFIRMED |
| Indices | `%Dictionary.CompiledIndex` (Name, Type, Properties, Function, WhereCondition) | idx1 row ok |
| Stored procs | `%Dictionary.CompiledMethod WHERE SqlProc=1` (no CompiledProcedure table — `-30`) | Ens.Alerting rows ok |
| namespace-wide member census | JOIN Compiled* to `ClassDefinition C ON …parent=C.Name WHERE C.Name NOT LIKE '\%%' ESCAPE '\'` | 7,118 classes / 1,577 user classes in 0.11 s |
| Global/stream DATA | SQL over the storage maps: `%Dictionary.CompiledStorageSQLMapData.RetrievalCode` etc. — DATA itself is plain globals, NOT source text | `action/search` cannot see it (documented impossibility) |
| production config ROWS | tables `Ens.Config.*` are queryable via `execute_sql` (Ensemble enabled on this container: feature DEEPSEE/ENSEMBLE listed at /api/atelier root) | documented as the one-liner answer, not a new tool |
| .inc scan | mask honoured (`'*.inc'` → `%BigData.*.inc(143): $SYSTEM…` with `sys=1`) **CONFIRMED scanned when masked**; QUIRK: `.inc` docs created through the Atelier `doc` API on 2025.3 are stored as `.mac` (PUT FProbeG.Macros.inc → cat RTN, doc exists only as `FProbeG.Macros.mac`; `.inc` GET → 16005). So a mask-`*.inc`-only scan misses doc-API-made macros → refs must scan BOTH `.mac` and `.inc`. |
| .csp scan | mask `*.csp` accepted (`Not found.` on this container — USER has none; `%CSP` pages live in the app's CSP dir; scanning them is mask-only, engine confirmed the same) |

## Perf-gate measurements (for G_SPEC §5)

- No index is shipped → the "build < 5 min" gate applies only to a
  persistent-index design (rejected); recorded as N/A with reason.
- Query < 2 s: fixture scan+fetch+classify measured end-to-end via HTTP in
  0.6–1.3 s wall (search 0.29–0.84 s, ~6 doc GETs < 0.05 s each incl. a
  2,579-line system doc); `rism search --system` against 5,916 docs: 3.09 s
  wall (system-scope is explicitly gated OFF by default → user-namespace
  default stays < 2 s). Census SQL: 0.11–0.16 s.

## Bottom line

- **Call graph:** no server mechanism exists on 2025.3 (xref endpoints 404,
  xref globals empty, SyntaxColor = tokenizer, CompiledCode/UDLText dead).
  The best available mechanism is **SQL-narrow + literal-search-narrow +
  parse-on-fetch** (dep-free `std::str`/`regex`-free scanner), shipping
  resolved edges + an honest `unresolved` bucket. That is the Tier-3 design
  from docs/code-search.md — WITHOUT the persistent index (bounded cost per
  query beat the index on measured latency, so an index would be pure
  staleness risk). `objectscript-core` is recorded as the parser substrate
  if a heavyweight design is ever revisited.
- **Partials:** pure SQL census (server mechanism — top of ranking) +
  mask documentation/flag work; two wire-compat decisions frozen in G_SPEC.
