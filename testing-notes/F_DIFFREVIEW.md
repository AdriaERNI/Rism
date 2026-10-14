# F_DIFFREVIEW — refactorer hunk review of PR #22 (task rism-pr22-review-f)

Refactorer lane. Owns the 26 F_PLAN rows marked `refactorer` (§1 M-18/26/27/30,
§2 H-08/10/11/13/18, §3 P-02/P-11, §4 B-02/03/05/07, §5 D-01..06, §6
DC-06/07/10/12/13) + the fix round for F-01..F-10. Skeleton first
(crash discipline); rows land as executed; findings rows get a
fixed(commit)/doc-fixed(commit)/wontfix(why) status in F_FINDINGS.md.

Binary under review: this worktree `target/release/rism` @ merged HEAD
(ffd3580 + coder F-era docs = code-identical to PR head). Server: docker
rism-iris (2025.3 b226U). Wire oracle: logging proxy on localhost (no
credentials leak; upstream = config base_url).

## §A Hunk review notes (git show 75272e8..HEAD -- src packaging bench)

- **src/iris/search.rs:50-84** — query and joined documents pre-encoded with
  `urlencoding::encode`, then concatenated into a raw string handed to
  `reqwest.get(&str)`. Live proxy proof (§B H-08): `&`,`+`,`=`,`%` all survive
  as single-param data (`%26/%2B/%3D/%25`), never a param split. No bug.
- **src/iris/search.rs:103-179 parse shapes** — array/null/`{}` empties
  accepted, foreign shapes → InvalidEnvelope (U6 + probe `{}`/`[]` parity ok).
  Line normalization: numeric string → `line`, `"Description+32"` → `label`
  with `line:null`, never dropped. Live JSON shape check (§B M-12-follow):
  class-attr hits show `attr`, `line:null`, `label:null` — agent sees
  member/attr/text as position; docs:33-35 honest.
- **truncated logic (search.rs:149-151)** — server marker OR client
  `match_count >= max`: at total==max the flag fires `true` even when the
  server returned everything (B-09 confirms the server ALSO prints the limit
  notice there, so no lie in practice; but the docs never mention the client
  half) → DC-06 wording fix (doc-fixed F-05 area).
- **src/tools/search.rs** — param mapping matches docs (`sys`/`gen` always
  sent, `case`/`word`/`regex` None-omit; §B P-02 wire proof: `regex` absent
  when flag unset). `documents` fallback `filter(!is_empty)` misses the
  `Some([""])` CLI shape → server 400 (F-04, fix = drop empty strings).
  Line 35 comment "server hard-caps at 200" contradicts live 500-honored
  (F-05, fix = comment + schema description).
- **src/tools/graph.rs** — BFS: cycle-safe via `visited`, depth clamp 64,
  budget 256 sets truncated (M-31 498/684 truncated=true honest). Diamond
  supers merged via exact re-split (`super_split_exact`) — prefix guard re-proved
  by coder M-20; depends_on+Super merge (D3) correct. **`WHERE parent = '<name>'`
  and `WHERE Name = '<name>'` are exact-match on the REAL class name: alias names
  (`%RegisteredObject`) → misleading all-zero rc=0** (F-03; §B re-derivation:
  NO `Alias` column exists in `%Dictionary.CompiledClass`/`ClassDefinition` on
  2025.3 — the brief's "query Alias" route is a dead end; `%ExistsId` confirms
  alias rows simply aren't in the dictionary by that Name. Fix = existence-check
  before zeros, honest not-found instead). Namespace NOT validated here → F-09.
- **src/tools/names.rs** — reject-set is per-char allowlist, correct for class
  names; no namespace predicate existed (F-09 fix adds one, mirroring the
  class rule; wire evidence shows hostile ns values were all benign 400/404,
  so this is defense-in-depth + docs honesty, not a CRITICAL).
- **src/cli/mod.rs search arms** — flag parity ok (docs/cli-reference.md:45-47
  matches `--help` verbatim, DC-12). `--documents` with `value_delimiter=','`
  always builds `Some(vec)` → empty-string foot-gun (F-04). `--max -1` clap
  rejects (rc 2, H-16 ok). CLI help says "server hard-caps at 200" (F-05).
- **settings** — defaults 200/10 match configuration.md:27-28, 63-64 (DC-13
  ok); env garbage keeps default (§B H-18 matrix proves all three garbage
  shapes hold 200).
- **src/mcp/mod.rs stubs** — four descriptions accurate; `class_members`
  description names the 4 tables (no oversell); `class_dependencies` says
  "DependsOn/Super references" (accurate) but docs/mcp-tools.md:246 calls
  used_by "the impact query" without the body-ref caveat → F-01 (doc fix).
- **packaging/mcp_probe.sh** — count logic greps `"name":` occurrences in the
  tools/list response and requires ==29 — sound (not just green: verified it
  fails when a name is removed, coder H-15 re-check).
- **bench/run_bench.py:537** — "1 HTTP round trip" is FALSE: proxy counts 2
  GETs per CLI call (auth + action; §B B-07) → F-10 doc fix.
  Precision denominator uses returned docs only (B-03) — recomputed
  independently from export rg truth: 20/20, no vacuous 1/1 case, so the
  published metric did not hide a failure this run (limitation noted in docs?
  no — bench/RESULTS.md prose, LOW; recorded here only).
- **bench/drift_port.py vs ps1** — §B D rows: identical on CI-equivalent tree;
  deltas are exactly the gitignore/untracked-scope issue (D-01, PS-only on
  dirty trees) and PS `Index op failed on null` crash-vs-clean-FAIL on a
  stray `#[tool(` at EOF (D-02; py's bad() check is strictly better).

## §B Refactorer matrix rows (26)

All executed; wire oracle = logging proxy port 31511 → 52773 (killed by PID
after runs; log scratch-free at /tmp). Binary = this worktree
`target/release/rism` @ 2d3dfec tree (code == PR head ffd3580).

| id | result (executed) |
|---|---|
| M-18 | stream/global CONTENT IMPOSSIBLE via action/search (docs-only scan) — consistent with M-05/M-09 XData-source hits; matrix row closed: honest "no workaround tool-side" (Tier-3). |
| M-26 | unicode `中文字` → wire `query=%E4%B8%AD%E6%96%87%E5%AD%97` (coder proxy) + re-fire via proxy: single param, echo intact, no mojibake in echo — FOUND (encoding ok end-to-end). |
| M-27 | `a+b`, `a&b`, `a=b`, `query=1&max=1` → wire `query=a%2Bb`, `a%26b`, `a%3Db`, `query%3D1%26max%3D1`: ONE param, no split, max unaffected — data stays data. FOUND (no injection). |
| M-30 | precedence: `RISM_IRIS_NAMESPACE=%SYS` beats config USER (ns echo `%SYS`); no-env run echoes USER (=config). Env > config > defaults chain holds. FOUND (docs/configuration.md wording ok). |
| H-08 | see M-27 rows — no second-param effect; 0 hits on USER data (params intact). PASS. |
| H-10 | `A' OR 1=1--` → wire path segment verbatim (`A'%20OR%201=1--/action/search`), server HTML 400 → honest error rc=1; no data, no crash. namespace NEVER validated client-side → F-09 guard added. |
| H-11 | `%41` → `%2541` (double-encode anomaly), honest miss 400; `%%`→`%25%25`, `%`→`%25` 400; empty → 404. All benign. |
| H-13 | `../../` → reqwest NORMALIZES to `GET /api//action/search` (404 — curl-confirmed 404); `../admin` → `/api/atelier/admin/action/search` (404 confirmed); `USER/%2e%2e` → `%252e%252e` 404. NO cross-route 200. Benign; guard now rejects client-side anyway. |
| H-18 | `RISM_SEARCH_MAX_RESULTS=abc/-5/999999999999` → all keep 200 (match_count=200, truncated=true); `=5` → 5. Garbage-keeps-default CONFIRMED (settings.rs:202 parse_env_u32). PASS. |
| P-02 | CLI `search Foo` wire shows **`regex` param ABSENT** (not `1`): `…query=Class&documents=…&sys=0&gen=0&max=2` — None-omits rule holds; MCP `{query}` identical. PASS. |
| P-11 | `RISM_GRAPH_DEPTH=2`: subclasses → depth=2 (not 10); deps ignores settings (always default 1 per graph.rs:461 — matches its `--help` "default 1"; note: settings key only affects subclasses). Env garbage (`0`,`banana`) → clamp 1 / keep default. Identical both doors (coder's half). PASS. |
| B-02 | independent oracle: export-rg marker→doc set == ground_truth.csv 20/20 EXACTLY (no self-proving loop); all 20 markers returned their doc → recall phase measured, not tautological. |
| B-03 | independent precision from raw JSON + export-rg truth = **20/20, zero false-positive docs, zero vacuous (empty-result) markers**. The denominator flaw EXISTS in run_bench.py:303 (can't fail if a search returns nothing) but did not bite this run → LOW methodology note, not a metric error. |
| B-05 | strict (doc AND marker-in-that-doc) vs loose (marker in ANY returned doc) hit rules: 20/20 == 20/20 — no cross-doc inflation in this corpus. |
| B-07 | per CLI call: wire shows EXACTLY 2 GETs (`/api/atelier/` auth + action) → "1 HTTP round trip" (run_bench.py:537, RESULTS.md:30) FALSE → F-10 wording fix (2 round trips; ratio band machine-specific note). |
| D-01 | both gates on REAL worktree: py rc=0; PS rc=1 (FAIL `swarmforge/TASK.md claims 4` — untracked-but-gitignored dir scanned by PS's `-Exclude target` leaf-name bug). On `git archive` CI-clone: BOTH rc=0, 56 identical ok-lines → py matches CI PS semantics; PS is strictly wider on dirty trees. Latent CI-nit only (CI always uses clean checkout) → documented here, no fix (wontfix + why). |
| D-02 | mutation (stray `#[tool(...)]` at EOF): py FAILS cleanly (`attr/fn pairing lost names: 29 vs 30`); PS CRASHES (`InvalidOperation: Index op failed; array index null` — nonzero exit though, so CI still red). Same verdict polarity, worse UX. py's bad() guard is correct; PS null-index latent → wontfix (installer script mirrors Windows CI; behavior parity: both fail). |
| D-03 | claim regex `(?<!\()\b(\d+)\s+tools\b` byte-identical in both. OK. |
| D-04 | tool names all snake_case → PS `-notmatch "``$tool``"` regex-safe today; py uses literal `in`. Equivalent (latent escaping nit only). |
| D-05 | config-path promises: three regexes identical; live both-gate run on archive copy: identical ok/FAIL sets. |
| D-06 | semver: py re.MULTILINE == PS Select-String; both read `0.2.2`; exit parity verified (both 0 on archive tree; D-01/D-02 runs show PS `$LASTEXITCODE`=fail-flag). |
| DC-06 | `truncated` = server marker OR client `match_count>=max` — docs mention only the server half → **misleading (LOW)**: fixed in docs (total==max now explained; B-09 shows server fires its own marker there, so flag never actually lies). |
| DC-07 | guard sentence covers class names ONLY; namespace interpolation site unguarded → docs now state both honestly + client guard added (F-09 fix). |
| DC-10 | every confirmed blind spot traces to Tier-3 xref EXCEPT used_by body-refs wording → F-01 (docs overstated). verdict: docs honest after F-01/F-06 fixes. |
| DC-12 | `rism search --help` / graph arms vs cli-reference.md:45-47 — flag list matches verbatim (`--documents` repeatable+comma, `--no-regex`, `--system`, `--generated`, `--case-sensitive`, `--whole-word`, `--max`). ok. |
| DC-13 | configuration.md:27-28/63-64 (`200`, `10`, env names) == settings.rs:105-106/202-210. Live precedence matrix (M-30/H-18) matches. ok. |

## §C Fix round (F-01..F-10)

Gates after the round: fmt clean; clippy -Dwarnings host OK; clippy -Dwarnings
`rustup run 1.99.0` OK; `cargo test -q` all suites ok (incl. new U19-U21 +
namespace_allowlist_shape pins); live `cargo test --test tool_matrix --
--ignored --test-threads=1` **23/23 ok**; mcp_probe 29 tools;
`python3 bench/drift_port.py` 0 fails; `mkdocs build --strict` ok (worktree).
NOTE: the live suite initially hit `#6713 Start target failed` + container
`unhealthy` (LICENSE LIMIT EXCEEDED) after I ran the parallel default-thread
suite; console slots reclaimed after ~7 min idle and the serialized re-run
went 23/23 green. No doc fixtures leaked during the window (verified below).

| id | action | commit | before → after evidence | second-pass |
|---|---|---|---|---|
| F-01 | doc-fixed (HIGH) — code-search.md Tier-2 section + mcp-tools.md `class_dependencies`: used_by reframed as DependsOn/Super edges, explicit body-reference blind spot + `search '##class(X)' --no-regex` workaround; behavior unchanged (proven honest-by-design, docs were the defect) | 6e7e715 | before: `graph deps FProbe.A --direction used_by` → inheritors only, docs call it "the impact query"; after: docs state the edge set + workaround verbatim | re-read shipped docs + re-ran coder's M-21 probe shape: wording now matches executed truth |
| F-02 | already closed by coder (doc drift fixed at 2d3dfec) — verified no residual "20/21" claim | — (coder e25353f) | grep: only historical notes mention it | ok |
| F-03 | FIXED (MEDIUM) — graph.rs: all-zero members read / empty subclasses / empty deps now cross-check `%Dictionary.ClassDefinition WHERE Name='<class>'`; no row → honest `class not found … if this is a class alias … query the canonical name`. Alias→canonical SQL rewrite IMPOSSIBLE (2025.3 has no Alias column in %Dictionary — probed: `Field 'ALIAS' not found` on CompiledClass/ClassDefinition in USER+%; `%ExistsId(alias)`=0) so honest-not-found is the minimal correct fix. U19 wiremock pin | 2c3d795 | before: `graph members %RegisteredObject` → `{counts:0/0/0/0}` rc=0; after: `Error: configuration error: class '%RegisteredObject' not found … canonical name, e.g. %Library.RegisteredObject` rc=1; canonical still 19/3/1/0; live fixture `FProbe.AliasT` (AliasName=FProbeAliasT): members/deps/subclasses all honest-not-found under the alias, correct under canonical; leaf class under real name still `count 0` ok | fixture recreated + deleted second pass; FProbe count 0 docs/0 rows after |
| F-04 | FIXED (LOW) — tools/search.rs: empty strings inside `documents` dropped before fallback → `--documents ''` uses default masks instead of server 400. U21 pin | 2c3d795 | before: `search Class --documents ''` → HTTP 400 (`documents=` on wire); after: rc 0, wire shows `documents=%2A.cls%2C%2A.mac%2C%2A.int` (proxy-verified) | second run identical results to masked-default search |
| F-05 | FIXED (LOW) — stale "server hard-caps at 200": SearchCodeArgs doc-comment (schema description), CLI `--max` help, iris/search.rs comment, MCP search_code description all corrected | 2c3d795 | before: tools/list `max_results.description` said hard-caps at 200; after: live `tools/list` shows "default: settings search_max_results = 200; newer servers honour higher values" and `--help` matches; 29 tools intact | re-dumped tools/list post-build |
| F-06 | doc-fixed (MEDIUM) — code-search.md practical-rules bullet: `.inc`/`.csp` need explicit `--documents`; default-mask `Not found.` for a macro name is a mask miss | 6e7e715 | before: docs never mention inc/csp; after bullet present | cross-checked vs coder M-10/M-16 live rows |
| F-07 | doc-fixed (MEDIUM) — mcp-tools.md class_members: four CLASS-member tables only; Queries/Triggers/Indices/Procedures named as NOT-listed with sql/literal workarounds; origin caveat ("when the dictionary reports one", F-07b) | 6e7e715 | before: "full member list" implied completeness; after explicit exclusion list | vs M-06/M-07 oracles |
| F-08 | doc-fixed (LOW) — code-search.md bullet: literal mode scans BODY sections; UDL header lines (Include/Class…Extends) are metadata-mode only (attr IncludeCode) | 6e7e715 | before: "raw source text" overpromised; after caveat | vs M-11 live rows |
| F-09 | FIXED (MEDIUM, docs+guard) — names.rs `valid_namespace` (%-or-letter start, alnum/_/- rest) enforced BEFORE HTTP in search_code + all three graph tools (where the ns interpolates into the URL path); docs state class-name AND namespace guards. U20 pin (zero HTTP, all four surfaces + %SYS regression) + names unit pin | 2c3d795 | before: hostile ns reached the wire (server benign 400/404); after: `--namespace "A' OR 1=1--"` → `Error: configuration error: invalid namespace (URL path guard)`, byte-similar refusal on a DEAD port (proves pre-HTTP); `%SYS`/`USER`/`MY-APP` unaffected (M-14-style %SYS members still 19/3/1/0 post-fix) | dead-port + live parity check re-run |
| F-10 | doc-fixed (LOW) — bench/RESULTS.md + run_bench.py wording: 2 HTTP round trips per CLI call (auth + action, proxy-verified) and ratio band marked machine-specific | 6e7e715 | before: "~1 … 1 HTTP round trip"; after: corrected in both files (regenerates identically on next run) | B-07 proxy log cited |

wontfix: D-01 (PS scans untracked md — CI checkouts are clean by construction;
local devs have drift_port.py), D-02 (PS crash-on-stray-attr — fails loudly,
correctness parity with py preserved; the installer copy mirrors Windows CI
byte-for-byte and was NOT under review for behavior). M-18/B-03 are recorded
conclusions, not defects.

