# E_SPEC — frozen contract, mission E (issue #17 Tiers 1+2)

Status: FROZEN by specifier lane 2026-10-12. Changes only via ledger note.
Base: v0.2.2 tip 75272e8, branch `feat/code-search-graph`.

§1 tool contract · §2 search wire/normalize · §3 graph SQL · §4 CLI ·
§5 settings · §6 bench protocol · §7 test matrix.

All "probe:" lines below are live-verified against the dev container
(IRIS 2025.3, Atelier negotiated v8, localhost:52773, USER + %25SYS) by the
specifier lane on 2026-10-12; they EXTEND the settled issue-#17 facts, never
contradict them.

## §1 The four tools (frozen names + arg fields)

All args structs: `#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]` (pattern: `src/tools/sql.rs:14`). Every field
doc-comment feeds both the MCP JSON schema and `clap` help. All four MCP
stubs carry `annotations(title = ..., read_only_hint = true,
destructive_hint = false)` — all four are pure reads; no endpoint other than
the existing `action/search` GET and `action/query` POST is touched.
Tool count 29 → 33 (README, docs/mcp-tools.md ×4 sections, mcp_probe count).

### search_code — `src/tools/search.rs::SearchCodeArgs`

| field | type | default | wire param |
|---|---|---|---|
| `query` | `String` | required | `query` (URL-encoded via `urlencoding`) |
| `namespace` | `Option<String>` | settings ns | in URL path |
| `documents` | `Option<Vec<String>>` | `["*.cls","*.mac","*.int"]` | `documents` (`,`-joined, then encoded) |
| `regex` | `Option<bool>` | `true` (server default) | `regex` as `1`/`0`, omitted when `None` |
| `include_system` | `Option<bool>` | `false` | `sys` `1`/`0`, omitted when `None` |
| `include_generated` | `Option<bool>` | `false` | `gen` `1`/`0`, omitted when `None` |
| `case_sensitive` | `Option<bool>` | unset (server default) | `case` `1`/`0` — ONLY when `Some` |
| `whole_word` | `Option<bool>` | unset | `word` `1`/`0` — ONLY when `Some` |
| `max_results` | `Option<u32>` | `settings.search_max_results` (200) | `max` |

Decisions: no `wild` param is exposed (real but undocumented; not needed —
masks cover document selection). `regex=None` and `regex=Some(true)` both
work; the wire emits `regex=1` when `Some(true)`, omits when `None` (server
default is already 1). Values of `max_results > 200` are sent as-is — the
server hard-caps at 200 (issue fact) and the response marks `truncated`
(§2); no client-side clamp, no paging (none exists).

### find_subclasses — `src/tools/graph.rs::FindSubclassesArgs`

`class: String` (required, `valid_class_name` gate), `namespace:
Option<String>`, `depth: Option<u32>` — max BFS levels, `Some(1)` = direct
children only, default `settings.graph_depth` = 10; `Some(0)` →
`Error::Config("depth must be >= 1")`; values > 64 clamp to 64.
Algorithm: bounded `Super [ 'F'` walk (§3). No `include_self` — the
result excludes `class` itself.

### class_members — `src/tools/graph.rs::ClassMembersArgs`

`class: String` (gate), `namespace: Option<String>`,
`kind: Option<MembersKind>` where
`pub enum MembersKind { All, Methods, Properties, Parameters, XData }`
(serde rename_all = "snake_case"; default `All`. XData is IN the enum:
`%Dictionary.CompiledXData` names are queryable (probe), only its `Data`
stream is not — the entry carries name only),
`include_inherited: Option<bool>` default `false` → `true` drops the
`Origin = class` filter (§3). No depth field.

### class_dependencies — `src/tools/graph.rs::ClassDependenciesArgs`

`class: String` (gate), `namespace: Option<String>`,
`direction: Option<DependencyDirection>` —
`pub enum DependencyDirection { DependsOn, UsedBy, Both }` default `Both`
(`used_by` = the impact query), `depth: Option<u32>` default `1` (applies to
the `used_by` walk only; `depends_on` is always the 1-hop dictionary field;
> 64 clamps to 64).

### Response JSON shapes (each tool's `structuredContent`; the text block is
`serde_json::to_string_pretty` of the same value — sql.rs pattern)

search_code:
```json
{ "namespace": "USER", "query": "...", "documents": ["*.cls","*.mac","*.int"],
  "server_tally": "Found 1 occurrence/s in 1 file/s.",
  "truncated": false, "doc_count": 1, "match_count": 1,
  "results": [ { "doc": "My.Cls.cls",
      "matches": [ {"member": "Set", "attr": null, "line": null, "label": null, "text": "Set"},
                   {"member": null, "attr": null, "line": 5, "label": null, "text": "Quit x"} ] } ] }
```
`line` is ALWAYS `u32|null` after normalization (§2); `label` (string|null,
always present in the schema) holds the raw non-numeric line token, e.g.
`"Description+32"`, when a routine/member line can't parse as u32; `attr`
appears only
for class members (e.g. `"Description"`); `member` is the member name for
class-file hits and `null` for routine-line hits, where `line` is the
1-based routine line.

find_subclasses:
```json
{ "class": "X", "depth": 10, "count": 3, "truncated": false,
  "subclasses": [ {"name": "Y", "super": "X", "level": 1} ] }
```
`level` = BFS distance from `class` (1 = direct child). `truncated` = true
when the walk hit the depth clamp.

class_members:
```json
{ "class": "X", "kind": "all", "include_inherited": false,
  "counts": {"methods": 2, "properties": 1, "parameters": 0, "xdata": 0},
  "methods":    [ {"name": "M", "class_method": true, "formal_spec": "a=%String",
                    "return_type": "%Status", "sql_proc": false, "origin": "X"} ],
  "properties": [ {"name": "P", "type": "%%String", "collection": "",
                    "origin": "X"} ],
  "parameters": [ {"name": "CAPTION", "type": null, "origin": "X"} ],
  "xdata":      [ {"name": "XML"} ] }
```
`origin` = declaring class (server column; with `include_inherited=false`
every row has `origin == class`). Field names of the underlying
`%Dictionary.Compiled*` tables verified live (§3 probes).

class_dependencies:
```json
{ "class": "X", "direction": "both", "depth": 1, "truncated": false,
  "depends_on": [ {"name": "Y", "kind": "class", "level": 1} ],
  "used_by":    [ {"name": "Z", "kind": "class", "level": 1} ] }
```
`kind`: "class" | "routine" | "other" (heuristic from the raw DependsOn
entry — contains no dot → routine/§3). Only the direction(s) requested are
populated; the other array is `[]` (field always present — schema stability).

Arg-struct unit tests pin deny_unknown_fields for all 4 (sql.rs test pattern).

## §2 search_code wire + normalization

Request (new `src/iris/search.rs`, qs pattern `src/iris/documents.rs:50-61`):

```
GET {api_url(ns)}/action/search?query={enc}&documents={enc("*.cls,*.mac,*.int")}
    &regex=1&sys=0&gen=0&case=1&word=1&max=200
```

- Path ns goes through `api_url()` (`%`→`%25`). Every param VALUE goes
  through `urlencoding::encode` (existing dep) — query is user-controlled
  regex text, so encoding is mandatory, not cosmetic.
- Params are emitted only when `Some`/non-default per §1 table: the wire
  never sends `case=`/`word=` unless the caller set them (probe: server
  defaults differ per param — sending explicit values would change results
  of callers who didn't ask for them).
- API version: none is pinned in the URL — `api_url` uses the negotiated
  version (probe: v8 on IRIS 2025.3; `action/search` answered at v2 AND v8;
  at v1 it returned an empty envelope for a query that v8 found matches —
  PROBE: `v1 USER action/search?query=Set` → `result:{}` vs v8 → hit. So
  v2+ is required; negotiation already gives ≥2 on every supported server).
  Error mapping: HTTP 404 (server without search) → `Error::HttpStatus` as
  everywhere; do NOT translate to DocNotFound.
- Empty-vs-missing `result`: probe shows `result:{}` (object) when the
  server found nothing at old API versions and `result:[]` (array) when it
  searched and found nothing. Parser: `env.result.get("result")` → treat
  absent, null, `{}` and `[]` all as zero results; anything else that isn't
  an array → `Error::InvalidEnvelope`.

Envelope → `SearchResult` normalization (unit-testable pure fn
`parse_search_result(env, requested_max) -> Result<SearchResult>`):

1. `doc` (string) → entry; missing doc key → InvalidEnvelope.
2. `matches[]`: each match has optional `member`, optional `attr`,
   optional `line`, present-or-absent `text` (missing → "").
3. LINE NORMALIZATION (issue fact: int for class members, string for
   routine lines; probe confirms both shapes in the wild:
   `{"line":"5"}` string for `%SYS.Access.int`, absent for .cls member
   hits): accept `Value::Number` or `Value::String`; parse to `u32`; a
   non-numeric string (e.g. a label name like `"Description+32"` — probe:
   `%Net.FtpSession.cls()[Description+32]`) keeps `line: null` and moves the
   raw string into a `label: string|null` companion field. Never error on
   unparseable line — degrade, don't drop the match.
4. `truncated: bool` — true iff the console contains the server's
   truncation notice (probe: `"Found 2 occurrence/s in 2 file/s. Maximum
   search limit reached."`) OR `match_count >= requested max` when the
   notice is absent (belt-and-braces; the marker test string is the
   constant `"Maximum search limit reached"`).
5. `server_tally`: LAST console line that starts with "Found " (probe
   shapes: `Found N occurrence/s in M file/s.` [+ ` Maximum search limit
   reached.`] or `Not found.`). The other console lines (the
   `Searching for '...' in '...'` echo) are NOT kept in the structured
   output — frozen: tally + counts only; the table renderer uses those.
6. Counts `doc_count`/`match_count` are computed client-side from the
   parsed array (server tally text is display-only — its numbers can
   exceed/undercount vs the returned array at the cap).

Wiremock pins (§7): URL query order-insensitive matching via
`query_param` predicates; test asserts encoded forms for `query="a$b.*"`
(`a%24b.%2A%2A`-safe — assert `%24` present, i.e. encode ran), absent
`case`/`word` params when options are None, `line` int AND string parsing,
`{}`-object empty result, truncation flag from console marker.

## §3 Graph SQL (frozen statements)

All graph SQL goes through `crate::iris::sql::query` (POST
`action/query` — body key `query`; already implemented). The endpoint has
NO bind parameters (issue fact), so every interpolated class name passes
`valid_class_name()` BEFORE the SQL string is built — same rule as
`run_tests`. `action/search` never interpolates a class name; its only
gate is URL-encoding (§2).

**`valid_class_name()` relocation (decision):** move the fn verbatim from
`src/tools/testing.rs:26` to a new `src/tools/names.rs` as
`pub(crate) fn`, and add `pub(crate) use crate::tools::names::valid_class_name;`
inside testing.rs so existing call sites/tests don't churn. Rationale:
shared single definition beats duplication (drift risk in a security
predicate); it's wire-invisible so no ledger entry is needed. Move
`valid_identifier` alongside (used by the dependency classifier below).

`%Dictionary` facts (all probed live 2026-10-12, USER, IRIS 2025.3,
Atelier v8):
- `ClassDefinition` cols include `Name, Super, DependsOn`. `Super` is a
  comma-separated list for multiple inheritance (probe:
  `%CPT.JS.Analyse.Function` → `"%RegisteredObject,%CPT.JS.Analyse.Clonable"`,
  no spaces). `Extends`/`Derived` DO NOT exist (issue fact).
- The contains operator `[` works in WHERE (probe: `Super [ 'X'` and
  `DependsOn [ '.'` both returned rows). It is SUBSTRING based:
  `Super [ 'EnsLib.EDI.SEF.Node'` also matches a class whose Super is
  `EnsLib.EDI.SEF.NodeList` (probe) — hence the exact-match client filter
  in every walk below.
- Table functions: `SELECT Name FROM
  %Dictionary.ClassDefinition_SubclassOf('X')` = TRANSITIVE descendants,
  single `Name` column, no parent/level column; `TOP n` and
  `WHERE Name [ 'pfx'` work (probed). `_SuperclassOf` = transitive
  ancestors. NOT used by the tools (no level info); noted for the ledger.
- `CompiledClass` has an `Ancestry` column but it read empty for user
  classes (probe) — the contract never relies on it.
- Member tables: `%Dictionary.CompiledMethod(parent, Name, ClassMethod,
  SqlProc, FormalSpec, ReturnType, Origin)`, `%Dictionary.CompiledProperty
  (parent, Name, Type, Collection, Origin)` (probe: Type fully-qualified,
  e.g. `%Library.String`), `%Dictionary.CompiledParameter(parent, Name,
  Type, Origin)` (Type often empty — nullable in the response; the raw
  default lives in column `_Default`, NOT selected; response has no
  `default` field), `%Dictionary.CompiledXData(parent, Name)` — never
  SELECT its stream cols (issue fact: unreadable). `Origin` = declaring
  class; own members have `Origin = class` (probe: a system class's
  method list filtered by `Origin = <itself>` returned rows;
  `EnsLib.EDI.SEF.Location`'s methods all originate elsewhere).
  `%Dictionary.CompiledMember` / `CompiledProcedure` DO NOT exist
  (probe: SQLCODE -30).

Placeholder legend: `{C}`/`{F}` = class name that passed
`valid_class_name()` (no `'`, `;`, spaces — injection-free by
construction); rows returned by the server are re-validated before being
re-interpolated as frontier values.

### find_subclasses — depth-bounded BFS

```sql
SELECT Name, Super FROM %Dictionary.ClassDefinition
 WHERE Super [ '{F}'
 ORDER BY Name
```
one query per frontier name, level by level, start frontier = `[C]`.
Algorithm (frozen): visited = {C}; for level in 1..=depth: next = ∅; for
each F in frontier: run the query; keep rows where `Super.split(',')`
contains F EXACTLY (substring over-match guard; multi-superclass edges
included); each kept name not in visited joins `next` with
`{name, super: <full Super string>, level}`; visited += name.
frontier = next; empty → done, `truncated=false`. Non-empty frontier when
the loop ends → `truncated=true`. Frontier names failing
`valid_class_name()` are dropped with a `tracing::warn!` (server-sourced
is NOT trusted). Query budget: at most 256 frontier queries per call;
exceeding it sets `truncated=true`.

### class_members — one query per requested table (kind picks which run)

```sql
SELECT Name, ClassMethod, SqlProc, FormalSpec, ReturnType, Origin
  FROM %Dictionary.CompiledMethod    WHERE parent = '{C}'{O} ORDER BY Name
SELECT Name, Type, Collection, Origin
  FROM %Dictionary.CompiledProperty  WHERE parent = '{C}'{O} ORDER BY Name
SELECT Name, Type, Origin
  FROM %Dictionary.CompiledParameter WHERE parent = '{C}'{O} ORDER BY Name
SELECT Name FROM %Dictionary.CompiledXData WHERE parent = '{C}' ORDER BY Name
```
`{O}` = ` AND Origin = '{C}'` when `include_inherited` is false/omitted;
empty string when true. `parent = '{C}'` is exact id equality — no
over-match risk. kind=all runs all four. XData rows are the class's own
declarations (no Origin filter, no origin field in the entry).
CompilerGenerated/Internal members ARE returned; `private` is NOT exposed.

### class_dependencies

depends_on (always 1 hop — raw dictionary field, no walk):
```sql
SELECT DependsOn FROM %Dictionary.ClassDefinition WHERE Name = '{C}'
```
Client: split raw text on `,`, trim, drop empties, strip a trailing
`(...)` member suffix, classify each entry: passes `valid_class_name()`
and contains `.` → `"class"`; passes `valid_identifier()` → `"routine"`;
else `"other"`. Empty field (probe: common) → empty array, not an error.

used_by (impact query; `depth` applies here):
```sql
SELECT Name, DependsOn, Super FROM %Dictionary.ClassDefinition
 WHERE DependsOn [ '{F}' OR Super [ '{F}'
 ORDER BY Name
```
BFS exactly like subclasses (same frontier re-validation, same
substring→exact client verification by re-splitting both fields; a
`Super` edge counts as a dependency — a subclass depends on its
superclass even when DependsOn omits it, which probe shows is frequent).
Default depth 1. Same 256-query budget. `truncated` = leftover frontier,
budget hit, or `has_more` on any query.

All graph queries run via `sql::query(client, ns, sql, settings.sql_max_rows)`
(1000 default).


## §4 CLI grammar

New `Commands` variants in `src/cli/mod.rs` (pattern: `Sql { query, max_rows }`
at :46), dispatched in `src/main.rs` — BOTH wildcard arms get the new cases
(constitution: `Commands::Mcp` handled above; add `Commands::Search`/
`Commands::Graph` next to the `Commands::Sql` arm which converts args →
`tools::…` directly).

```
rism search <PATTERN>
    [--documents <MASK>[,MASK...]]   (repeatable or comma list; default
                                      "*.cls,*.mac,*.int")
    [--no-regex]                     (default regex ON; the flag flips
                                      regex=Some(false); takes no values)
    [--system]                       -> include_system=true
    [--generated]                    -> include_generated=true
    [--case-sensitive]               -> case_sensitive=true
    [--whole-word]                   -> whole_word=true
    [--max <N>]                      -> max_results=Some(N)
    [--namespace <NS>]               (global flag already exists; use it)

rism graph subclasses <CLASS>  [--depth <N>] [--namespace <NS>]
rism graph members    <CLASS>  [--kind <all|methods|properties|parameters|xdata>]
                               [--include-inherited] [--namespace <NS>]
rism graph deps       <CLASS>  [--direction <depends-on|used-by|both>]
                               [--depth <N>] [--namespace <NS>]
```
`Graph` is a `#[command(subcommand)] GraphCommands` enum (Doc/Test/Debug
pattern); `Search` is a flat variant. Defaults map 1:1 to §1 defaults
(omit → `None` → tool resolves via settings). `--kind`/`--direction` use
`value_enum` (clap snake_case values; `depends-on`/`used-by` accept the
kebab spellings clap derives from the variant names — freeze:
`depends_on`, `used_by`, `both`, `all`, `methods`, `properties`,
`parameters`, `xdata` as the literal accepted strings).

Output: same `--format table|json` global parity as every other arm —
`table` renders search as `doc | member/line | text` rows (one row per
match, `truncated` and tally echoed after the table as a footer line) and
graph as one row per entry; `json` prints the §1 structured value with
`serde_json::to_string_pretty` (Sql/Doc arm convention — coder follows
`src/cli/render.rs` helpers; no new rendering framework).

## §5 Settings

`src/settings.rs` adds two fields (`#[serde(default)]` block, env tier
follows the existing `RISM_*` if-let chain):

| field | type | default | config.toml key | env |
|---|---|---|---|---|
| `search_max_results` | `u32` | 200 | `search_max_results` (root, like `sql_max_rows`) | `RISM_SEARCH_MAX_RESULTS` |
| `graph_depth` | `u32` | 10 | `graph_depth` | `RISM_GRAPH_DEPTH` |

Precedence unchanged: CLI flag > env > config.toml > default. Env parse
behavior mirrors the existing numeric env handlers in
`Settings::load_with_env` (`RISM_IRIS_API_VERSION` /
`RISM_TERMINAL_MAX_OUTPUT_CHARS` — coder: copy their failure path exactly;
do not invent new semantics). `search_max_results > 200` is allowed on the wire (server
caps; §2). Env names for the CLI flags themselves ride the existing
global `--namespace`/`RISM_IRIS_NAMESPACE` — no new ones.

## §5 Settings

(to be filled)

## §6 Bench protocol

(to be filled)

## §7 Test matrix

(to be filled)
