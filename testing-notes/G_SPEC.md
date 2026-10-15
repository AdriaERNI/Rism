# G_SPEC — frozen contract for the reference graph + partials

Decides everything implementation must NOT re-litigate. Evidence:
testing-notes/G_RESEARCH.md (live probes, 2026-10-14, rism-iris 2025.3).
Ranking rule applied: server-mechanism > parse-on-fetch (dep-free) >
persistent index.

## 1. Chosen mechanism

**Call graph (gap A):** NO server xref exists on 2025.3 (all probed:
Atelier xref/Graph endpoints 404; `^XDEF`/`^%CC` absent; `%SyntaxColor` is a
tokenizer with no token-class 12 on this build; `%Dictionary.CompiledCode`
does not exist; `CompiledUDLText` empty; `Rtn`/`RtnLabel` carry pointers,
not bodies). Therefore ship **parse-on-fetch with server-side narrowing**,
zero new deps, no persistent index:

1. narrow candidate docs with `action/search` (literal mode) +
   `%Dictionary` SQL (`CompiledProperty.SqlComputeCode`,
   `CompiledTrigger.Code`, `ClassDefinition.Super`),
2. fetch each candidate doc ONCE via the existing `doc` endpoint,
3. classify call sites with a **std-only line scanner** (no regex crate, no
   `unsafe`; `str::find`/`split` + char checks; the prototype in
   G_RESEARCH proves the pattern set classifies 10/10 fixture edges),
4. return resolved edges + an honest `unresolved` bucket.

`objectscript-core` (crates.io, MIT, InterSystems) is REJECTED as in-tree
substrate: its tree-sitter grammar crates carry `cc` build-deps → C
compilation → violates the pure-Rust dep gate. Recorded as the substrate if
a heavyweight design is ever revisited out-of-tree.

**Partials (gap B):** server SQL only (top of ranking) + mask fixes:
- `class_members` gains member kinds Queries / Triggers / Indices /
  Procedures (SQL: `%Dictionary.CompiledQuery|CompiledTrigger|CompiledIndex`,
  Procedures = `CompiledMethod WHERE SqlProc=1`; `CompiledProcedure` table
  does not exist — verified).
- Namespace-wide census: DOCUMENT exact SQL one-liners (verified 0.11–0.16 s)
  in docs; NO new census tool.
- Global/stream DATA + production config ROWS: NOT source text — document
  `execute_sql` as the path; explicitly listed as impossible-for-search (§7).
- `.inc` joins the default scan masks; `.csp` stays opt-in (both verified
  scannable when masked).

## 2. New tool: `find_references` (MCP) — surface 29 → 30

Args (`deny_unknown_fields`, mirrors `class_members` style):

```rust
pub struct FindReferencesArgs {
    /// Target grammar (validated BEFORE any HTTP, names.rs):
    ///   "Package.Class.Member" | "Package.Class" | "^RoutineName"
    /// (leading ^ selects a routine target; otherwise the whole string is a
    ///  class name — member suffix must itself be a valid member name when
    ///  the class part resolves to no member-less reading is attempted
    ///  first: try `Class.Member` split at LAST dot only when the left part
    ///  passes valid_class_name; a bare name that is not a class answers
    ///  honest not-found.)
    pub target: String,
    pub namespace: Option<String>,
    /// Include system docs in the scan (default false; gate: system scan
    /// measured 3.09 s wall — must not be the default).
    pub include_system: Option<bool>,
    /// Include generated/compiled .int docs (default false: they carry
    /// macro-expanded edges but flood generated global refs — precision
    /// collapse, see G_RESEARCH 2b).
    pub include_generated: Option<bool>,
    /// Max resolved+unresolved edges returned (default: settings
    /// search_max_results).
    pub max_results: Option<u32>,
}
```

Result shape:

```rust
pub struct FindReferencesResult {
    pub namespace: String,
    pub target: String,               // canonicalized
    pub kind: String,                 // "class" | "class_member" | "routine"
    pub edges: Vec<RefEdge>,          // RESOLVED — scanner proved the reference
    pub unresolved: Vec<UnresolvedRef>, // dynamic forms, never silently dropped
    pub docs_scanned: usize,
    pub truncated: bool,              // hit max_results or search truncation
}
pub struct RefEdge {
    pub doc: String,                  // "FProbeG.Caller.cls"
    pub member: Option<String>,       // containing member/label if attributed
    pub line: Option<u32>,
    pub text: String,                 // the source line, trimmed, ≤200 chars
    pub via: String,                  // "class_call"|"self_call"|"routine_do"
                                      // |"super"|"computed"|"trigger"
}
pub struct UnresolvedRef {
    pub doc: String, pub member: Option<String>, pub line: Option<u32>,
    pub text: String,
    pub hint: String,                 // "oref_call"|"dynamic_string"|"macro"
}
```

Semantics frozen:
- `Class` target → edges = `##class(Class)`, `..`-self only inside Class,
  `Extends`/`Super` rows (via=super, from SQL), `$system`-style `Class.`
  static refs NOT scanned (regex-free scanner: literal `Class` word alone is
  precision poison — out of scope, document).
- `Class.Member` → additionally every `##class(Class).Member(` occurrence;
  `o.Member(` textual hits go to unresolved (oref_call), `"Member"` inside
  `$zobjmethod`/quotes → unresolved (dynamic_string), `$$$X` usage → the
  macro definition search runs (`--inc`-masked, `define $$$X`) and hits are
  reported unresolved(macro) pointing at the expansion owner doc.
- `^Routine` → `Do ^R`/`Goto ^R`/`$zobjmethod(^R`/`^R(` literal hits
  (resolved routine_do); a method-name query CANNOT find routine edges by
  construction — documented in the recall table.
- Class-not-found → honest `Error::not_found` like class_members (reuse
  `class_exists` + `not_found_msg`), never empty success.
- All names guard-before-HTTP incl. namespace (`valid_namespace`) and a new
  `valid_routine_name` (letters/digits/`_`/`-`/`.`, ≤255, no `^` except the
  target grammar prefix, which the parser strips before interpolation).

## 3. `class_members` extension (non-breaking additive JSON)

`MembersKind` gains `Queries`, `Triggers`, `Indices`, `Procedures` (and
`All` = current four + queries/triggers/indices; methods/properties/
parameters unchanged). Result gains:

```rust
pub queries: Vec<MemberQuery>,      // name, formal_spec, sql_name, sql_view(bool), type
pub triggers: Vec<MemberTrigger>,   // name, event, time, code_mode
pub indices: Vec<MemberIndex>,      // name, type, properties
pub procedures: Vec<MemberProcedure>, // name, return_type  (SqlProc=1 rows)
```

SQL (verified live): `SELECT Name, FormalSpec, SqlName, SqlView, Type FROM
%Dictionary.CompiledQuery WHERE parent='{c}' [AND Origin='{c}']` etc.
`Time` is a T-SQL keyword → column `_Time` aliased `AS Time` in SELECT
(verified working). counts gains the four fields.

## 4. Wire-compat decisions (each with PRO/CON)

- **DEFAULT_DOCUMENTS += `*.inc`** (search + refs). PRO: macros invisible
  under today's defaults = silent blind spot, docs already call it a footgun.
  CON: existing `search_code` calls may return extra `.inc` docs (more
  matches, same ranking) → results CHANGE without an error. Accepted: recall
  over byte-identical results; recorded in docs + CHANGELOG-adjacent note.
  `.mac` stays in defaults and covers doc-API-made `.inc` (quirk: doc-API PUT
  of a `.inc` stores it as `.mac` on 2025.3 — verified; scanning both masks
  is therefore already correct).
- **`.csp` NOT added to defaults** — PRO opt-in only: pages are markup;
  literal noise dominates. No breaking change.
- **class_members JSON: additive fields only** — no renames/removals;
  existing consumers unaffected (`deny_unknown_fields` is args-side only).
- **`find_references` is net-new** — no compat surface.

## 5. Recall protocol + perf gates (measured; gate = PASS/FAIL in G_PROOFS)

Fixture set: the 10 planted edges + decoy from G_RESEARCH (FProbeG.* docs,
`FProbeG*`-prefixed → keep exact names used in probes so numbers
reconcile; load/compile/teardown in the live test, cleanup verified to 0).

Required recall of `find_references` on the fixtures:

| query | required |
|---|---|
| `FProbeG.Target.TgtMethod` | E1 E2 E4 E5 E6 E7 E8 E9 E10 — E7/E8 resolved, E1/E2/E6 resolved, E4/E5/E10 **in unresolved with correct hint**, ≥ 9/10 docs surfaced, E3 NOT required (method query) |
| `^FProbeG.Mid` | E3 resolved |
| `FProbeG.Target` | E1 E2 E6(+super/Sub) resolved, class-call edges present |
| workaround baseline (recorded, not shipped as the answer) | 5/10 exact-literal, 9/10 name-word |
| precision | decoy comment-line hit appears ONLY as unresolved(dynamic_string) or is excluded by string-lexer state — zero resolved decoy edges allowed |

Perf (project gates; measured on rism-iris USER, 1,577 user
classes / 5,916 docs incl. system):
- `find_references` with member: < 2.0 s wall p95 (3 runs) — prototype
  measured 0.6–1.3 s.
- `find_references --include-system`: report timing, no gate breach claim
  (3.09 s measured baseline = why system is off by default).
- census SQL < 1 s. Build: N/A — persistent index REJECTED (document once:
  bounded-cost per-query scan measured faster than any checkout-free index
  build path; no staleness machinery shipped).

## 6. CLI parity + docs + tests

- `rism graph refs <TARGET> [--namespace NS] [--system] [--generated]
  [--max N] [--format json|table]` — table shows `doc(member:line) via —
  text`; unresolved section marked `?hint`.
- MCP: `#[tool] find_references`, annotations read_only_hint=true, title
  "Find References"; description ≤ budget; instructions field gains ONE line
  if < 2048 holds (measure; else skip — never budget-break).
- Tool count 29 → 30: every `*.md` "N tools" claim + mcp-tools.md entry +
  drift gate (`bench/drift_port.py` 0 fails).
- Unit tests (U*): target-grammar parser (Class.Member / Class / ^R / junk),
  guards (refusal = ZERO HTTP calls, wiremock), scanner classification on
  fixture strings (all 10 edge shapes + decoy + comment + `///` doc line),
  class_members kind routing + `_Time` alias SQL shape, deny_unknown_fields.
- Live tests (L*, tool_matrix): load fixtures → three recall queries →
  assert §5 table → delete docs → assert census 0 (FProbeG* classes+
  routines). class_members census fields on a fixture class. search default
  mask now includes *.inc (probe an .inc-shaped fixture via the .mac quirk +
  an SQL-doc'd note).
- mcp_probe both doors, mkdocs --strict, fmt, clippy -Dwarnings host +
  `rustup run 1.99.0`, cargo test -q.

## 7. Document as still-impossible (code-search.md rewrite)

- server-side xref/call-graph store: none on 2025.3 (probed);
- global/stream DATA and production config ROW search via source search
  (use `execute_sql` — document the exact queries);
- truly assembled identifiers (`"FProbeG"_"Target"` with NO literal
  fragment) — reported unresolved(max $zobjmethod/dynamic-string forms
  found);
- `$zf`-evaluated runtime-only dispatch;
- cross-namespace references (one namespace per call);
- incremental persistent index (deliberate design decision, cost model in
  §5, revisit only if per-query latency fails real corpora).
