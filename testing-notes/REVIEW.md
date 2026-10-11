# R1 consolidated review — streamable-HTTP door + MCP surface @ cb51ecf (PR #18)

Architect lane (CONSOLIDATE), 2026-10-10. Merges REVIEW_PLAN.md (specifier),
REVIEW_FACTS.md (coder), REVIEW_ATTACKS.md (refactorer) into one findings
table + verdict. This swarm changed NO code; PR #18 untouched.

## Scope
Diff `9a1ddd7..cb51ecf` on `feat/windows-mcp-http` (16 files, +1483/−18):
src/mcp/http.rs (door), src/mcp/mod.rs, src/main.rs, src/cli/mod.rs,
src/settings.rs, tests/{http_door,http_door_stateless,cli_transport}.rs,
packaging/mcp_probe.sh, docs/*. Interacting surface reviewed adjacent:
src/mcp/tasks.rs, src/tools/jobs.rs, call_tool capability gate (Rail-A),
rmcp 3.4.1 vendored streamable_http_server.

## Method
SPEC lane mapped questions A–G to file:line + authority and wrote the probe
plan. VERIFICATION lane ran the cheap probes (live docker rism-iris, env -i,
gates green: cargo test/clippy/fmt tails in REVIEW_FACTS.md). ADVERSARIAL
lane ran churn/mutation/race attacks. CONSOLIDATE re-ran the disputed
probes personally (below). Every finding cites measured output, a source
line at cb51ecf, or a spec quote; uncited opinions are labeled nit.

Authorities: MCP spec 2025-06-18 §Transports (Security Warning re-fetched
verbatim for this report: "Servers MUST validate the Origin header on all
incoming connections to prevent DNS rebinding attacks"; follow-up section:
session-id SHOULD-400 on missing id); vendored rmcp-3.4.1 streamable_http
source (tower.rs/session/local.rs lines quoted per finding);
documentation/rust-bestpractices.md; documentation/mcp.md (§8 stdout rule
line 238 async-everywhere pitfall; §11 entropy note line 388).

### Conflict resolution (architect re-probes — my run wins per role)
1. **GET-family status matrix** (FACTS A5 said 400/406; ATTACKS §7 rows said
   405 `allow: POST` with sid-capture failure): RE-PROBED on a fresh
   `--port 0` door with full `-D` header dumps — results match FACTS:
   GET no-sid `*/*` → **406** "Client must accept text/event-stream";
   GET no-sid `text/event-stream` (± MCP-Protocol-Version header) → **400**
   "Session ID is required"; GET sid+es → **200** `text/event-stream`
   (priming `data:`/`id: 0`/`retry: 3000`, stream held); GET sid+`*/*` →
   **406**; PUT/OPTIONS → **405** `allow: GET, POST, DELETE`; DELETE no-sid
   → **400**; POST tools/list no-sid → **422**. ATTACKS' 405 rows are
   RETRACTED as the harness artifact their own caveat suspected (sid=''
   runs against a stale door). The two ATTACKS-proven extras (415 wrong CT,
   413 >4 MiB) stand — orthogonal to the conflict.
2. **FACTS F3 "blocking getaddrinfo on async path"** — downgraded to nit:
   proven dead in practice by ATTACKS §2's finding (see F-02/P-08): the
   resolver branch can never call DNS for any input.
3. **A1 leak question** — CLOSED as bounded: keep_alive=300 s reaper fires
   for all session shapes including init-only-without-`initialized`
   (ATTACKS §1: 500 sessions → all DEAD(404) at 272–302 s; second churn
   cycle shows no re-growth; residual = glibc arena high-water ~37 MB at
   500 concurrent, flat).

## Findings table

| ID | Sev | Area | Where | Finding | Evidence | Recommended fix |
|----|-----|------|-------|---------|---------|-----------------|
| F-01 | **major** | security | src/mcp/http.rs:149–159 (config site; no Origin code anywhere) | **The Origin header is never validated on any bind** — rmcp short-circuits when `allowed_origins` is empty and `validate_empty_origin_allowlist=false` (tower.rs:197, :932–936) and our config never calls `enforce_origin_validation()` — violating the spec Security-Warning MUST on a no-auth, 25-tool door. Under `--allow-all-interfaces` the Host guard is additionally cleared (http.rs:154–158 → tower.rs:214–216, :811–813 ⇒ allow ALL). | Architect re-run: `Origin: https://evil.test` → **200** on loopback AND allow-all doors; `Host: evil.example` → 403 loopback / 200 allow-all (guard live, and gone under opt-out — matches FACTS A3 + ATTACKS §3). Spec 2025-06-18 transports §Security Warning line 76 (verbatim quote above). Mitigation measured: cross-origin *browser* drive-by is de-facto blocked by rmcp's 415 on form content-types + OPTIONS→405 (no CORS preflight can succeed), so exploitability is low — the MUST gap and the same-IP/XSS pivot class remain. | Non-breaking, config-only: `config.enforce_origin_validation()` with an explicit origins allowlist (must be verified against rmcp semantics so Origin-less native clients — curl, MCP SDKs — still pass; the knob belongs in settings tier like the rest). Add a status-pinning test for guard-on/off matrix. |
| F-02 | minor | security/rust | src/mcp/http.rs:50–52 | **Loopback NAMED hosts are refused by dead code**: `str::to_socket_addrs` on a *portless* name always errors "invalid socket address" (std treats `str` as "host:service"), so the `is_ok_and` guard is unreachable — only the `"localhost"` literal (http.rs:44) and IP literals (http.rs:47) can ever pass. `--host ip6-localhost` / any /etc/hosts loopback name → refusal whose error text steers the user to `--allow-all-interfaces`, which for that same name BINDS LOOPBACK FINE but flips the exposure posture + disables the Host guard (silent security escalation to fix a non-problem). This also subsumes the B1 TOCTOU: the gate never resolves names at all. | Architect re-run (rustc probe): to_socket_addrs("localhost"/"ip6-localhost"/"127.0.0.1") → all `Err(InvalidInput)`; `--host ip6-localhost` and `--host hermes-agent` (127.0.1.1) refuse exactly as predicted; `--host ip6-localhost --allow-all-interfaces` binds `[::1]:P`. ATTACKS §2 (independent). Unit test http.rs:230 passes only because the branch is dead. | One line: `tokio::net::lookup_host(format!("{host}:0")).await` (async, also closes nit P-08) — or drop the branch and document "IP literal or 'localhost' only". Non-breaking. |
| P-03 | minor | mcp-protocol | delegated — rmcp tower.rs:1766+ (no line in our repo) | Missing `Mcp-Session-Id` on a POST that is not initialize answers **422** ("Unexpected message, expect initialize request"); the spec says a session-requiring server SHOULD answer **400** (GET/DELETE no-sid → 400 IS spec-shaped, measured). FACTS A5 corrected the plan's "POST → 400". | Architect + FACTS both measured 422; spec 2025-06-18 §Streamable HTTP session-management. | Nothing in our code to change (rmcp-owned). File/note an upstream rmcp issue; no doc claim depends on it. |
| P-04 | minor | tests | tests/http_door_stateless.rs:136–142 | DELETE leg asserts only `.expect("delete answered")` — no 202 status assert and no follow-up POST-on-deleted-sid → 404, though the comment promises "the id must be dead". Behavior itself is correct. | FACTS A4 measured 202 then 404; ATTACKS §6 re-confirmed the vacuity at read level; file quoted above. | Two asserts in the existing test (status == 202; POST after DELETE → 404). Non-breaking. |
| P-05 | minor | tests | tests/cli_transport.rs:~50–85; whole 4xx family | (a) Connect-must-fail assertions on FIXED port 39999: a foreign squatter flips a true contract to red — the flake channel was demonstrated. (b) NO test pins any 4xx of the status matrix (406/400/404/405/415/413) or the allow-all ⇄ Host-guard toggle: mutation A (delete the `disable_allowed_hosts` branch) keeps 7/7 tests green — the security-surface pin is missing. (c) TASK.md's "port 3111" does not exist in the tree (grep empty — corrected). | ATTACKS §6 (squat experiment FAILs cli_transport; mutation PASSES suite), FACTS F5 (5× flake sniff clean in-tree — risk is environmental). | Ephemeral-port discovery (bind `:0`, drop, reuse number) for (a); one status-matrix test + guard-toggle pin for (b). |
| P-06 | minor | tests | packaging/mcp_probe.sh:199–203 | "SIGTERM exit 0" is a hardcoded echo: `wait $http_pid 2>/dev/null || true` swallows the status before the claim prints. | FACTS G4 + ATTACKS §6 (child trapping TERM and exiting 3 still prints OK; control without `\|\| true` shows the failure). The claim is currently TRUE (door exit 0 measured by two lanes, FACTS D1/ATTACKS §5) — but unasserted. | `kill -TERM; if ! wait $http_pid; then echo FAIL; exit 1; fi` before the OK echo. |
| P-07 | minor | parity | src/mcp/http.rs:106–146 vs :167 | Ready line prints only after handler build (negotiate): with a dead/slow IRIS the socket is ALREADY listening (backlog accepts) for up to timeout_secs=30 before the line. Harnesses that gate on port-open race a not-yet-ready door; mcp_probe's ready-line budget is 40×0.25 s = 10 s — would time out on an IRIS slower than 10 s while the door itself is healthy. | FACTS B4 measured (listener in `ss` at t+26 s, ready line at t+30.2 s, blackhole base_url); docs say "actual port printed on the stderr ready line" — honest about mechanism, silent about lag. | Print the ready line immediately after bind (the port is the same), or document the lag. Cosmetic-but-cheap. |
| P-08 | nit | rust | src/mcp/http.rs:51 | FACTS F3's blocking-std-DNS-on-async-path is confirmed in SOURCE but UNREACHABLE at runtime (dead branch — see F-02; IP literals parse at :47, "localhost" returns at :44, and the actual bind at :106 is tokio-async). No stall exists on any input. | rustc probe (F-02 evidence) + path enumeration; supersedes FACTS F3 severity. | Fold into F-02's fix (`lookup_host`); no standalone action. |
| P-09 | nit | mcp-protocol | src/mcp/mod.rs:646 vs src/mcp/http.rs:167–170 | Banner asymmetry: stdio readiness is a tracing-timestamped INFO appearing after the first initialize; http readiness is a bare eprintln at startup. Both stderr-only, stdout purity holds on both (measured; http_door.rs:198–201 pins). | FACTS D3. | Cosmetic — unify format if the door code is touched anyway. |
| P-10 | nit | parity | src/settings.rs:11–13 | "Prism parity" comment oversells the alias map: Prism also accepts `sse`, we reject it (parse_alias `_ => None`, settings.rs:27–36; test pins 'sse'→None). Rejecting SSE is the RIGHT call (deprecated by the spec) and our docs promise it nowhere (grep docs: zero hits) — only the comment implies full parity. | FACTS G2 (Prism serve.py:18 vs ours). | Reword to "Prism-compatible aliases (minus deprecated sse)". |
| P-11 | nit | mcp-protocol | rmcp tower.rs:819–823 | Host allow-list match is PORT-AGNOSTIC (`Host: 127.0.0.1:<wrong port>` → 200): two loopback doors cross-ack Headers; rmcp-owned, spec does not mandate port pinning. | ATTACKS §3 measured. | Upstream note only. |
| P-12 | nit | parity | src/cli/mod.rs:121–124 + docs/configuration.md | `--port` has an env tier (RISM_MCP_PORT) but `--host` has none (CLI+config.toml only) — undocumented asymmetry; docs match the code honestly. Also: running two `rism mcp` processes (natural way to serve stdio+http) silently splits the 16-job cap 2× and pid-routes task ids (ATTACKS §4 C3 structural proof: id embeds creator pid). | FACTS G2, ATTACKS §4. | One configuration.md line per item. |
| P-13 | nit | security | docs (mcp.md / getting-started HTTP section) | Session-memory churn shape is undocumented: ~73 kB per live session until keep_alive (300 s) reaps — flat/bounded, NOT a leak (proven), but a steady initialize-flood is a cheap memory amplifier on a no-auth door. | ATTACKS §1 (500-session proof, all DEAD at 272–302 s; cycle-2 no re-growth). | One sentence in the HTTP-door docs ("sessions live ≤5 min; bound the door with a reverse proxy if exposed"). |

**No finding touches the frozen surface** (25 tools, wire names, stdio-door
behavior). The bare-stdio door was proven byte-identical to base by
rebuilding 9a1ddd7 and `cmp`-ing an init+initialized+tools/list session
(FACTS G1) — no breaking-change section is needed for this review.

## Verified-OK ledger (attacked, not broken)
- **A2 session-id entropy**: uuid v4 (122 bits, getrandom CSPRNG — Cargo.lock
  chain verified), 200/200 unique, canonical hyphenated form satisfies the
  visible-ASCII MUST. Stronger than task ids (jobs.rs:181–189 documented).
  ATTACKS §8 could not break the method. PROVEN-OK.
- **A4 DELETE termination behavior** (202, id then dead 404), **A6 ::1 path**
  (binds, bracketed ready URL, Host `[::1]:P` accepted via normalize_host),
  **B3 refusal UX names the flag** (http.rs:66–72, pinned
  cli_transport.rs). PROVEN-OK.
- **C1 factory cadence**: handler built ONCE (http.rs:146–147); rmcp clones
  the service per request, factory only at initialize/inline-serve sites
  (tower.rs:1890/:1983); oracle = exactly 1 "negotiated Atelier" across 5
  sessions + 10 inlines (coder) and across 500 initializes + 300+ inlines
  (refactorer). PROVEN-OK.
- **C2/E3 shared state**: jobs registry static (jobs.rs:134, 16-cap
  process-wide); dbgp sessions global OnceLock (tools/debugger.rs:149–151);
  NO std MutexGuard crosses an `.await` anywhere in jobs.rs (exhaustive
  adversarial audit, §4). 50×tools/list + 10×tasks/get during a live job:
  max 41 ms, zero errors. PROVEN-OK.
- **D1 lifecycle**: SIGTERM/Ctrl+C → exit 0 <1 s, `quit_reason=Closed`,
  races with 30 concurrent initializes and an open GET stream clean;
  background children die with the door (marker test: not orphaned).
  PROVEN-OK.
- **E1 dead-IRIS contract**: every tool answers; background=true without a
  Tasks declaration gets the Rail-A refusal (mod.rs:436–456) naming the
  extension; the SEP-2575 inline lifecycle (full 2026-07-28 header set)
  reaches terminal `failed` on a dead IRIS without hanging; `tasks/result`
  → -32601 is an INTENTIONAL pinned decision (tests/background_jobs.rs:626–
  629), correctly promised nowhere (server instructions say poll tasks/get).
  PROVEN-OK.
- **G3 size claim**: +18.5% reproduced byte-exact (11,115,648 B release at
  this tip vs 9,378,168 B base). **G1 stdio purity** byte-diff proven.
  **Gates**: fmt clean, clippy --all-targets -D warnings silent, test suite
  green (tails in REVIEW_FACTS.md).

## Ranking

FIX-BEFORE-MERGE (critical/major with user-facing/security weight):
1. **F-01** — Origin spec-MUST unmet on a no-auth door. One config call + a
   pinning test; decide the origins knob semantics first (native clients
   send no Origin — verify rmcp's behavior with the flag set so the fix
   does not break curl/SDK users).

Strongly recommended in the same pass (cheap, adjacent code):
2. **F-02** (+ folds P-08) — one-line `lookup_host` fix; it is the ONLY
   finding where our own error UX can push a user into a needless
   no-auth LAN exposure.
3. **P-04/P-05/P-06** — the test pins (guard toggle, DELETE legs, 4xx
   matrix, probe exit-code honesty). Mutation evidence shows today's suite
   would not catch a regression in the door's two security behaviors.

FOLLOW-UP (rest of minors/nits): P-03 (rmcp issue note), P-07 (ready-line
ordering — decide print-after-bind), P-09…P-13 (wording/docs nits).

## Verdict
**FIX-BEFORE-MERGE** — the door is well-built Rust and spec-honest in
behavior (every measured protocol edge correct except the rmcp-owned 422;
lifecycle, factory, shared-state, entropy, and stdio-purity all survived
two independent adversarial passes); the single gate is F-01, the
unvalidated-Origin spec MUST, with F-02 the one-line bind-gate fix worth
landing beside it. No frozen-surface violation; no breaking findings.

## PR-comment-ready summary (post to #18 if the user chooses)

> **R1 swarm deep-review verdict: FIX-BEFORE-MERGE (1 blocker).**
> Adversarial, evidence-first review of the streamable-HTTP door at
> `cb51ecf` (3 lanes: plan → measured facts → attacks; findings-only, no
> code changed). Protocol behavior measured correct on every probed edge:
> session lifecycle (init/DELETE→202/404-after, 300 s idle reaper proven
> over 500 sessions), 406/400/405/415/413 statuses, one-negotiate factory
> invariant, SIGTERM exit 0 under races, uuid-v4 CSPRNG session ids,
> byte-identical stdio door, dead-IRIS answers-everything contract incl.
> Rail-A and the SEP-2575 inline task lifecycle. **Blocker:** the MCP spec
> Security Warning MUST (validate `Origin`) is unmet on every bind — rmcp's
> Origin validation is never enabled (`enforce_origin_validation()` absent;
> empty allow-list short-circuits) — measured `Origin: https://evil.test` →
> 200, and under `--allow-all-interfaces` the Host guard goes off too
> (`Host: evil.example` → 200). Practical browser drive-by is de-facto
> blocked (415 on form CTs + no CORS preflight), but the MUST is the MUST;
> fix is config-only. One-line co-fixer: `is_loopback_host`'s DNS branch is
> dead (`to_socket_addrs` needs a service part) so loopback *named* hosts
> (`ip6-localhost`) refuse with an error nudging toward the
> exposure-flipping `--allow-all-interfaces` flag. Plus 5 test-pin findings
> (mutation-proven: deleting the Host-guard branch keeps CI green; probe
> "SIGTERM exit 0" echo is vacuous; 39999-fixed-port assert squats flake)
> and 7 minors/nits. Full table: testing-notes/REVIEW.md.
