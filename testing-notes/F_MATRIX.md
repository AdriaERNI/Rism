# F_MATRIX — discovery completeness matrix (PR #22 review, task rism-pr22-review-f)

Coder lane. One row per F_PLAN §1 need. **Result = live probe output only**
(found / blind / partial); mode/params column states what makes it findable;
oracle column records the INDEPENDENT cross-check (%Dictionary SQL, export+rg,
table functions) — the tool never grades itself. Workaround column gives the
escape hatch or an honest "server-side impossible".

Binary: `.worktrees/coder/target/release/rism` @ c16f08e (PR head ffd3580 + docs-only).
Server: docker rism-iris, IRIS 2025.3 b226U, Atelier v8, USER + %SYS.
Fixtures: `FProbe_*` docs, deleted per section; `doc list --filter FProbe_%` == empty asserted.
Raw probe evidence: scratch/bench_f/{m_run_a,m_run_b,m_followup,m_micro,h_wire,h_mcp,p_parity,m_run_*}.json,
bench re-run cli_log + RESULTS_f.md (this lane, 2026-10-14 12:29 UTC run).

Legend: FOUND = need satisfied as documented · PARTIAL = satisfied only with
specific flags/workaround · BLIND = not findable via the new surfaces ·
IMPOSSIBLE = no server-side route exists (Tier-3 deferral).

<!-- rows land as executed; BLOCKED: <reason> if a row cannot run live -->

| id | need | probe cmd | result (found/blind) | mode/params that work | oracle (independent) | blind-spot / workaround |
|---|---|---|---|---|---|---|
| M-01 | full member list of class C | `graph members FProbe.Tri` | FOUND | default (own) | O1 set-diff: Compiled{Method,Property,Parameter,XData} WHERE Origin='FProbe.Tri' == tool output exactly ([Sp],[Data],[],[Blk]) | hypothesis REFUTED: CompilerGenerated rows (%BuildIndex…) exist in Compiled* (90+) but the Origin filter removes them — pollution claim in E_SPEC §3 is wrong for own-lists |
| M-02 | same incl. inherited | `graph members FProbe.D --include-inherited` | PARTIAL | `--include-inherited` (46 methods on diamond D) | O1: CompiledMethod WHERE parent='FProbe.D' (no Origin filter) | some inherited rows carry `origin:""` (e.g. `%%OIDIsValid`/`%%OIDSet`) — mcp-tools.md:235 "inherited rows then carry their declaring class in origin" is false for those rows → F-07b LOW |
| M-03 | all callers of method X | `search Own` (regex) vs `search '##class(FProbe.A).Own' --no-regex` | PARTIAL | literal mode `--no-regex` finds the call site (FProbe.Caller.cls, member=Go line=1); regex mode returns ONLY the definition (member-name metadata) | planted caller fixture (doc get); no server xref table exists (Tier-3) | regex mode BLIND to call sites — by design (engine split); cap 200 hides docs silently when X common but `truncated=true` fires (cap_check: server prints "Maximum search limit reached.") |
| M-04 | where %ZLIB is used | `search '%ZLIB'` USER/%SYS; `search ZLIB --namespace %SYS --system --no-regex` | PARTIAL | %SYS + `--system` + literal mode: 15 hits in 6 files (%CSP.UI.Portal.Mirror.*, SYS.Mirror, MIRROR.int) | O1 %SYS: ClassDefinition %ZLIB=0 rows (class lives in %SYS as %Library.ZLIB); USER literal of ZLIB=0 | USER namespace: both modes `Not found.` — system classes are not USER docs (confirms pre-probe). Regex mode in %SYS also misses the class NAME (metadata scan ≠ doc-name index) |
| M-05 | production config referencing X | `search FProbe.Tri --generated --system` | PARTIAL | literal mode finds XData CONTENT inline in the .cls source (`<production><Item ClassName="FProbe.Tri"…` — msample proves stream not needed for source); `graph members --kind xdata` lists names only (Blk) | O1: `Ens.Config.Item` table does NOT exist in USER (SQLCODE -30, no production) — config ROWS are SQL data, only reachable via execute_sql when Ens is enabled | for a real production: ClassName in the deployed config = SQL table, not source → sql_exec is the workaround (source XData blocks ARE searchable) |
| M-06 | trigger code referencing X | `graph members FProbe.Tri`; `search FPROBE-TRIGGER-BODY-7 --no-regex` | PARTIAL | literal finds trigger body (member=T1 attr=Code) | O1: CompiledTrigger parent='FProbe.Tri' → T1 (+3 %OBJ* system rows); tool returns NO trigger bucket (keys = methods/parameters/properties/xdata only) | trigger existence invisible in the "full member list" — F-07 docs gap; workaround: `search 'Trigger' --no-regex` or O1 SQL |
| M-07 | SQL query defs in class | `graph members` + `search 'Query Q1' --no-regex` | PARTIAL | literal finds `Query Q1 ` (member=Q1); metadata finds nothing ("As %SQLQuery" Not found) | O1: CompiledQuery parent='FProbe.Tri' → [Extent, Q1] — tool member list omits BOTH | no queries bucket — docs say "all four member tables" (true of the 4 implemented; devs expecting Queries get silence) → F-07 |
| M-08 | all SqlProcs in ns | `graph members FProbe.Tri --kind methods` (sql_proc flag) | BLIND (by design) | per-class flag only (`Sp` → sql_proc:true correct) | O1: SqlProc=1 census = hundreds across USER (rows sampled) | no namespace-wide member probe exists; must use execute_sql on CompiledMethod — docs state 4 tables are per-class (mcp-tools.md:229) — OK but flag in code-search "Where to go next" absent → F-07 |
| M-09 | compiled SQL text of a query | `search 'SELECT Name FROM %Dictionary.ClassDefinition' --no-regex`; `search FProbe.Tri.queryclass --generated --no-regex` | PARTIAL | SOURCE text found (member=Q1 attr=SqlQuery); generated `FProbe.Tri.1.int` routine appears ONLY with `--generated` (M-05 msample) | O1: CompiledQuery.StatementText column does not exist on 2025.3 (#5540) → SQL route dead; export+rg = source of truth | compiled SQL text = generated code hidden behind gen=0; `--generated` finds the generated routine |
| M-10 | macro `$$$MYMAC` definition | `search FPROBE_MYMAC` default vs `--documents '*.inc' --no-regex` | PARTIAL | `action/search` DOES scan .inc when masked (Ensemble.inc/#define rows found with documents=*.inc) — default masks miss $defs entirely (`Not found.`) | O2: doc get of %assert.inc 404s in USER (inc files are %Library.* classes in %SYS); USER .inc set = Ensemble.inc etc, `#define` lines matched with mask | usage sites ARE findable in default masks (FProbe.Inc.cls `set v = $$$FPROBE_MYMAC(2)`); definition needs `--documents '*.inc'` → F-06 docs gap |
| M-11 | who INCLUDEs R | `search 'Include FProbeMac' --no-regex` (0!) then `search FProbeMac` regex | PARTIAL | the UDL **header** line `Include FProbeMac` is NOT in the literal body index (line 1 of the .cls; `Not found.`) — but METADATA attr `IncludeCode` IS scanned: regex search for the .inc name hits attr=IncludeCode | O1: ClassDefinition.FProbe.Inc DependsOn="" ; `graph deps` shows only %RegisteredObject | no include EDGE in graph tools (DependsOn/Super only); "what breaks if I edit this .inc" → search the .inc NAME (metadata IncludeCode) works — F-08 LOW |
| M-12 | label / label-offset position | `search FProbeStart --documents '*.mac'`; `search FProbe.Tri --no-regex` | FOUND (shape confirmed) | routine labels return numeric `line` (FProbeStart line=2, FProbeSecond line=6 — true file lines vs doc get); .cls attribute hits return `line:null` + member/attr (e.g. member=Storage attr="Default,DataLocation") | O2: doc get FProbeLab.mac line numbers match tool `line` exactly | U6 shape holds: for .cls attr hits the agent position = member+attr+text (docs:33-35 honest) — no `label` token needed for these (label stays null) |
| M-13 | code in uncompiled drafts | `doc put FProbe.Draft.cls` (never compiled) → `search FProbeDraftMethod` (regex) + `search FPROBE-DRAFT-COMMENT-42 [--no-regex]` | FOUND | BOTH modes see the draft: regex finds the identifier (member attribution) AND the /// comment via attr=Description; literal finds the comment | O1: ClassDefinition WHERE Name='FProbe.Draft' → 1 row (indexed at put? NO — row present because %Dictionary keeps definition storage; CompiledMethod absent — `rism sql CompiledMethod parent='FProbe.Draft'`=0) | docs:22 "compiled or not — uncompiled docs included" CONFIRMED for both engines (bench preflight metadata_uncompiled=True reproduced) |
| M-14 | code in %SYS | `search --namespace %SYS --system …`; `graph members %RegisteredObject --namespace %SYS` | PARTIAL | search works: wire path `%25SYS` correct (P-05 proxy), literal+sys finds SYS.Mirror/%CSP.UI code; SQL works (ClassDefinition count 5941) | O1 %SYS: works; **but** `graph members %RegisteredObject` (either ns) → ALL counts 0 — %Dictionary.Compiled* parent is the REAL name `%Library.RegisteredObject` (19 methods own) | %-alias system classes silently return EMPTY member lists — misleading no, not honest no → F-03 |
| M-15 | generated classes | `search queryclass --generated --no-regex` | FOUND (with flag) | default: `Not found.` (gen=0); `--generated` surfaces `.1.int`/generated routines (M-05 sample) | O1: ClassDefinition GeneratedBy<>'' → 6 %Api.*.disp rows in USER (the .spec siblings are the searchable docs) | generated proxies findable only with `--generated` — documented (mcp-tools include_generated default false) |
| M-16 | CSP page calling proc P | `search Show --documents '*.csp' --no-regex` | FOUND (with flag) | `*.csp` masks ARE scanned (2 hits in /csp/user/showsource.csp, line=73/81 with text); DEFAULT masks → miss (docs:34 excludes) | O2: doc list filter '%.csp' = 2 real pages in USER (/csp/user/*); doc get shows same lines | web tier NOT blind — needs explicit `--documents`; docs never mention .csp → F-06 |
| M-17 | code touching ^MyData | `search '\^FProbeData'` (regex) vs `^FProbeData --no-regex` | PARTIAL | literal: 2 hits (FProbe.Caller.cls + FProbeLab.mac) = exactly the planted refs; regex `\^X`: `Not found.` — regex mode is METADATA scan, never body text | O2: fixture planted twice; rg of doc get agrees | global DATA content: IMPOSSIBLE via action/search (source docs only — honest no, F_MATRIX §Tier-3) |
| M-18 | stream/binary field content | `--no-regex` for stream payload markers (no independent fixture — conclusion derived from executed M-05/M-09/M-17 rows, see F_DIFFREVIEW §B) | IMPOSSIBLE | — (no flag combo reaches it) | global/stream DATA never enters action/search: only the *source text* of docs is scanned (XData source blocks inside .cls are text — M-05 proved those hit; stored chunks of a %Stream field are global data — M-17 precedent); CompiledProperty Type='%Stream.GlobalCharacter' rows exist (M-23) but have no doc surface | honest no — server-side impossible via this surface; workaround = sql_exec on the extent + stream OID read |
| M-19 | every class extending %RO (huge) | `graph subclasses %RegisteredObject --depth 1` | FOUND | returns 1141 with `truncated=true` (deeper levels exist — correct semantics per graph.rs:153-156) | O1: ClassDefinition WHERE Super [ '%RegisteredObject' → COUNT 1141 == returned 1141 EXACT | sql_max_rows=1000 does NOT clip it (1141>1000 — batching works); truncated bit honest here |
| M-20 | diamond + multi-super + clamp | `graph subclasses FProbe.A --depth 64/1/0`; `… FProbe.Mi --depth 10` | FOUND | d1=[B,C]; d64=[B:1,C:1,D:2]; depth 0 → `Error: configuration error: depth must be >= 1` rc=1; Mi → [FProbe.Mid] only | O3: ClassDefinition_SubclassOf('FProbe.A')={B,C,D} == BFS; _SubclassOf('FProbe.Mi')={Mid} == tool | prefix-over-match guard holds for FProbe_Mi/Mid pair (real edge, no over-match) |
| M-21 | who uses class X (impact) | `graph deps FProbe.A --direction used_by --depth 3`; `graph deps FProbe.Caller --direction depends_on` | PARTIAL (flagship) | used_by finds INHERITORS {B,C,D}; body-only caller FProbe.Caller MISSED by BOTH directions (Caller.depends_on = [%RegisteredObject] only) | O1: ClassDefinition.FProbe.Caller DependsOn = "" (CONFIRMS E_BUILD D3: ##class() calls don't land in DependsOn); literal `##class(FProbe.A)` search finds Caller (1 hit) | **body-reference blind spot CONFIRMED**: workaround `search '##class(Name)' --no-regex`; docs present used_by as "the impact query" without this caveat → F-01 |
| M-22 | who calls routine R | `search '^FProbeUtil' --no-regex --documents '*.cls,*.mac,*.int'`; `search FProbeUtil` (regex) | PARTIAL | literal `^Name` finds call sites (FProbe.Caller); regex mode finds the routine doc by metadata only (FProbeUtil.mac) | O2: fixture = planted `do ^FProbeUtil` (doc get) | substring noise without the `^`; graph tools are class-centric (no routine edges in used_by) — honest partial |
| M-23 | all properties of Type T | `search '%Stream.GlobalCharacter' --max 600` | FOUND | regex metadata scan matches attribute VALUES (13 docs/15 hits without flags; `--generated --system` → 33 docs/42) | O1: CompiledProperty WHERE Type='%Stream.GlobalCharacter' → COUNT 41 (== 42 matches incl. 1 duplicate-doc; same-class multi-prop) | docs:21 attribute-value scan claim CONFIRMED live for the first time; gen/sys defaults hide ~2/3 of users → flag needed |
| M-24 | case-insensitive search | `search fprobe-txt-9137 --no-regex` (lowercase of planted marker) | FOUND (docs honest) | literal default is CASE-SENSITIVE on this server (lowercase → `Not found.`, exact case → 1 hit); `case_sensitive:true` same result; whole_word flag also changes results (M-25) → param has wire effect in literal mode only | O2: rg -i vs rg counts (marker unique) | regex mode: case flag unobservable (engine never scans body text) — docs:36 "only take effect in literal mode" = TRUE, live-confirmed → DC-03 ok |
| M-25 | whole-word 'Set' | `search Quit --no-regex --whole-word --max 10` vs without | FOUND | word flag WORKS in literal (word: 5 docs spread; no-word: 1 doc all hits — server-side semantics differ visibly); regex-mode `\bQuit\b` = metadata scan → 0 | O2: rg `\bQuit\b` vs `Quit` over export | confirms word is literal-only, matching docs |
| M-26 | unicode query text | `search '中文字' --no-regex` re-fired via logging proxy 31511 (refactorer) | FOUND (encoding ok) | query is single param, UTF-8 percent-encoded end-to-end, echoed intact — no mojibake | proxy wire line `query=%E4%B8%AD%E6%96%87%E5%AD%97` + result echo (F_DIFFREVIEW §B) | refactorer |
| M-27 | URL metachars in query | `a+b`, `a&b`, `a=b`, `query=1&max=1` re-fired via proxy (refactorer) | FOUND (no injection) | wire shows `query=a%2Bb`, `a%26b`, `a%3Db`, `query%3D1%26max%3D1` — ONE param, `max` unaffected (stayed the sent value) | proxy wire log (F_DIFFREVIEW §B H-08 row) + coder h_wire evidence | refactorer |
| M-28 | namespace-wide member names | `search OnBuild`; `--max 5000` | FOUND (empty is correct) | regex metadata finds member names namespace-wide; OnBuild=0 in USER | O1: CompiledMethod WHERE Name='OnBuild' USER → COUNT 0 == 0 == 0 parity both caps | truncation semantics verified separately (M-29/M-03): at cap server prints limit marker and `truncated=true` |
| M-29 | max edges | `--max 0` / `1` / `-1` / `500`; MCP max_results u32::MAX | FOUND | 0 → `Not found.` truncated=false (docs:39 TRUE); 1 → 1 hit truncated=true; CLI `-1` → clap reject rc=2; 500 → 500 matches returned (server honors >200 on 2025.3); MCP u32::MAX → no panic, honest results | wire: `max=500` sent as-is (proxy log) | issue #17 "hard cap 200" is STALE for 2025.3 (confirms bench D5); BUT src/tools/search.rs:35 + mcp schema say "server hard-caps at 200" — contradicts live behavior → F-05 LOW (see DC-08) |
| M-30 | namespace default precedence | `rism search x` vs config vs `RISM_IRIS_NAMESPACE` (refactorer, executed) | FOUND | env `%SYS` echoes `%SYS`; no-env echoes config `USER` — precedence env > config.toml > defaults confirmed live (proxy-free; ns echo field) | configuration.md wording matches | refactorer |
| M-31 | table-fn vs BFS equivalence | `graph subclasses %Persistent --depth 64` | PARTIAL | BFS returns 498 with `truncated=true` | O3: `_SubclassOf('%Persistent')` = 684 total; `Super [ '%Persistent'` substring = 457 (substring ≠ real) | BFS ≠ complete on wide trees (256-query frontier budget, graph.rs:18): **truncated flag is what tells you** — 498/684 found, flag correct; deep trees need sql_exec table fn for exactness |
| M-32 | exotic document masks | `--documents ''`, `*.zzz`, `'*.cls';--`, `'*.csp,*.cls'` | PARTIAL | `''` → CLI sends `documents=` → **HTTP 400 from server** (wire shows documents= empty); `*.zzz` → clean `Not found.`; `'*.cls';--` → Not found, no injection (encoded %27); `'*.csp,*.cls'` single arg → splits to two masks in echo, wire `%2A.cls%2C%2A.csp` joined-then-encoded → comma smuggles an extra mask (by design, value_delimiter=',') | wire proxy logs | empty-mask 400 is an un-guarded CLI foot-gun → F-04 LOW |
| M-33 | production items ClassName=X | `search ClassName --max 5` | IMPOSSIBLE (this box) | config ROWS not present: `Ens.Config.Item` SQL → table not found (no production enabled in USER); ClassName as attribute text IS searchable in Ens.* source (4 docs) | O1: attempted both doors; table genuinely absent | honest no on this server: production config data = SQL tables where Ens exists (execute_sql); ties to M-05 |

## §2 Hostile inputs (coder rows: H-01..07, 09, 12, 14..17)

Zero-HTTP oracle: `RISM_IRIS_BASE_URL=http://127.0.0.1:1` variant must print the
IDENTICAL guard error (dead_same=True) — if HTTP were attempted the message would
be a connect error. Harness: scratch/bench_f/h_class_names.py (CLI door, 33 rows)
+ h_mcp.py (MCP stdio door) + h_wire.py (logging-proxy, port 31431, killed).

| id | input | expected | actual (live) | zero-HTTP proof | verdict |
|---|---|---|---|---|---|
| H-01 | `Bobby'; DROP TABLE x;--` × subclasses/members/deps | guard error, 0 HTTP | `Error: configuration error: invalid class name (interpolation guard): <payload>`, rc=1, all three tools | dead-port output byte-identical (dead_same=True, every row) | PASS |
| H-02 | `a' OR '1'='1` | refused | same guard message, all three tools | dead_same=True every row | PASS |
| H-03 | ``Robert ; ; ` `` (E_ATTACKS 7th) | refused | same guard ×3 rows | dead_same=True every row | PASS |
| H-04 | `Foo%27`, `%27%20OR` | refused pre-decode | same guard ×3 rows | dead_same=True | PASS — guard runs before urlencoding, no double-decode path reached |
| H-05 | `../../etc/passwd`, `''`, `1abc`, `a b`, `x'y` | refused | same guard ×3 rows | dead_same=True | PASS |
| H-06 | `%RegisteredObject'` | refused | same guard ×3 rows | dead_same=True | PASS |
| H-07 | metachar storm via search, BOTH doors | honest server regexp error, no crash | CLI: `#8311 Syntax error in regexp pattern` (`.*+?[](){}`), `#8313 Unrecognized backslash escape`, `#8316 Incorrectly nested parentheses` (`(`, `(?:`), `#8325 Missing closing bracket` (`[a-z`); bare `^` `$` `|` are VALID regex → searched metadata (rc 0). MCP door: same #8311 class, isError=true | error text identical across doors (h_mcp.json) | PASS — no crash, honest errors both doors |
| H-08 | `a&b`, `a+b`, `a=b`, `query=1&max=1` (param-splitting shapes) | treated as data; NEVER a second-param effect | re-fired via logging proxy (refactorer): wire shows `query=a%26b` / `a%2Bb` / `a%3Db` / `query%3D1%26max%3D1` — ONE param each, the sent `max` unaffected, query echoed intact | proxy wire log (F_DIFFREVIEW §B) — the split-attempt string never becomes two params | PASS (refactorer-executed) |
| H-10 | `--namespace "A' OR 1=1--"`, `..%2f..`, `USER&x=1`, `''` | namespaces NEVER validated → wire? | PRE-FIX (proxy log): reached the path verbatim, server HTML 400 / honest client error, no leak, no panic, `&` stayed path-side (no param split). POST-FIX (this lane): refused client-side, identical message on a dead port (zero HTTP) | dead-port parity + U20 wiremock (0 requests) | PASS-after-Fix (refactorer; guard = F-09 fix commit) |
| H-11 | `%`, `%%`, `%41`, `a%` | double-encode behavior | PRE-FIX: `%25`, `%25%25`, `%2541` on wire → 400 honest miss; POST-FIX: bare `%`/`%%`/`%41`/`a%` refused client-side (`%` must lead a letter, %SYS-shape) | names.rs unit pin + proxy log | PASS-after-Fix (refactorer) |
| H-13 | `../../`, `USER/%2e%2e` | traversal must NOT reach another endpoint | reqwest path-normalizes `../../` → `GET /api//action/search` and `../admin` → `/api/atelier/admin/action/search` — both 404 (curl-confirmed directly), `USER/%2e%2e` → `%252e%252e` 404; NO cross-route 200, no panic | curl status codes + proxy wire log | PASS (benign; now also refused pre-flight) (refactorer) |
| H-18 | `RISM_SEARCH_MAX_RESULTS=abc` / `-5` / `999999999999` | garbage keeps default 200 | all three garbage shapes → default 200 honored (match_count 200, truncated true); `=5` → 5; no panic, no zero-value leak | live env matrix (F_DIFFREVIEW §B H-18; settings.rs:202 parse_env_u32) | PASS (refactorer) — note: `%SYS%`/`USER25`-style trailing-junk namespaces (H-12) now refuse client-side post-F-09 instead of a server 404 |
| H-09 | 5000-char query; `a%b`; `%25` | echoed intact, no panic | both doors: rc 0, query echoed verbatim in result, `Not found.`/hit as data; `%25` treated literally as text (single encode %2525 on wire — data stays data) | h_wire + h_mcp json | PASS |
| H-12 | `--namespace USER25`, `%SYS%` | recorded, no panic | USER25 → server non-JSON 404 → `invalid Atelier response` rc=1; `%SYS%` → path `%25SYS%25` → same honest error; NO panic | proxy log: ns lands raw-in-path (%-replace only) | PASS (behavior recorded; asymmetry vs class guard = F-09 docs) |
| H-14 | H-01..06 payloads via MCP tools/call (24 combos) | same guard error class | `isError:true` + `configuration error: invalid class name (interpolation guard): …` on all; session alive; tools/list still 29 after | dead_port CLI variant byte-identical (dead_same=True) | PASS |
| H-15 | `{query,wild:true}`, `kind:bogus`, `direction:all`, missing `query`, `{class,extra}` | validation errors, alive | all 5 → `isError:true` `failed to deserialize parameters: unknown field/variant/missing field …`; session survives; tools/list=29 re-verified after storm | deny_unknown_fields live-confirmed (U1/U2 pin units reproduced) | PASS |
| H-16 | depth 0 (MCP), depth u32::MAX, max 0, max u32::MAX, CLI `--max -1`, CLI `--depth -1` | per graph.rs:652 / clap | MCP depth0 → `configuration error: depth must be >= 1`; depth 4294967295 → **clamped: result echo `depth:64`**; max0 → `Not found.` truncated=false; max u32::MAX → no panic honest result; CLI both -1 → clap reject rc=2 | CLI vs MCP identical clamp semantics (graph.rs resolve_depth) | PASS |
| H-17 | `kind:"x_data"`, `kind:"XData"`, `direction:"Used_By"` | refused (frozen lowercase) | all → `unknown variant …` deserialization error | schema enum spellings (tools/list $defs) == refusal set | PASS |

**Namespace headline (H-10/11/13 are refactorer-owned; wire evidence for the
handoff, h_wire.json)**: namespace is NEVER validated — `' OR 1=1--` reaches the
path segment (`A'%20OR%201=1--/action/search`, server 400 HTML → honest client
error, no data leak); `..%2f..` → `%252f..` (pre-encoded % re-encoded; traversal
defused); `USER&x=1` → `&x=1` sits in the PATH not the query (no param split);
`../../` and `../admin` are **normalized away by the HTTP client** — wire shows
`GET /api//action/search` and `GET /api/atelier/admin/action/search` (server
404s both: verified by direct curl = 404, not 200) — no cross-route access.
Empty ns → 404 honest. `%41` → `%2541` double-encode (honest miss).
Verdict: all hostile namespace inputs benign → the guard sentence in docs
covering "every interpolation site" overstates (DC-07/F-09).

## §3 CLI parity (coder rows: P-01, P-03..P-10, P-12)

Harness: p_parity.py — one live `rism --format json` call vs one
`tools/call` over `rism mcp` stdio, deep-equal after dropping `_ms` keys.
**17/18 comparisons byte-equal; the one diff is the error-text case below.**

| id | case | CLI out | MCP out | diff | verdict |
|---|---|---|---|---|---|
| P-01 | search happy (`Foo --no-regex --max 5` ≡ `{query,regex:false,max_results:5}`) + identifier case | deep-equal on namespace/query/documents/server_tally/truncated/doc_count/match_count/results | equal | — | PASS |
| P-02 | search default flags (regex omitted) | `search Foo` → wire `…&sys=0&gen=0&max=200` with **no `regex=` param** (logging proxy, F_DIFFREVIEW §B) | `{query:"Foo"}` → same single request, None-omits rule | wire-shape identical both doors; server default (regex on) applies either way | PASS (refactorer wire half executed) |
| P-03 | all four flags true | equal | equal | flag-expressiveness note: CLI `--case-sensitive`/`--whole-word` are bool SET-flags → always sent when given; CLI cannot send case=false explicitly, MCP omits-when-None. Omitted vs false is NOT observable in results (server default applies) | PASS |
| P-04 | documents repeatable | equal | equal | wire identical: `documents=%2A.cls%2C%2A.mac` (joined-then-encoded) | PASS |
| P-05 | namespace %SYS | equal | equal | wire `%25SYS` both doors | PASS |
| P-06 | empty result | equal | equal | both: match_count 0, results [], server_tally "Not found.", truncated false; CLI rc 0 | PASS |
| P-07 | empty query | `Error: IRIS returned HTTP 400 for GET …/action/search?query=&…` rc=1 | isError:true, SAME 400 text | error STRING equal modulo "Error: " prefix (keys differ only there) — semantic parity holds | PASS-with-note (DC-04 ok) |
| P-08 | graph subclasses depth 2 | equal | equal | JSON key `super` present spelled `super` on BOTH doors (renderer no rename) | PASS |
| P-09 | members ×5 kinds + inherited | equal ×6 | equal | counts vector + kind echo equal | PASS |
| P-10 | deps used_by depth 3 / default | equal ×2 | equal | truncated bit equal | PASS |
| P-11 | RISM_GRAPH_DEPTH=2 both doors | levels seen [1,1,2] | [1,1,2] | env default honored identically (not 10) — refactorer CLI-half re-check: `graph subclasses %Persistent` echoed `depth:2` under the env (default 10 otherwise); `graph deps` depth default is 1 by design (help + graph.rs resolve), unaffected by the settings key — documented asymmetry, consistent both doors | PASS |
| P-12 | hostile class | rc=1 + guard msg on stderr | isError + same msg | substring identical ("invalid class name (interpolation guard)") | PASS |

## §4 Bench re-run + methodology (coder rows: B-01, 04, 06, 08, 09, 10, 11)

Re-run: scratch/bench_f/run_f.py (imports bench/run_bench.py UNCHANGED, wraps
cli() to log every call+raw JSON to cli_log.jsonl). Window 2026-10-14 12:24-12:30 UTC.
`git checkout -- bench/RESULTS.md` after → tracked file byte-clean (verified `diff`).

| id | question | method | result |
|---|---|---|---|
| B-01 | graded metrics match published RESULTS.md? | full re-run; diff every table vs bench/RESULTS.md | recall **20/20 MATCH**; graph **11/11 MATCH**; cliff MATCH (200/199 truncated, 500 honored); docs loaded 80 / compiled 38/80 MATCH. **precision: published 20/21 (95%) vs re-run 20/20 (100%) — MISMATCH RESOLVED: the one published "other doc" was `RismCI.Probe5.cls`, a stale container fixture that E_VERIFY B3 deleted; RESULTS.md was never regenerated after that cleanup** (re-run M-01 row: 1 returned doc vs published 2). The docs-vs-artifact disagreement is 20/21 (bench/RESULTS.md:71, code-search.md:85) vs 20/20 (TASK.md §"3-run agreement") → F-02 doc drift, fixed in docs. Latency deltas re-run ≈3.6× published (2643 vs 724 p50 broad) — shared-container contention, not a product change; band claim 22-60× understated (now up to 93×) — environment, noted not graded |
| B-04 | marker placement bias | open 4 export files (M-01 plain-comment 51, M-13 string-literal 35, M-17 plain 264, M-19 regex-decoy 21 + M-190 22) | markers ride PLAIN SOURCE lines (comment or Set t="…"), never XData/stream/gen code — verified by rg -n of export (doc get == export content for these). Categories: plain→literal scan, literal→string content, regex→identifiers in metadata… recall ran in LITERAL mode for all 3 categories (docs:31 honest: "recall numbers only make sense against literal mode") |
| B-06 | recall over COMPILED docs only | join ground_truth × compiled set from RESULTS_f (38 compiled) | compiled-only recall = **11/11 = 100%** (every compiled-doc marker HIT) — recall does NOT drop restricted to compiled docs; uncompiled coverage already claimed by M-13 |
| B-08 | truncation cliff exactness | phase_d rows from cli_log | max=200 → match_count 200 EXACT + server marker; max=500 → 500 EXACT (498 docs) + server marker; doc_count 199 both — no off-by-one in the client rule; server honors 500 (issue #17 stale) |
| B-09 | truncated false-positive at total==max | planted token with exactly N hits searched with max=N and max=N+1 (B-09 probe block) | N=2 max=2 → truncated=**true** + server ITSELF appended "Maximum search limit reached." (so the marker, not just the client heuristic, fires when total==max); N=2 max=3 → false. Direction of the "lie": conservative (claims truncation when it returned everything at the exact cap). Client heuristic `match_count>=max` (iris/search.rs:149-151) agrees with the server here → DC-06 wording for refactorer |
| B-10 | cleanup honesty incl. SIGINT | fresh bench run killed `kill -INT` mid-phase (79 Bench docs on server at signal time) | `signal 2: running cleanup` → `CLEANED: 80 docs deleted; remaining Bench* docs: 0 []`; `doc list --filter Bench%` post = 0 verified independently; RESULTS.md untouched by the interrupted run (finally path didn't rewrite) |
| B-11 | graph_truth 11 checks re-derived independently | O3 table functions + O1 SQL vs BFS during window (docs were loaded); diamond/levels/prefix re-derived live on FProbe_* equivalents (M-19/M-20/M-21) | 11/11 bench checks PASS in re-run; independent derivation: subclasses(A)={B,C,D} == _SubclassOf set (exact), levels match SUPER-chain (B:1,C:1,D:2 vs diamond), Mi prefix-guard empty-correct (no over-match: FProbe.Mi→[Mid] is a REAL edge, checked via _SubclassOf) — tool does NOT grade itself here |

## §4b Refactorer methodology rows (B-02/03/05/07, D-01..06)

Recomputed offline from the coder run's saved artifacts (`scratch/bench_f/cli_log.jsonl`
raw per-call JSON + `bench/.work/export` + `ground_truth.csv`) — no re-run needed.

| id | question | method | result |
|---|---|---|---|
| B-02 | is ground_truth.csv just measuring the injector? | independent `grep -rl RISM-BENCH-M-` over the export doc set → marker→doc map, set-diffed vs ground_truth rows | **NO self-proving loop**: 20/20 markers' export doc-set == the ground_truth doc EXACTLY; all 20 recall searches returned their doc (recomputed from raw JSON). Metric measures server visibility, not the injector |
| B-03 | precision denominator = returned docs only → vacuous pass? | recompute per-marker precision from rg truth: docs_returned ∩ rg-doc-set / docs_returned | independent **20/20, zero false-positive docs, zero empty-result (vacuous) markers** this run. The structural flaw is real (an empty search could score 1/1) but cannot fire while recall is 100%; noted, no metric error in the published numbers |
| B-05 | loose hit rule (marker in ANY returned doc) inflates hits? | re-score both rules from the same raw JSON | loose 20/20 == strict 20/20 — **identical**: every marker text is unique in the corpus, decoy M-190 adjacency never crossed doc boundaries. Rule limitation is latent, honest for this corpus |
| B-07 | "1 HTTP round trip" claim; ratio band fairness | logging proxy counted requests for ONE `rism search` invocation | **2 GETs per CLI call** (`/api/atelier/` auth + the action call) — run_bench.py:537 + RESULTS.md wording FALSE → F-10 doc-fixed (2 round trips + machine-specific band note; both files updated). Ratio band re-measured on a busier container: up to 93× — now marked machine-specific in the text |

**D rows (drift_port.py vs analyze-docs-drift.ps1, mutation-tested):**

| id | assertion pair | result |
|---|---|---|
| D-01 | which *.md are scanned | MUTATION-CONFIRMED: PS scans UNtracked *.md (`-Exclude target` matches the leaf name only) — an untracked `untracked_notes.md` claiming a mismatched "ninety-nine tools" total FAILS PS (rc 1) while py skips it (rc 0, git ls-files scope). On a CI-equivalent `git archive` checkout both are byte-equivalent (56 identical ok-lines, both rc 0). The py comment's "replicates a CI checkout" claim is TRUE for CI; PS is wider on dirty trees — wontfix (CI never has dirty trees; py remains the local mirror) |
| D-02 | attr/fn pairing loss | MUTATION-CONFIRMED: stray `#[tool(...)]` after the last fn → py: clean `FAIL: attr/fn pairing lost names: 29 vs 30` (rc 1); PS: `InvalidOperation: Index operation failed; the array index evaluated to null` (rc 1 — ERRONEOUS but LOUD, not silent-green). Hypothesis "PS exits 0 silently" REFUTED — both fail; PS error text is a crash not a verdict. No CI correctness gap |
| D-03 | tool-count claim regex | byte-identical `(?<!\()\\b(\\d+)\\s+tools\\b` in both |
| D-04 | tool-name doc check | py literal `` `name` `` in doc; PS `-notmatch "``name``"` regex — equivalent for snake_case names (all 29 are); latent-only |
| D-05 | config-path promises | three regexes identical; archive-copy both-gate run: identical ok/FAIL sets |
| D-06 | semver + exit codes | py re.MULTILINE vs PS Select-String, `0.2.2` both; exit parity verified across all runs above (0/0 clean tree, 1/1 both mutations) |

## §6/DC quick results (coder rows)

| id | claim | verdict + evidence |
|---|---|---|
| DC-01 | regex=metadata / literal=source (code-search.md:16-22) | ok — M-03/M-17/M-24 live split reproduced 4 ways |
| DC-02 | uncompiled docs included (:22) | ok — M-13 (both modes, ClassDefinition row present, Compiled* absent) |
| DC-03 | case/word literal-only (:36) | ok — M-24/M-25 (flags visibly change literal results; inert in regex) |
| DC-04 | empty query → HTTP 400 surfaced (:37) | ok — P-07 both doors |
| DC-05 | max=0 → Not found, truncated=false (:39) | ok — M-29 |
| DC-08 | "200 cap is client default" (:45-47) | ok for 2025.3 (M-29/B-08) — BUT src/tools/search.rs:35 doc-comment + MCP schema say "server hard-caps at 200" → contradicts (F-05 LOW) |
| DC-09 | headline table (:84-91) | precision 20/21 STALE vs fresh 20/20 (B-01) → F-02 fixed in docs; other numbers ok (289/303 p50/p95 published ≈ fresh 1012/1047 = contention, band claim 22-60× honest for its run) |
| DC-11 | mcp-tools.md params vs live inputSchema | ok — 29 tools; required/optional sets, enum spellings `xdata`/`used_by`, defaults text, `documents` list shape ALL match jq-diff (tools_list.json) |
| DC-14 | mcp_probe.sh = 29 tools | ok — run live this session: "MCP probe OK: 29 tools, SQL answered… HTTP door answered (29 tools…)" |
| DC-15 | "Reproduce: 4-minute run; cleans every loaded doc on all exit paths" | ok — B-10 (SIGINT 80/80 cleaned, 0 Bench docs); ~4 min wall reproduced |
| DC-06 | "`truncated` reflects the server's notice" (:47-48) | MISLEADING (LOW) → doc-fixed (commit 6e7e715): code ORs the client heuristic `match_count>=max`; server itself also prints the marker at total==max (B-09), so the flag never lies — docs now explain both halves |
| DC-07 | "class names allowlisted before any request" (:62-63, mcp-tools.md) | TRUE for class names; namespace interpolation site had NO client guard (H-10..13 benign server-side but the sentence implied global policy) → fixed: guard added (commit 2c3d795) + docs state both surfaces (6e7e715) |
| DC-10 | "Tier 3 ruled out: no server xref machinery" | ok — every confirmed blind spot traces to that deferral EXCEPT the used_by wording overstatement → F-01 doc-fixed (6e7e715) |
| DC-12 | cli-reference vs `--help` verbatim | ok — flag lists match (F_DIFFREVIEW §B); stale "server hard-caps at 200" help text fixed with F-05 (2c3d795) |
| DC-13 | configuration.md 200/10 defaults + env names | ok — settings.rs:105-106/202-210 line-for-line; live precedence matrix M-30/H-18 confirms |

## Cleanup proof (handoff gate)

- `doc list --filter FProbe%` → 0 documents AND `%Dictionary.ClassDefinition WHERE Name LIKE 'FProbe%'` → 0 rows (asserted 2026-10-14 ~13:20 UTC, after all M/H/P/B rows executed).
- `doc list --filter Bench%` → 0 (post bench re-run + post SIGINT run).
- No stray `rism`/proxy processes; `ss -tln` shows no 31xxx listeners (logging proxy 31431/31432 killed by PID).
