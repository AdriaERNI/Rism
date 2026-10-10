# R1 REVIEW_FACTS — VERIFICATION lane (coder), 2026-10-10

Head run: `18fd1c9` (= specifier plan merge; production tree identical to
`cb51ecf` — the merge fast-forwards over the door commits only).
Environment: docker `rism-iris` Up (healthy, curl 401 on /api/atelier/ = auth
wall, up per repo rule), cargo 1.97.0, debug + release binaries built ONCE in
this worktree. All probes `env -i HOME=$HOME PATH=<worktree>/target/debug:/usr/bin:/bin`.
Plan verdicts below use the plan's criteria: PASS / FINDING / INCONCLUSIVE /
OPEN-FOR-ATTACK.

## A. Session / security model

### A1 — LocalSessionManager churn + idle GC — PASS (with note), leak-shape OPEN-FOR-ATTACK
Source (verified quotes, rmcp-3.4.1):
- session/local.rs:34 — `pub sessions: tokio::sync::RwLock<HashMap<SessionId, LocalSessionHandle>>` (no cap).
- local.rs:67 insert at create_session; local.rs:84–90 removal in `close_session`.
- local.rs:1254 `DEFAULT_KEEP_ALIVE = Duration::from_secs(300)`; local.rs:1125–1126 idle arm returns `WorkerQuitReason::IdleTimeout`; tower.rs:1334–1371 `spawn_session_worker` calls `close_session` when the worker exits → map entry reclaimed ~5 min after last activity.
Measured (debug door, `--port 0`):
- churn WITH DELETE: 60 initialize+DELETE cycles: RSS 24,892 kB → 28,440 kB (+3.5 MB, no plateau observed within the run; not asserted monotone).
- churn WITHOUT DELETE: 60 initializations: RSS 25,292 kB → 31,964 kB; t+5.5 min idle: 29,652 kB; t+10.4 min idle: 29,652 kB (plateau ~+4.4 MB above baseline; new initialize still 200; all 60 sessions were created then went idle WITHOUT `notifications/initialized` — see caveat).
Caveat for the adversarial lane: keep_alive timer runs in the session worker event loop; whether idle reclamation fires for sessions that never completed the handshake is NOT proven by this sample. RSS plateau at +4.4 MB after 10 min ≈ not fully reclaimed within keep_alive+60s for THIS traffic shape (init-only, no initialized notification). The plan's PASS criterion ("RSS returns near baseline within keep_alive+60s") was NOT met within the observation window; drift then flattened. → refactorer: churn shape (init-only vs full session), longer window, and per-session memory floor.

### A2 — session-id entropy — PASS
- RMCP transport/common/server_side_http.rs:15–17: `pub fn session_id() -> SessionId { uuid::Uuid::new_v4().to_string().into() }` (quoted).
- rmcp Cargo.toml `[dependencies.uuid] features = ["v4"]` (verified via tomllib); Cargo.lock uuid 1.26.1 deps = getrandom 0.4.3 (`cargo tree -p uuid`: uuid v1.26.1 → getrandom v0.4.3); `cargo tree -p rmcp -e features` shows getrandom 0.4.3 with `sys_rng` on the rand path. OS CSPRNG-backed.
- Sweep: 200 concurrent initializes on one door → 200 collected, `sort -u` = 200, shape violations vs canonical v4 UUID regex = 0. Visible-ASCII MUST satisfied.
- Contrast (quoted in-repo): src/tools/jobs.rs new_id() = `rism-{pid:x}-{64-bit rand::random}` — session ids are the stronger of the two.

### A3 — Host guard + Origin gap — FINDING (confirms plan design-gap 1)
Measured, one ephemeral loopback door:
- `Host: evil.example` POST → 403 `Forbidden: Host header is not allowed` ✔ guard live.
- `Origin: https://evil.test` POST (loopback door) → **200**.
- `--host 127.0.0.1 --allow-all-interfaces` door (warning banner printed, verified in stderr): `Host: evil.example` → **200**; `Origin: https://evil.test` → 200.
Source: tower.rs:195 default `allowed_hosts: vec!["localhost","127.0.0.1","::1"]`; :214–216 `disable_allowed_hosts()` = `clear()`; :811–813 empty list ⇒ allow ALL; :932–936 `validate_origin_header` short-circuits `Ok(())` when `!validate_empty_origin_allowlist && allowed_origins.is_empty()`; our config sets neither (src/mcp/http.rs:149–158: only `.with_legacy_session_mode(true)` + conditional `disable_allowed_hosts()`; `enforce_origin_validation()` never called).
⇒ On ANY bind of this door, Origin is unvalidated. Spec Security Warning (plan quote): "Servers MUST validate the Origin header on all incoming connections to prevent DNS rebinding attacks." No browser cross-origin wall on a no-auth door that can run shell tools. Non-breaking fix: builder calls only. Verdict severity = refactorer/architect.

### A4 — DELETE termination — PASS (behavior), FINDING (test vacuity, confirmed)
Measured: initialize → sid; `notifications/initialized` → 202; `DELETE` w/ sid → **202** empty body; POST with the deleted sid → **404** `Not Found: Session not found`. Shipped test (tests/http_door_stateless.rs:136–142) only `.expect("delete answered")` — no status assert, no after-DELETE 404 probe (quoted). rmcp handle_delete → close_session → `accepted_response()` (202; server_side_http.rs:24–29). Spec: DELETE SHOULD from client, 405 allowed → 202 compliant.

### A5 — status matrix — PASS 3/4 + FINDING (plan's 400 expectation wrong for POST)
One door:
- malformed Accept (json only) → 406 ✔ (tower.rs:1716–1735 quote verified).
- GET without event-stream Accept → 406 ✔ (tower.rs:1578–1591).
- PUT → 405 + `allow: GET, POST, DELETE` ✔ (tower.rs:1547–1551 quote verified).
- bogus sid POST → 404 ✔ (tower.rs:1787 region quote verified).
- **missing-sid POST** (tools/list, ping, notifications — with/without id): **422** `Unexpected message, expect initialize request` — NOT 400. Cause: `is_legacy_request` (tower.rs:382–439) returns legacy=true for version-2025 bodies; the sid check at tower.rs:1622 belongs to the **GET** handler; the legacy-session POST branch (tower.rs:1766+) treats missing sid as "sessionless initialize" and fails message-shape with 422. GET/DELETE without sid DO answer 400 ✔ (measured, "Bad Request: Session ID is required").
FINDING (mcp-protocol, severity → verdict lane): spec says servers requiring a session id SHOULD answer 400 for missing id; rmcp answers 422 (and 422 is not in the spec's enumeration). Our door delegates this 100% to rmcp; repo has NO test pinning any 4xx of this matrix (grep cb51ecf tests: only 200/202 asserts). Plan's "missing session id → 400 tower.rs:1631" is right for GET/DELETE, wrong for POST — corrected here.

### A6 — ::1 bracket path + localhost ready URL — PASS
- `--host ::1 --port 0`: binds; ready line `http://[::1]:P/mcp`; initialize via that URL → **200** with session id; explicit `Host: [::1]:P` → 200 (`normalize_host` strips brackets — tower.rs:780–781 quote). Plan's "documents a bind it cannot serve" risk NOT reproduced.
- `--host localhost --port 0`: ready line prints `http://127.0.0.1:P/mcp` (local_addr form); connecting to it → 200; `Host: localhost:P` → 200. getent localhost → 127.0.0.1 on this box (IPv6-first environments remain THEORY).

## B. Bind policy

### B1 — resolve/bind TOCTOU — SOURCE-READ (confirmed shape), hosts-file demo NOT run
src/mcp/http.rs:43–54 quoted in plan is exact (line 44 `host == "localhost"` early-true; :50–52 `to_socket_addrs().is_ok_and(|mut addrs| !addrs.any(|a| !a.ip().is_loopback()))`). Bind at :106–107 re-resolves the NAME (`format!("{}:{}", host, port)`). Two resolutions, two moments — architecturally as the plan describes; not safely reproducible read-only (no /etc edits). Rust-quality note (blocking std getaddrinfo on the async path) is F3 below.
RUNNABLE sanity (measured): `--host localhost` starts+serves ✔; `--host 127.0.0.1` ✔; `--host 203.0.113.7` refuses via pinned test (cli_transport non_loopback_bind_without_optout_refuses ✔ ok in-suite).

### B2 — `--host ::1` end-to-end — PASS (see A6; bind string `::1:0` accepted by tokio's ToSocketAddrs path, no UX inconsistency).

### B3 — failure UX names the flag — PASS (source + test quoted: http.rs:66–72 contains `--allow-all-interfaces`, host, "NO authentication"; pinned by tests/cli_transport.rs:107–127).

### B4 — bind-before-ready-line window — FINDING (measured, plan's prediction confirmed)
Blackhole IRIS `RISM_IRIS_BASE_URL=http://10.255.255.1:8099`: listener present in `ss` at t+26 s while ready line had NOT printed (ready-line grep count 0); first curl attempt at that moment did not complete within its 2–3 s window (SYN/ACK accepted, request parked — hyper has an accept loop only after serve() polls; connection queued). Ready line printed t+30.2 s (negotiate connect timeout, matches settings.rs:97 timeout_secs 30). ⇒ socket accepts (backlog) BEFORE handler ready; ready line = "handler built" fence, not "now serving" fence. Any harness that gates on PORT OPEN instead of the ready line can race a dead-IRIS startup; mcp_probe gates on the ready line with 40×0.25 s = 10 s budget (packaging/mcp_probe.sh:166–171) → would time out on a >10 s slow IRIS. Docs say ephemeral port "printed on the stderr ready line" — honest about the mechanism, silent about the lag.

## C. Factory semantics

### C1 — factory call cadence — PASS (one negotiate)
Source (verified): tower.rs:1086 `call()` clones the *service struct* (cheap Arc'd config), not the factory; `get_service()` sites = :1323 (tool-schema cache), :1469 (session-store restore — dead: config.session_store None), :1857 (per-request-negotiated direct serve), :1890 (initialize→create session), :1983 (stateless per-request). Our factory = `move || Ok(handle.clone())` (http.rs:147), handle built once at http.rs:146.
Measured oracle: RUST_LOG=info door + live IRIS; 5 full initialize sessions + 10 inline stateless tools/list → `grep -c "negotiated Atelier"` = **1**. No renegotiation, no per-call ToolRouter rebuild (clone comment http.rs:140–143 honest; router clone cost note only).

### C2 — shared state across clones — PASS/SOURCE
RismMcp = {client: IrisClient, tool_router} (mod.rs:42–45); IrisClient = Arc<Inner> (src/iris/http.rs:12–15, "Cheap to clone (Arc)"). jobs registry = `static JOBS: OnceLock<Mutex<Registry>>` (src/tools/jobs.rs:134) + MAX_RUNNING=16 (jobs.rs:31) — process-wide across all clones ✔. Debug-session state: src/iris/dbgp.rs has NO statics/OnceLock (grep: only `pub struct DbgpConnection` at :34) — debugger state is per-connection object, none of it forked by RismMcp clone at the rmcp layer; whether two HTTP sessions can hold concurrent DBGP attaches to the SAME IrisClient is refactorer-territory → OPEN-FOR-ATTACK (dbgp concurrency across clones).

### C3 — dual door — SOURCE-READ answer: not shipped, single transport per invocation (main.rs:295–306 match, one arm); jobs registry static ⇒ a future dual-door shares the 16-cap correctly. Prism serve.py is also single-door (read: transport option picks one). No doc claims a dual door → design note only, per plan.

## D. Lifecycle

### D1 — SIGTERM — PASS (clean exit; background jobs die with the door)
- live-IRIS background job running: `kill -TERM` → exit code **0**, elapsed <1 s, stderr tail shows rmcp `quit_reason=Closed` then `INFO rism::mcp::http: rism MCP server stopped (http)`.
- orphan test: background job `sleep 12 && echo survived > marker`; SIGTERM at t+2 s; marker ABSENT at t+16 s ⇒ the child shell tree died with the door (jobs are tokio-spawned process groups; exit reaps them). No task left "running" in a reachable registry (process gone).
- Ctrl+C parity: same `signal_shutdown` select arms (http.rs:193–196) — source-read ✔; VM row D exercised Ctrl+C already.
- Note (source): `keepalive` arm (http.rs:173–179) holds the last `ct` clone — on shutdown, the graceful future cancels `ct` and serve exits; the dropped-token hazard the comment alludes to is plan-correct that tokio CancellationToken does not fire on drop; arm is defensive.

### D2 — ready-line race — measured under B4; ready line is printed AFTER `RismMcp::new` (http.rs:146 vs :167 — order quoted). THEORY judgment (print-before-build?) → verdict lane; behavior recorded.

### D3 — banner parity — FINDING (minor parity)
- HTTP door ready: `rism MCP server ready (http) - listening on http://127.0.0.1:P/mcp` via raw `eprintln!` (http.rs:167) — NO timestamp, NOT through tracing.
- stdio door: `tracing::info!("rism MCP server ready (stdio)")` (mod.rs:646) → timestamped line, no URL, and it appears only AFTER the first initialize completes (stderr capture of `rism mcp` with one initialize: INFO line ordering shows "ready (stdio)" after rmcp "Service initialized").
- stdout purity http door: process stdout empty in all our door runs ✔ (also pinned http_door.rs:198–201).
Both doors stderr-only ✔; format asymmetry = cosmetic/parity nit with evidence.

## E. Error / edge surface

### E1 — dead-IRIS over HTTP — PASS + protocol FINDING (no reachable task mode on this door's handshake path)
Door env `RISM_IRIS_BASE_URL=http://127.0.0.1:1`:
- startup fine (negotiate best-effort, src/iris/http.rs:195–210 quoted).
- session (2025-03-26) + `background=true` → **23 ms**: tool-level error naming "Tasks extension" (Rail-A, mod.rs:443–456) ✔ — matches mcp.md §11 rule.
- handshake initialize `2026-06-30` + tasks ext → server negotiates **2025-11-25** (measured, both stdio and http) → background=true then also gets Rail-A (no task wire shapes on an old-version session — SEP-2663 compat honored).
- inline SEP-2575 `_meta` (protocolVersion 2026-07-28 + clientInfo + clientCapabilities w/ tasks ext) + `MCP-Protocol-Version: 2026-07-28` header + `Mcp-Method`/`Mcp-Name` standard headers over HTTP: tools/call background=true → **resultType "task"** seed, `tasks/get` (correct headers: `Mcp-Name: <taskId>` for tasks/*) → terminal **failed** within one poll vs dead IRIS ✔ (answers, never hangs; -32020 on unknown task id ✔).
- FINDING shape discovered en route: the plan's E1 recipe ("inline `_meta` tasks declaration" with only `io.modelcontextprotocol/tasks` in `_meta`, or `_meta` version 2026-06-30) is NOT accepted by rmcp 3.4.1: `_meta` per-request versioning requires `_meta["io.modelcontextprotocol/protocolVersion"] >= 2026-07-28` AND `MCP-Protocol-Version: 2026-07-28` header AND SEP-2243 `Mcp-Method` (+`Mcp-Name`) headers; 2026-06-30 in `_meta` is rejected (measured 400 "-32020 request _meta protocolVersion requires MCP-Protocol-Version header" then version-validation wall). documentation/mcp.md §11 says exactly this for stdio ("2026-06-30 falls back to 2025-11-25; the compliant tasks path is the SEP-2575 INLINE lifecycle") — so the doc is consistent, the PLAN row was loose. On HTTP the inline lifecycle works ONLY with the full 2026-07-28 header set. Tasks-live over HTTP is therefore reachable — with caveats the spec-text lane should judge: every task request is stateless (no session), and `tasks/result` is NOT implemented (-32061/-32601 `Method not found` measured with correct headers — see E1b).
- E1b: `tasks/result` (SEP-2663 poll-completion method) → `{"error":{"code":-32601,"message":"tasks/result"}}` (measured, inline lifecycle, live IRIS, correct headers). Stdio path pinned by tests/background_jobs.rs? — refactorer: whether our handler intentionally omits tasks/result (rmcp would answer -32601 by default) vs implements it; docs promise "poll tasks/get" only — cite mcp.md instructions line (server instructions string says "poll tasks/get, stop tasks/cancel" — honest, no tasks/result promise). NOT a defect: the -32601 is an INTENTIONAL, already-pinned design decision — tests/background_jobs.rs:629 `legacy_tasks_result_answers_32601` ("SEP-2663 compat: the legacy `tasks/result` method is NOT a named route"). Removing from OPEN-FOR-ATTACK; recorded as PASS-with-cite.

### E2 — rejected-request sessions — PASS
Bogus-sid POST 404 / no-sid POST 422 runs above: no session was created for them (subsequent initialize behaves normally; A1 RSS shows no step per 4xx). rmcp creates sessions only at initialize (tower.rs:1890 quote).

### E3 — concurrency while a job streams — PASS (10 s live-IRIS job; 50×tools/list on a real session: max 41 ms, 0 wrong counts; 10×tasks/get: all "completed" (job finished ~8 s), zero deadlock; job output intact: streamed frames present in tasks/get content — the grep in my one-shot script mis-counted because the payload embeds `line40` inside an escaped JSON string (single-line "line1\nline2..." content block), re-run with `echo alpha; sleep 2; echo beta` shows the intact `line`-free content on a completed task; no data loss observed but content-shape of tasks/get vs a *finished* job → refactorer may want a byte-exact output pin).
`MutexGuard` across `.await` audit: jobs.rs `lock_reg()` returns the tuple guard (jobs.rs:141–159 quote) so guards drop at end of statement scope; full await-crossing proof is adversarial-lane work → OPEN-FOR-ATTACK (E3 plan row: prove no `.await` sits between lock and drop across every jobs.rs path).

## F. Rust quality

- F1 unwrap/expect: none in src/mcp/http.rs outside `#[cfg(test)]` (grep of :1–216 empty ✔). main.rs new `unreachable!("serve_mcp is only called for Commands::Mcp")` follows the existing precedent style (main.rs:60,123 `unreachable!("matched above")`) ✔ nit-consistent.
  Clippy re-run on this tree: `cargo clippy --all-targets -- -D warnings` exit 0, zero warnings (this lane, after all merges).
- F2 Error::Config(String): 28 uses repo-wide (count measured: grep -c sums to 28) — house-consistent; `is_user_error` keys off the VARIANT (error.rs) so the bind-refusal exit-code class is unchanged; the VALIDATION note that classification stays correct is consistent with cli_transport failure-path test ✔. Nit only, matches plan.
- F3 blocking getaddrinfo in async: confirmed — http.rs:51 `to_socket_addrs()` (std) inside async `serve_http` path. Cite documentation/mcp.md pitfall 2 (mcp.md:237–238 "Use tokio/async everywhere") + tokio `lookup_host` alternative. Minor (startup-only), SOURCE-READ PASS of the plan's claim. Note: "localhost" early-return (http.rs:44) means the DEFAULT config path never hits DNS; named-host users pay it once.
- F4: factory/service Arc discipline ✔ (http.rs:140–163, `serve_http_on` takes listener by value); `keepalive` naming nit confirmed (it's a token-holder arm, not a keep-alive).
- F5 test ports: `3111` NOT present anywhere in the tree (grep empty — plan's TASK.md row corrected); 127.0.0.1:39999 used 3× in tests/cli_transport.rs (connect-must-fail assertions at :53/:81 area — if a foreign listener squats 39999, two tests FAIL spuriously; flake channel REAL but only via squatting, not via our own doors — http tests bind :0 ✔; two http_door*.rs files run their non-ignored tests in parallel process/thread with :0 — no race). Flake sniff on port-sensitive files: cli_transport ×5 = all ok, ~0.5 s per invocation; http_door_stateless ×3 = all ok ~0.47 s. Zero flakes in-tree. Squat-theft experiment = adversarial lane (plan's `nc -lp 39999` loop).

## G. Parity / claims

- G1 bare `rism mcp` — **PASS, byte-diff done**: built 9a1ddd7 binary in this worktree (detached build) vs cb51ecf binary; same initialize line → stdout sha256 equal (405 B); extended session (init+initialized+tools/list) → `cmp` **byte-identical** (both exit 0). docs/cli-reference.md:153 "byte-identical to every released version" claim now has a measured byte-diff (suite alone only proves semantics — FINDING was test-design, now resolved by this probe). stdio dispatch diff 9a1ddd7..cb51ecf = pure refactor move of the logging block (git diff quoted: init_logging_stderr extraction) ✔.
- G2 Prism parity: `/mcp` ✔ (http.rs:164); port 3000 default ✔ (settings.rs:103 vs Prism serve.py:9 `DEFAULT_PORT = 3000`); alias map DIVERGENCE CONFIRMED: Prism accepts `sse` (serve.py:18 `"sse": "sse"` + help text line 35), we reject it (settings.rs:303 `parse_alias("sse") == None`; error names accepted set ✔). Our docs never promise sse (grep docs: zero hits) → comment at settings.rs:11–13 "Prism parity" oversells the alias map slightly — nit with cite, matches plan. `rism mcp --help` vs docs/cli-reference.md table: flags/env/defaults one-for-one (transport env RISM_MCP_TRANSPORT ✔, port env ✔, `--host` NO env tier — verified cli/mod.rs:121–124: `--host` has no `env=`) ✔; configuration.md tables list exactly the two env vars ✔ (asymmetry port-vs-host = nit confirmed: host has config.toml tier but no env tier).
- G3 size claim — **PASS exact**: `cargo build --release` at this tip (18fd1c9 ≡ cb51ecf product) → **11,115,648 B**, matching VALIDATION.md:42's 11,115,648 B byte-for-byte (+18.5% vs base 9,378,168 B arithmetic in VALIDATION). Reproducibility note: this worktree only; profile per Cargo.toml [profile.release].
- G4 probe honesty: replay under env -i: `MCP probe OK: HTTP door answered (25 tools, SQL clean), SIGTERM exit 0` — and the echo IS vacuous as the plan says: probe ends `kill -TERM $http_pid; wait $http_pid 2>/dev/null || true` (mcp_probe.sh ~:201–202 quoted) — exit status swallowed by `|| true` before the "exit 0" claim prints. FINDING (area: tests, minor) CONFIRMED at read level; the panic-door proof is adversarial-lane (worktree-only patch). The tools/list count leg works on the compact one-line SSE frame (counted 25 in the live replay ✔).
- G5 docs: ready-line format claim ✔ (measured string identical); "Works with Claude Desktop/Cursor" getting-started — unverifiable here (nit, no evidence either way); no `RISM_MCP_HOST` env row in docs — matches code ✔.

## Cargo test + gate tails
- `cargo test -q` (whole tree, non-ignored): 73+13+6+1 passed, 17+1 ignored, 0 failed (lib 73, doc 0, integration shards incl. cli_transport 6, http_door_stateless 1, background 18-2? per-shard tails above; all green).
- `cargo test --test http_door -- --ignored` (live IRIS up): http_door_live_binary_roundtrip ok, 0.11 s.
- `packaging/mcp_probe.sh`: all four OK lines, HTTP leg included (tail quoted above).
- Clippy `--all-targets -D warnings`: exit 0, silent. `cargo fmt --check`: clean.

## Process hygiene
Every door/server spawned by this lane was killed by PID (scripts reap; final `pgrep -x rism` for MY scripts' pids empty). NOTE (not mine): 7 stray `rism mcp --transport http --port 0` processes owned the coder worktree path started 01:24–01:43 today from a DELETED cwd (`/proc/PID/cwd` = worktree "(deleted)") — left untouched per "kill every process YOU spawn"; flagging to the coordinator for cleanup.

## OPEN-FOR-ATTACK (handed to refactorer)
1. A1: idle-GC under FULL sessions (init+initialized+one call) churned without DELETE; prove map shrinks past keep_alive OR show retained-entry growth. (My init-only sample showed a +4.4 MB plateau.)
2. C2/E3: jobs.rs `MutexGuard` vs `.await` exhaustive audit; dbgp multi-session attach coherence across clones.
3. ~~E1b tasks/result~~ — RESOLVED during drafting: intentional -32601, pinned by tests/background_jobs.rs:629; only the HTTP-side equivalence of that pin needs a glance.
4. F5: 39999 squat-flake experiment; G4: panic-on-SIGTERM door vs mcp_probe vacuous assert.
5. B1: any read-only hosts-file/DNS demo (I judged not-safely-runnable; cite suffices).
6. D2/B4: judgment on ready-line-before-handler-build ordering.
7. A5: whether the 422 (vs spec SHOULD-400) on missing-sid POST warrants an rmcp issue-link note in docs.
