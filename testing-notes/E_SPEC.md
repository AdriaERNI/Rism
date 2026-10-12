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
children only, default `settings.graph_depth` = 10, values > 64 clamp to 64
(0 → error `Config("depth must be >= 1")`). Uses the transitive table
function for the flat name list + a bounded `Super IN (...)` walk for
per-level assignment (§3). No `include_self` — result excludes `class`
itself.

### class_members — `src/tools/graph.rs::ClassMembersArgs`

`class: String` (gate), `namespace: Option<String>`,
`kind: Option<MembersKind>` where
`pub enum MembersKind { All, Methods, Properties, Parameters, XData }`
(serde rename_all = "snake_case"; default `All` — decided: XData INCLUDED in
the enum because `%Dictionary.CompiledXData` is queryable (probe: returns
parent/Name rows), even though its `Data` stream is not),
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
      "matches": [ {"member": "Set", "attr": null, "line": null, "text": "Set"},
                   {"member": null, "attr": null, "line": 5, "text": "Quit x"} ] } ] }
```
`line` is ALWAYS `u32|null` after normalization (§2); `attr` appears only
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
  "parameters": [ {"name": "CAPTION", "default": "\"\"", "type": "", "origin": "X"} ],
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
   reached.`] or `Not found.`); `null` when no such line. Console line 1 is
   the echo of the search (`Searching for '...' in '...'`) — kept raw in
   the structured output? NO — frozen: console is NOT echoed beyond the
   tally; the CLI/table renderer uses tally+counts only.
6. Counts `doc_count`/`match_count` are computed client-side from the
   parsed array (server tally text is display-only — its numbers can
   exceed/undercount vs the returned array at the cap).

Wiremock pins (§7): URL query order-insensitive matching via
`query_param` predicates; test asserts encoded forms for `query="a$b.*"`
(`a%24b.%2A%2A`-safe — assert `%24` present, i.e. encode ran), absent
`case`/`word` params when options are None, `line` int AND string parsing,
`{}`-object empty result, truncation flag from console marker.

## §3 Graph SQL (frozen statements)

(to be filled)

## §4 CLI grammar

(to be filled)

## §5 Settings

(to be filled)

## §6 Bench protocol

(to be filled)

## §7 Test matrix

(to be filled)
