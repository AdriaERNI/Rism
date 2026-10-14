# F_PLAN — adversarial review plan for PR #22 (task rism-pr22-review-f)

Planner: specifier. Surface reviewed to write this: `git diff --stat 75272e8..ffd3580`
(47 files), plus reads of `src/tools/search.rs`, `src/tools/graph.rs`,
`src/tools/names.rs`, `src/iris/search.rs`, `src/iris/http.rs::api_url`,
`src/cli/mod.rs`, `docs/code-search.md`, `docs/mcp-tools.md`,
`bench/run_bench.py`, `bench/drift_port.py`, `installer/static/analyze-docs-drift.ps1`,
`testing-notes/E_SPEC.md §2-§4`, `E_ATTACKS.md`. No deep review here — coder/refactorer
execute rows and file findings (F-nn).

Conventions:
- **Row = need → probe-command shape → ORACLE (independent completeness cross-check,
  never the tool grading itself) → blind-spot hypothesis (CONFIRM/REFUTE live) → owner.**
- Live server: docker `rism-iris` (IRIS 2025.3 b226U, Atelier v8), USER + %SYS.
  Every doc-creating row uses the `FProbe_` prefix, deletes its docs, and asserts
  `doc list --filter FProbe_%` empty before handoff (constitution: unique prefixes, clean up).
- Binary worktrees: coder runs from `.worktrees/coder` (HEAD ffd3580b58); its
  `target/release/rism` is the PR binary (the main worktree binary is stale pre-#22 —
  it has no `search` subcommand; verified this session).
- Oracles: (O1) `rism sql` against `%Dictionary.*` (server truth), (O2) export docs
  (`rism doc get` / project export) + `rg` over the export (text truth), (O3)
  `_SubclassOf`/`_SuperclassOf` table functions (transitive truth for graph walks).

## §1 Discovery matrix (M rows — the user's question)

33 rows. Every row is executed live by coder (or refactorer where noted) and the
result (found / blind / partial + which mode/params make it findable) lands in
F_MATRIX.md. Probe shapes assume `.worktrees/coder/target/release/rism` (PR head
ffd3580b58 — verified this session: main-tree binary is pre-#22 and has NO
`search` subcommand). Every fixture-creating row uses `FProbe_*` doc names and
deletes them before handoff.

| id | need (developer/agent phrasing) | probe command shape | ORACLE (independent completeness check) | blind-spot hypothesis (confirm/REFUTE live) | owner |
|---|---|---|---|---|---|
| M-01 | full member list of class C (methods/props/params/XData, own) | `rism graph members C` | `rism sql "SELECT Name FROM %Dictionary.Compiled{Method,Property,Parameter,XData} WHERE parent='C'"` — set-diff | CompilerGenerated/Internal rows pollute the list (E_SPEC §3:289 admits it); docs never warn | coder |
| M-02 | same incl. inherited | `rism graph members C --include-inherited` | drop the `Origin=` filter in the O1 queries | inherited rows' `origin` correct only if Compiled* Origin semantics hold for multi-super | coder |
| M-03 | all callers of method X across namespace | regex `search X` (definitions) AND `search X --no-regex` (text) | export all *.cls/*.mac/*.int docs (`doc list`+`doc get`) → `rg` for `##class(...).X` and bare `X` occurrences, doc set | BLIND to call sites in regex mode (metadata only) and no xref (Tier-3 deferred); literal mode finds them but the 200-cap silently hides docs when X common → `truncated` must be true there | coder |
| M-04 | where %ZLIB is used | `search '%ZLIB'` both modes; `--system`; `--namespace %SYS` | `rism sql` on `%Dictionary.ClassDefinition WHERE Name='%ZLIB'` in %SYS + rg of a %SYS export of referencing docs | PRE-PROBED TODAY (spec worktree): USER namespace, *.cls, both modes → `Not found.` — system classes/routines are not docs in USER; %SYS + sys flag may still miss `$system.ZLIB` call sites (string, not class) | coder |
| M-05 | production config referencing class X (Ens.Config.Production XData `<Item` XML) | `search X --generated --system` masks *.cls | `rism sql "SELECT Name,ClassName FROM Ens.Config.Item"` (+ `doc get Ens.Config.Production.cls` and rg its XData text) | BLIND hypothesis: XData *content* lives in a stream the search engine may not scan → production Job/Item class refs invisible to both modes; class_members lists XData NAMES only. Workaround row if confirmed: doc get on the config class | coder |
| M-06 | find trigger code referencing X | `search X --no-regex` (trigger bodies live in .cls source); `graph members` on a class with a trigger | `rism sql "SELECT parent,Name FROM %Dictionary.CompiledTrigger"` (+ rg of export for `Trigger .*X`) | members tool has NO trigger bucket (4 tables only) → trigger existence invisible in "full member list"; body findable only via literal mode | coder |
| M-07 | SQL query definitions in a class (`Query Q As SQL`) | `graph members` (expect: no queries bucket) + `search 'As SQL' --no-regex` | `rism sql "SELECT Name FROM %Dictionary.CompiledQuery WHERE parent='C'"` set-diff vs what tools return | class_members silently omits Queries/Triggers/Indices/Procedures while docs say "all four member tables" — a dev trusting `kind=all` misses queries → HIGH if docs imply completeness | coder |
| M-08 | all SqlProcs in namespace | per-class `graph members --kind methods` (sql_proc flag) — no namespace-wide probe exists | `rism sql "SELECT parent,Name FROM %Dictionary.CompiledMethod WHERE SqlProc=1"` count | BLIND by design: no namespace-wide member query surfaced; docs must say so | coder |
| M-09 | the compiled SQL text of a query | `search 'SELECT' --no-regex` (source text) | export + rg the `##sql`/query body; %Dictionary.CompiledQuery.StatementText not readable | compiled (generated-class) SQL hidden behind gen=0; source-mode SQL findable — confirm both | coder |
| M-10 | where is macro $$$MYMAC defined (`$DEF`) | default masks → expect miss; then `search 'MYMAC' --documents '*.inc' --no-regex` | export *.inc set (rg `\$DEF.*MYMAC` over `doc get` of each) | BLIND by DEFAULT (masks cls/mac/int, src/tools/search.rs:13); does `action/search` even accept `*.inc`? probe | coder |
| M-11 | who INCLUDEs routine/class R / INCLUDE dependencies | `search 'Include { ' --no-regex`, `search '#include'`; routine header `INCLUDE` | export rg for include directives (`Include` / `#Include` statements) per doc vs tool doc set | no include edge anywhere in the graph tools (DependsOn/Super only) → "what breaks if I edit this .inc" is BLIND; confirm DependsOn doesn't carry includes either | coder |
| M-12 | a routine label / label-offset hit — what does the agent see for position? | `search 'SomeLabel' --documents '*.mac'`; craft `FProbe_Lab.mac` with labels; regex hit on class attr reporting `Description+32` | `doc get` the same file + `rg -n` true file line | label tokens land in `label` with `line:null` (U6) → agent loses position for `.cls` attribute hits; docs' "treat member+text as position" is the honest workaround — confirm shape live | coder |
| M-13 | find code in uncompiled drafts | `doc put FProbe_Draft.cls` (no compile) → `search` regex vs `search --no-regex` | the put file itself + `doc get` | bench preflight claims literal sees uncompiled, metadata does not → identifier search in default regex mode MISSES drafts — confirm + docs line 22 claim | coder |
| M-14 | code in the system namespace | `search ... --namespace %SYS [--system]`; `graph members %RegisteredObject --namespace %SYS` | `rism --namespace %SYS sql` on %Dictionary | namespace is NOT validated (only class names are — names.rs); `%SYS`→`%25SYS` encoding via api_url (http.rs:109) — confirm works AND hostile ns values can't break the path (see H-10..13) | coder |
| M-15 | generated classes (web-service stubs, Ens.Config.*) | `search X` default → miss? → `--generated` | `rism sql "SELECT Name FROM %Dictionary.ClassDefinition WHERE GeneratedBy IS NOT..."` / name-pattern census | default gen=0 hides the generated proxy that actually references your class → confirm which flag combo finds it | coder |
| M-16 | CSP page calling proc P | `search 'P' --documents '*.csp'` ; also default masks → expect miss | export *.csp docs (doc list filter) + rg | *.csp excluded by default masks; does action/search scan them at all when masked? probe → if not, web tier is BLIND (HIGH for docs honesty) | coder |
| M-17 | code that touches global ^MyData | `search '\^MyData'` (regex escape) + `--no-regex` | export rg `\^MyData` over cls/mac/int | code refs findable (bench Q8 `^Trigger` precedent); global DATA contents — impossible server-side via this surface → honest-"no" row | coder |
| M-18 | content inside %Stream.GlobalCharacter fields / binary docs | (no probe can reach it — attempt `--no-regex` for a stream payload marker anyway) | export doc, inspect | impossible via action/search — matrix records "no workaround tool-side"; refactorer wordings | refactorer |
| M-19 | "every class extending %RegisteredObject" (huge result) | `find_subclasses %RegisteredObject --depth 1` (and MCP); `search` regex for it | `rism sql "SELECT COUNT(*) FROM %Dictionary.ClassDefinition WHERE Super [ '%RegisteredObject'"` | thousands of children: has_more→truncated via sql_max_rows=1000? system children hidden by …? (graph SQL runs in USER unless --namespace — %Dictionary in USER sees system class DEFINITIONS? confirm) | coder |
| M-20 | subclass tree correctness: diamond + multi-super + depth clamp | FProbe_A←B,C←D diamond fixtures (mirror bench GRAPH_CLASSES with FProbe_ names) + `--depth 64`, `--depth 0` | O3: `%Dictionary.ClassDefinition_SubclassOf('FProbe_A')` transitive set-diff vs BFS union at depth 64 | `[` substring + exact re-split guard (graph.rs:637) fails on names that are prefixes (`BenchGraph.Mi` case passes in bench) — re-verify with FProbe_Mi/FProbe_Mid | coder |
| M-21 | who uses class X (impact) via body references, no inheritance | `graph deps X --direction used_by` | export rg `##class\(X\)` doc set − used_by set | E_BUILD D3 says DependsOn is often EMPTY → used_by finds only Super edges + whatever DependsOn happens to hold; body-only users BLIND (this is the flagship blind spot) | coder |
| M-22 | who calls routine R ("delete-safe?" for a routine) | `search 'R' --documents '*.mac,*.int,*.cls' --no-regex` (substring noise) | export rg for `^R` label references and `R^` / `$ZOBJ` patterns (word-boundary variants) | graph tools are class-centric: routine→callers edge only via literal search noise → partial row | coder |
| M-23 | all properties of a given Type (e.g. %Stream.GlobalCharacter) | `search '%Stream.GlobalCharacter'` (docs claim metadata scan covers attribute VALUES, code-search.md:21) | `rism sql "SELECT parent,Name FROM %Dictionary.CompiledProperty WHERE Type='...'"` | attribute-value scan claimed but never probed for Type across a real namespace; gen/sys defaults could hide most users | coder |
| M-24 | case-insensitive search for 'error' vs 'Error' | `search error` (regex) / `search error --no-regex --…` — `case_sensitive:false` explicit via MCP | export `rg -i` vs `rg` doc counts | docs line 36: case flag "only takes effect in literal mode" — confirm regex mode IGNORES the param (server), else docs lie | coder |
| M-25 | whole-word 'Set' | `--whole-word` with `--no-regex`; same without | export `rg -w '\bSet\b'` vs plain | word flag literal-only claim; regex-mode `\bSet\b` equivalence check | coder |
| M-26 | unicode query text (é, 中文 comment planted in FProbe_ doc) | `search '中文字' --no-regex` | `doc get` shows the line; rg over saved file | urlencoding::encode is UTF-8-yes; Atelier server may expect UTF-8 query param — confirm end-to-end no mojibake | refactorer |
| M-27 | query containing `+`, `&`, `=`, `%` (URL metachars) | plant `FProbe_U.url` literal `a+b&c=d%e` in a doc; `search 'a+b&c=d%e' --no-regex` | the planted doc | qs are pre-encoded then concatenated into URL string passed to reqwest.get(&str) — double-encode (`%`→`%25`) or split-on-`&` bug? (iris/search.rs:50-86) | refactorer |
| M-28 | every class having a method named OnBuild (namespace-wide member search) | `search 'OnBuild'` regex default; then `--max 5000` | `rism sql "SELECT parent FROM %Dictionary.CompiledMethod WHERE Name='OnBuild'"` count | docs say metadata scan finds member NAMES — verify count parity vs dictionary; truncation flag at 200 | coder |
| M-29 | max edges | `--max 0`, `--max 1`, `--max -1` (CLI clap error expected), MCP `max_results: 4294967295` | doc-level: 0 → "Not found."+truncated=false (docs:39); huge → server behavior recorded | u32::MAX on the wire — server overflow or 400? issue #17 "hard cap 200" is stale for 2025.3 (D5) — both doors consistent? | coder |
| M-30 | default namespace from config, not CLI | `rism search x` with no --namespace vs config.toml iris_namespace | `rism info` echo + env `RISM_IRIS_NAMESPACE` override precedence | settings drift: `RISM_SEARCH_MAX_RESULTS` garbage keeps default (settings.rs:202) — confirm precedence chain documented in configuration.md | refactorer |
| M-31 | table-function vs BFS equivalence | `find_subclasses X --depth 64` | `SELECT Name FROM %Dictionary.ClassDefinition_SubclassOf('X')` exact set | budget 256 frontier queries on wide trees → truncated=true before completeness; which depth does USER hit it at? | coder |
| M-32 | exotic document masks | `--documents ''` ; `--documents '*.zzz'` ; `--documents '*.csp,*.cls'`; MCP `documents:["*.cls',';--"]` | `doc list` names census | `documents=[]` falls back to defaults (search.rs:98 `.filter(!is_empty)`); a mask containing `,` breaks the join→encoding (iris/search.rs:57 joins THEN encodes — commas survive as separators: mask `"a,b"` smuggles a second mask — confirm) | coder |
| M-33 | production items whose ClassName points at class X | `search 'X' --generated` ; `graph deps X` | `rism sql "SELECT Name FROM Ens.Config.Item WHERE ClassName='X'"` (in a production-enabled NS; USER has Ens* — probe availability first) | config rows are SQL table data, not code → findable ONLY via sql_exec, invisible to both new tools unless XData streams are scanned (ties to M-05) | coder |

## §2 Hostile-input cases (H rows)

Injection payloads reused EXACTLY from E_ATTACKS.md §A (the 10-payload set) plus new
namespace/query vectors E_ATTACKS never covered (its own table says "Payloads × tools ×
doors: 45/45" for CLASS NAMES ONLY — namespace and query hostility are planner-identified
gaps). Every refusal row must show **zero HTTP requests** — oracle: dead-port variant
(`RISM_IRIS_BASE_URL=http://127.0.0.1:1`) must print the SAME guard error, not a connect
error (pattern: bench/attack_refusal.py), plus wiremock U8–U10 counts.

| id | surface | exact input | expected | oracle / note | owner |
|---|---|---|---|---|---|
| H-01 | graph class names (all three tool surfaces) | `Bobby'; DROP TABLE x;--` | Config guard error, 0 HTTP | dead-port repro proves pre-HTTP (names.rs:8 allowlist) | coder |
| H-02 | graph class names | `a' OR '1'='1` | refused | same | coder |
| H-03 | graph class names | the E_ATTACKS 7th payload: "Robert" + space + ";" + space + ";" + space + backtick | refused | same | coder |
| H-04 | graph class names | `Foo%27` , `%27%20OR` | refused (guard is pre-decode: `%27` chars are not in allowlist) | confirm no double-decode path in api_url | coder |
| H-05 | graph class names | `../../etc/passwd` , empty , `1abc` , `a b` , `x'y` | refused (names.rs first-char rule) | U15 unit + live | coder |
| H-06 | graph % prefixed | `%RegisteredObject'` (trailing quote after a valid % start) | refused | guard boundary `%`+valid then invalid char | coder |
| H-07 | search query | the metachar storm `.*+?[](){}`, caret, pipe, backslash, `^\$\` | honest server error `#8311 Syntax error in regexp pattern` via BOTH doors, no crash | E_ATTACKS §A did regex only; repeat + MCP door parity | coder |
| H-08 | search query | `a&b`, `a+b`, `a b`, `a=b`, `query=1&max=1` (param-splitting shapes) | treated as data (encoded); results match a planted `FProbe_Enc` doc OR honest Not found — NEVER a second param effect (e.g. max change) | ORACLE: plant literal `query=1&max=1` text; a search for it that returns the doc with max=1 default 200-count proves no injection into qs (iris/search.rs:50-86 pre-encodes values — verify `%26` on wire via tcpdump-free proxy check: `--url` to localhost nc, or assert result parity vs plain query of same string) | refactorer |
| H-09 | search query | 5000-char query; query with lone `%`; query `%25` | echoed intact / server behavior recorded; no panic | E_ATTACKS row "5000-char accepted" — MCP door re-check | coder |
| H-10 | namespace param (search+graph) — NEW GAP | `--namespace "A' OR 1=1--"`, `--namespace ..%2f..`, `--namespace "USER&x=1"`, `--namespace ''` | **hypothesis: namespaces are NEVER validated** (tools/search.rs:91 clones raw; ns_of graph.rs:23) → what reaches the wire? Path traversal into another API route or 400 from server? Any crash/panic = CRITICAL | wire-shape oracle: dead-port run shows the URL in the connect error? else run under `RISM_IRIS_BASE_URL` pointing at a local `python3 -m http.server` that LOGS the request path — that log IS the ground truth of what the client sends | refactorer |
| H-11 | namespace | `%` alone, `%%`, `%41` (pre-encoded 'A'), `a%` | `%`→`%25` replace (http.rs:109) on `%41` gives `%2541` — double-encode anomaly: server 400? acceptable? record | same logging oracle | refactorer |
| H-12 | namespace unicode | `--namespace USER25` vs `%SYS%` | behavior recorded; no panic | | coder |
| H-13 | namespace traversal | `--namespace ../../` , `--namespace "USER/%2e%2e"` | must NOT reach another endpoint (e.g. act as GET /action/search of root) — 400/404 = ok, 200 = CRITICAL | logging-proxy oracle | refactorer |
| H-14 | MCP door | all H-01..H-05 + H-09 payloads through `tools/call` JSON | identical error class (not clap) | mcp_probe pattern; both doors same message | coder |
| H-15 | args serde | `{"query":"x","wild":true}` , `{"class":"X","kind":"bogus"}`, `{"class":"X","direction":"all"}`, missing required | validation errors, server stays alive, tools/list still 29 | U1/U2 pin units; live MCP session re-check | coder |
| H-16 | depth/max overflow | MCP `depth: 0` (err), `depth: 4294967295` (clamp 64), `max_results: 0`, `max_results: 4294967295`, negative via CLI `--max -1` (clap u32 reject) | per graph.rs:652-657 + settings; no panic, no infinite budget | CLI vs MCP same clamp | coder |
| H-17 | kind/direction enums wire | MCP `kind:"x_data"`, `kind:"XData"` | refused (frozen lowercase `xdata`, graph.rs:182) | wire ledger check | coder |
| H-18 | settings env | `RISM_SEARCH_MAX_RESULTS=abc` / `-5` / `999999999999` | keeps default 200 (settings.rs:202 parse_env_u32) — no panic, no 0 | configuration.md claim cross-check | refactorer |

**Namespace guard asymmetry is this section's headline**: class names are allowlisted
"before any HTTP request" and the docs repeat it (docs/code-search.md:62-63), while
`namespace` flows raw into the URL path — if H-10..H-13 show more than a benign 400,
that's CRITICAL; if benign, it's still a docs-HIGH (guard described as covering "every
interpolation site" but the URL path is an interpolation site too: src/iris/search.rs:49).

## §3 CLI-parity cases (P rows)

Rule: for each case, run the CLI (`rism --format json …`) and the MCP door
(`tools/call` over stdio with equivalent args) against the same live state and
**diff the JSON payloads field-by-field** (ignore the CLI's added `_ms`-style
instrumentation if any, and the table renderer — `--format json` only). Any
semantic difference (field present/absent, value, ordering that changes meaning)
is a finding. `rism mcp` stdio harness: existing probe pattern in
`packaging/mcp_probe.sh` (must still report 29 tools).

| id | case | CLI | MCP | parity oracle | owner |
|---|---|---|---|---|---|
| P-01 | search happy | `search Foo --no-regex --max 5` | `search_code {query:"Foo",regex:false,max_results:5}` | deep-equal on namespace/query/documents/server_tally/truncated/doc_count/match_count/results | coder |
| P-02 | search default flags (regex omitted) | `search Foo` | `search_code {query:"Foo"}` | equal AND wire shows `regex` param ABSENT (not 1) per tools/search.rs:106 None-omits rule — compare via logging proxy | refactorer |
| P-03 | search flags on/off | `--system --generated --case-sensitive --whole-word` | all four true | sys=1&gen=1&case=1&word=1 identical results; case/word ONLY sent when set (search.rs:109-110) vs CLI bool=false always-sends? **asymmetry hypothesis: CLI `--no-regex` is a bool flag → regex=Some(false) ok, but CLI cannot express case_sensitive=false while MCP can omit it → document flag-expressiveness diff** | coder |
| P-04 | documents repeatable | `--documents '*.cls' --documents '*.mac'` (value_delimiter ',') | `documents:["*.cls","*.mac"]` | same joined wire `*.cls,*.mac` (encoded) | coder |
| P-05 | namespace override | `--namespace %SYS search ...` | `{namespace:"%SYS"}` | identical incl. `%25SYS` wire path | coder |
| P-06 | empty result shapes | `search DefinitelyNoSuchToken_XYZ` | same | both return `{}`-or-`[]` result normalized to zero-count object, `server_tally:"Not found."`, exit code 0 CLI | coder |
| P-07 | error shape hostile | `search` empty query `""` | `{query:""}` | both doors surface HTTP 400 with the same message text (docs:37 claims error surfaced, not "everything") | coder |
| P-08 | graph subclasses | `graph subclasses X --depth 2` | `find_subclasses {class:X,depth:2}` | deep-equal; `super` key spelled `super` not `parent` in JSON on BOTH doors (serde rename graph.rs:47 — CLI renderer must not rename it back) | coder |
| P-09 | graph members kinds | `graph members X --kind xdata` etc ×5 | kind each value | counts vector equal; `kind` echo string equal | coder |
| P-10 | graph deps directions | `graph deps X --direction used_by --depth 3` | same | equal; truncated bit equal | coder |
| P-11 | depth default source | `graph subclasses X` (no --depth) with `RISM_GRAPH_DEPTH=2` env | MCP same env | both honor settings default 2 (not 10) — env applies to both doors identically | refactorer |
| P-12 | --format json vs table exit codes | hostile class name CLI | hostile MCP | CLI exit≠0 + stderr guard msg; MCP is_tool_error + same msg substring | coder |

## §4 Bench methodology audit (B rows)

Mission: re-run once under `bench_f_` naming (the published markers are frozen
`RISM-BENCH-M-*`; the task prefix rule applies to DOC names coder creates
manually — run_bench.py's own docs are `Bench*`, which cleanup asserts; coder
must NOT rename harness internals just to prefix, but MUST verify cleanup
leaves zero `Bench*` docs after the F re-run). Audit questions:

| id | question | how to check (no tool self-grading) | owner |
|---|---|---|---|
| B-01 | Do the graded metrics match published bench/RESULTS.md? (recall 20/20, precision 20/20 aggregate — note TASK.md says 20/20 but docs/code-search.md:85 says **20/21 = 95%**: the two published surfaces ALREADY disagree — which is right?) | one full re-run; diff every table number vs committed RESULTS.md; reconcile precision 20/20 vs 20/21 discrepancy as a finding (docs-vs-artifact drift) | coder |
| B-02 | Self-proving loop #1: ground_truth.csv is written by the same script that injects markers (inject_markers.main) — is "recall" just measuring the injector? | independent oracle: after load, `rg -l 'RISM-BENCH-M-' bench/.work/export` doc set + marker table (file-level) vs ground_truth.csv rows — must agree before trusting any search number | refactorer |
| B-03 | Self-proving loop #2: precision denominator counts only docs RETURNED by the tool (`other_docs` from tool output, run_bench.py:303) — an empty search scores precision 1/1 vacuously? | recompute per-marker precision from rg ground truth (expected_docs / returned_docs) and compare to the harness number; report if the published metric can't fail | refactorer |
| B-04 | Marker placement bias: D4 says markers ride INSIDE the class body because trailing comments fail the parser — do markers sit in files/lines the scan is guaranteed to read (not in XData/stream/generator code)? | open 4 export files, verify marker is a plain source line; cross-check categories plain/literal/regex actually exercise the 3 claimed scan paths | coder |
| B-05 | Hit rule = "expected doc returned AND marker substring in some match text" — a marker that appears in a DIFFERENT doc (M-190 adjacency, D5) still scores hit for M-19? decoy accounting honest? | recompute hits with a stricter rule (doc AND line) from raw JSON saved during B-01; diff the two scorings | refactorer |
| B-06 | `compiled ok 66/80`-style counts: recall was measured over loaded-but-uncompiled docs too (run_bench.py comment: "recall runs over all loaded docs") — would recall drop if measured only over COMPILED docs? | from RESULTS.md compiled list + ground_truth join, recompute compiled-only recall from B-01's saved JSON (no re-run needed) | coder |
| B-07 | Latency fairness: rism wall ms includes process startup + auth round trips; rg ms is a warm local scan. Is the table's framing honest about the ~22-60× gap AND about auth count (does each CLI call re-login)? | `strace -f -c` alternative: count HTTP requests per CLI call via logging proxy on 1 call; if login+search = 2+ round trips, docs claim "1 HTTP round trip" (run_bench.py:537) is FALSE | refactorer |
| B-08 | Truncation cliff: default capped at 200 but D5 says 2025.3 honours 500 — the "no silent paging" claim relies on truncated flag; confirm `--max 500` truncated=false AND match_count=500 exactly (not 499/501 → off-by-one in the client-side `match_count >= max` rule, iris/search.rs:149-151) | from B-01 run's phase-d JSON | coder |
| B-09 | Client truncation heuristic bug hunt: `match_count >= requested_max` marks truncated even when the server returned EVERYTHING at exactly the cap — false positive `truncated=true` when total==max? | craft FProbe doc set with exactly N==max hits; oracle = rg count on export; document as MEDIUM finding if flag lies in either direction | coder |
| B-10 | Cleanup honesty: `doc list --filter Bench%` after run — but preflight docs BenchProbe.* deleted mid-run; SIGINT path tested? | run cleanup unit: kill -INT a fresh partial run, assert 0 Bench docs remain via `doc list` | coder |
| B-11 | graph_truth.csv "hand-verified" — 11 checks re-derived independently from %Dictionary (O3 table functions) not from the tool | `sql` oracle per bench row during B-01 window (docs still loaded) | coder |

## §5 drift_port.py vs analyze-docs-drift.ps1 semantic diff (D rows)

Method (refactorer): line-by-line pairing of the four checks (already done once by the
specifier — the suspected divergences below are **hypotheses to confirm by mutation
test**, not verdicts). For each: (a) read both sources side by side, (b) where they
could disagree, construct a tiny repo-state mutation that distinguishes them (add a
temp untracked .md with a wrong "ninety-nine tools" claim (any total that mismatches
the code count), etc. — mutations only in a scratch
COPY of the tree, never committed; the real tree stays byte-clean), (c) run BOTH gates
on the real tree at ffd3580 and require identical exit code + same ok/FAIL set.

| id | assertion pair | python (bench/drift_port.py) | powershell (installer/static/analyze-docs-drift.ps1) | hypothesis to confirm | owner |
|---|---|---|---|---|---|
| D-01 | which *.md files are scanned | `ROOT.rglob("*.md")` filtered to `git ls-files` (lines 16-18, 51-53) | `Get-ChildItem -Recurse -Include *.md -Exclude target` (line 30) — NO tracked filter; `-Exclude target` matches the leaf name only, so it excludes NOTHING under target/ | on a dev machine (or any checkout with untracked md — swarmforge/, testing-notes/, target/ docs) PS FAILS where py passes: the gitignore claim in py's comment only holds on a pristine CI checkout. On OUR worktrees (target/, swarmforge/ exist) both must be run and the delta explained | refactorer |
| D-02 | attr/fn pairing loss | explicit `bad()` when `len(tools)!=len(attr_idx)` (py:43-44) | a `#[tool(` after the last `async fn` yields `$k=$null` → silently adds a null tool, `-Unique` may drop it → count drift with NO error (ps:20-24) | mutation: append stray `#[tool(broken)]` at EOF of a COPY of src/mcp/mod.rs → py reports FAIL, ps exits 0? confirm | refactorer |
| D-03 | tool-count claim regex | identical `(?<!\()\b(\d+)\s+tools\b` | identical | equal by inspection — diff byte-for-byte | refactorer |
| D-04 | tool-name documentation check | `` f"`{short}`" in doc `` | `-notmatch "``$tool``"` — regex match: a tool name containing regex metachars (none today: all snake_case) is safe; equivalent today | confirm no current name needs escaping; note as latent only | refactorer |
| D-05 | config-path promise check | same three regexes (BaseDirs block, %APPDATA%, .config path, serde skip) | same | equal by inspection; run both, diff output sets | refactorer |
| D-06 | semver check | `re.search MULTILINE` | `Select-String` | equal; both exit-code 0/1 — verify exit code parity with `$LASTEXITCODE` / `$?` | refactorer |
| D-07 | process hazard for THIS mission | both scan tracked *.md — our committed F_* docs with a legit "29 tools" claim are fine, but ANY digit followed by the lowercase word "tools" where the digit ≠ the code count (in findings prose) breaks the CI gate we're verifying. Writing rule for both lanes: never write `<digit> tools` unless the digit IS the tool total — say "the four new tools" spelled out. | — | **PROVEN live this session**: this plan's own commit initially failed `drift_port.py` with 3 FAIL rows before rewording (phrasings that matched the gate's claim regex with wrong totals) — coder+refactorer: run `python3 bench/drift_port.py` (must exit 0) before EVERY docs-lane commit | both |

## §6 Docs-vs-truth claims checklist (DC rows)

Each claim: quote → probe that lands it in the M/H/P/B matrix (no re-invention) → verdict
(ok / misleading / wrong) in F_FINDINGS.md.

| id | doc claim (file:line) | re-check via | owner |
|---|---|---|---|
| DC-01 | "regex scans class METADATA; literal scans raw SOURCE" (docs/code-search.md:16-22) | M-13, M-28, E_ATTACKS row regex0-vs-regex1 | coder |
| DC-02 | "uncompiled docs included" (code-search.md:22) | M-13 oracle | coder |
| DC-03 | "case_sensitive/whole_word only take effect in literal mode" (:36) | M-24/M-25 with explicit `case:false` wire check | coder |
| DC-04 | "empty query rejected server-side (HTTP 400)" (:37) | P-07 both doors | coder |
| DC-05 | "max=0 → Not found., truncated=false" (:39) | M-29 | coder |
| DC-06 | "`truncated` reflects the server's Maximum-search-limit notice" (:47-48) | B-09 — code ORs a client heuristic `match_count>=max` (iris/search.rs:149-151) the docs never mention → suspected **misleading** | refactorer |
| DC-07 | "class names allowlisted before any request; unsafe names = zero HTTP calls" (:62-63, mcp-tools.md:217) | H-01..06 (true for classes) + §2 headline: namespace path NOT covered by the sentence — docs-omission finding if H-10 benign | refactorer |
| DC-08 | "200 cap is the RISM CLIENT default, not a server hard cap" (:45-47 vs bench D5) | M-29 `--max 5000` + issue #17 text quoted in settings.rs:71-73 comment ("older builds cap at 200") — coherent? | coder |
| DC-09 | bench headline table (:84-91): recall 20/20, precision **20/21=95%**, graph 11/11, cliff "default caps at 200" | B-01 (and the TASK.md-vs-docs 20/20 vs 20/21 disagreement) | coder |
| DC-10 | "Tier 3 ruled out: no server xref machinery" (:67-68) | M-03/M-21/M-22 — blind spots must all trace to this honest deferral, else docs understate | refactorer |
| DC-11 | docs/mcp-tools.md param lists for the four new tools vs LIVE `tools/list` inputSchema (required, defaults text, enum spellings `xdata`,`used_by`) | dump `tools/list` over stdio once; jq-diff each inputSchema vs the doc table | coder |
| DC-12 | docs/cli-reference.md `rism search`/`rism graph` flags vs `rism search --help` / `rism graph --help` verbatim | diff help text word lists vs doc | refactorer |
| DC-13 | docs/configuration.md `search_max_results=200`, `graph_depth=10` defaults + env names vs settings.rs:105-106, 202-210 | grep pair + M-30 live precedence | refactorer |
| DC-14 | "mcp_probe.sh = 29 tools" (TASK claim) | run packaging/mcp_probe.sh live | coder |
| DC-15 | code-search.md "Reproduce: 4-minute run; cleans every loaded doc on all exit paths" | B-10 | coder |

## §7 Lane assignment summary + execution order

Totals: **33 M + 18 H + 12 P + 11 B + 7 D + 15 DC = 96 rows** (>30 required).
Per-lane (machine-counted from the owner column): coder **69**, refactorer **26**,
shared **1** (D-07 is a process rule for both).

- **coder** owns the live-execution matrix rows in §1–§4/§6 as marked per row
  (all of §6 DC-01..05/08/09/11/14/15, §4 B-01/04/06/08/09/10/11, §3 P-01/03-10/12,
  §2 H-01..07/09/12/14..17, §1 M-01..17/19..25/28/29/31..33).
- **refactorer** owns wire-inspection + methodology + drift verdicts as marked
  (§1 M-18/26/27/30, §2 H-08/10/11/13/18, §3 P-02/11, §4 B-02/03/05/07,
  §5 D-01..06, §6 DC-06/07/10/12/13).

Execution order (docker rism-iris is SHARED — live lanes serialize; bench window is
exclusive):
1. **B-01 bench re-run first** (coder, longest, ~4-6 min, owns the server + corpus docs;
   keep `--keep` OFF but save raw JSON of every phase output to bench/.work/f_run/ for
   B-03/B-05/B-06/B-08 recomputation). While it runs, refactorer does pure-static work:
   D-01..07 (no server), DC-12/13, H-18, M-30 (env-matrix, own process only — no writes).
2. coder M-rows (fixtures FProbe_*; each M-row's oracle query batched right after its
   probes; delete fixtures at section end, `doc list --filter FProbe_%` == empty).
3. refactorer wire-proxy rows (H-08/H-10/H-11/H-13, P-02, B-07) — logging proxy on
   localhost needs the server only for upstream, safe to interleave.
4. P-rows (both doors, no fixtures beyond USER reads) + H-14..17 + DC-11/DC-14.
5. Findings synthesis needs every row's live result — 'per docs'/'probably' rows
   are NOT DONE (constitution); any row a lane cannot execute gets
   `BLOCKED: <reason>` in F_MATRIX.md, never a guess.

Highest-value rows if the clock runs short (do first): M-03, M-05, M-21 (flagship blind
spots), H-10 (namespace gap), B-01 (metrics match + precision contradiction),
DC-06 (truncated-flag honesty), M-10/M-16 (.inc/.csp default-mask blind).
