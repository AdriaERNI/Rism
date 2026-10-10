# R1 REVIEW PLAN — specifier lane (questions A–G → code + authority + probe)

Head audited: `cb51ecf` (feat/windows-mcp-http, PR #18), diff base `9a1ddd7`.
This lane ran NO cargo and probes NOTHING — every probe below is written for
the coder (VERIFICATION) and refactorer (ADVERSARIAL) lanes. Classifications:
**RUNNABLE** (cheap command, record measured fact), **SOURCE-READ** (answer is
static; cite the line), **THEORY** (adversarial/race/leak work).

RMCP = `/home/hermes/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-3.4.1/src`
SPEC = MCP spec 2025-06-18 §Transports (Streamable HTTP), fetched and quoted
below; upstream 2026-07-28 text is identical for the MUSTs cited
(rmcp's own doc-comment links the 2026-07-28 URL — RMCP/transport/streamable_http_server/tower.rs:965).

Line references were taken with `grep -n` against cb51ecf files; if a coder's
file differs, re-anchor by the quoted code string, not the number.

---

## A. Session / security model

### A1. LocalSessionManager growth bound + idle GC (leak under churn)
- Code: `src/mcp/http.rs:163` (`Arc::new(LocalSessionManager::default())`) —
  WE pass the stock default config; no cap, no custom SessionConfig.
- Authority (source-read):
  - RMCP/transport/streamable_http_server/session/local.rs:34 — sessions =
    plain `RwLock<HashMap<SessionId, LocalSessionHandle>>`. **No size cap.**
  - local.rs:67 — every initialize INSERTS; local.rs:84–85 — removal happens
    ONLY via `close_session` (DELETE at tower.rs:2101, or worker exit).
  - local.rs:1253/1263 — `keep_alive = Some(300s)` DEFAULT: an idle *worker*
    exits with `IdleTimeout` (local.rs:1125–1126) and
    tower.rs:1334–1371 `spawn_session_worker` then calls `close_session` →
    the map entry IS reclaimed ~5 min after last activity.
  - local.rs:489–492 — `evict_expired_channels` reaps per-session completed
    channel caches by `completed_cache_ttl` (60 s, local.rs:1254/1265), but
    it runs ONLY inside the worker event loop (local.rs:1109) — a session
    that goes fully idle evicts nothing until its keep-alive kills it.
  - Spec gap check: no MCP-spec MUST bounds sessions; the risk is DoS-shaped
    memory on an auth-less door.
- Verdict criterion: not a leak *per se* if A2's churn probe shows the map
  shrinking. **RUNNABLE (adversarial)**: churn script — N parallel initialize
  (no DELETE), then `curl` side-channel impossible → measure RSS of the door
  process (VmRSS from /proc/PID/status) at t0, t+5min idle; PASS = RSS returns
  near baseline within keep_alive+60s ± scheduler slop; FAIL = monotone RSS.
  Second variant: churn WITH immediate DELETE — RSS must not grow at all.

### A2. Session-id entropy (SEP-2663 Security: ids double as bearer tokens)
- Code: nothing in our diff — session ids come entirely from rmcp.
- Authority: RMCP/transport/common/server_side_http.rs:16–18 —
  `session_id() = uuid::Uuid::new_v4()`. rmcp's uuid dep enables the `v4`
  feature (RMCP Cargo.toml `[dependencies.uuid] features=["v4"]`), and our
  Cargo.lock pins uuid 1.26.1 → getrandom 0.4.3 (Cargo.lock:2381–2388, 679+),
  i.e. OS CSPRNG. v4 UUIDs = 122 random bits. Visible-ASCII MUST: canonical
  hyphenated hex satisfies it (spec §Streamable HTTP: "MUST only contain
  visible ASCII (0x21–0x7E)").
- Contrast pinned in-repo: task ids use `rand::random::<u64>()` + pid
  (`src/tools/jobs.rs:180–189`, documented in documentation/mcp.md §11
  "Task ids carry entropy"). Session ids are stronger than task ids — fine.
- Criterion: **SOURCE-READ** — PASS if the shipped Cargo.lock resolves the
  rmcp uuid feature to a getrandom-backed build (verify with
  `cargo tree -p rmcp -e features` in coder worktree, RUNNABLE) and no code
  path overrides `session_id()`. Adversarial: collect ≥200 issued ids,
  `sort -u` count = 200, check shape /^[0-9a-f]{8}-…/ (RUNNABLE).

### A3. Host-header guard: which rmcp field, coherent with legacy_session_mode?
- Code: `src/mcp/http.rs:149–158` — config default kept on loopback binds;
  `.disable_allowed_hosts()` ONLY under `allow_all_interfaces`.
- Authority: tower.rs:195 — default `allowed_hosts = ["localhost","127.0.0.1","::1"]`;
  tower.rs:214–216 — disable = `clear()`; tower.rs:811–814 —
  **empty list ⇒ allow ALL hosts**. Enforced at tower.rs:1541
  `validate_dns_rebinding_headers` → :915–930 → 403 (forbidden_response).
  The guard is orthogonal to `legacy_session_mode` (that field only gates
  session/GET/DELETE handling: tower.rs:1547–1557).
- Coherence findings to judge:
  - Host matching is exact-string (tower.rs:819–823 host equality, port
    optional). Binding `localhost` on a machine whose `localhost` resolves
    elsewhere still ACCEPTS `Host: localhost` — the guard validates the
    header text, not the bind (matches spec's DNS-rebinding intent; rmcp behavior, cite only).
  - **OUR GAP CANDIDATE**: with `--allow-all-interfaces` we call
    `disable_allowed_hosts()` (hosts allow-all) AND never call
    `enforce_origin_validation()` (RMCP tower.rs:225–229). With empty
    `allowed_origins` and `validate_empty_origin_allowlist=false`
    (tower.rs:197), `validate_origin_header` (tower.rs:932–963) returns Ok
    for ANY Origin. Spec Security Warning (fetched text): "Servers MUST
    validate the Origin header on all incoming connections to prevent DNS
    rebinding attacks." So on a LAN-exposed, no-auth door, a browser page
    can drive it cross-origin (no Private Network Access check in rmcp
    either). Verdict lane decides severity; label any fix
    non-breaking (config-only, `enforce_origin_validation()` + host list).
- **RUNNABLE**: loopback bind + `curl -H 'Host: evil.example'` → expect 403;
  same with `--host 0.0.0.0 --allow-all-interfaces` → expect 200 (documents
  the guard is gone). Origin bypass: `curl -H 'Origin: https://evil.test'`
  against BOTH binds → if both 200, spec-MUST is unsatisfied at the rmcp
  layer (report; not our code).

### A4. DELETE termination actually supported?
- Code: nothing — rmcp. `legacy_session_mode(true)` at `src/mcp/http.rs:152`.
- Authority: tower.rs:1557 (DELETE only handled when legacy mode true),
  :2076–2111 handle_delete → close_session → 202 (accepted_response,
  RMCP/transport/common/server_side_http.rs:24–29). Spec: clients SHOULD
  send DELETE; server MAY answer 405 (so 202 is compliant and generous).
- Shipped test gap (cite!): `tests/http_door_stateless.rs:136–142` sends the
  DELETE but only `.expect("delete answered")` — it does **not** assert 202
  status, and the comment says "then the id must be dead" but NO follow-up
  request asserts the 404. **THEORY→RUNNABLE (coder)**: POST after DELETE
  with same sid → expect 404; DELETE status = 202. PASS = both. This is a
  test-vacuity finding candidate (area: tests).

### A5. 406/404/400/405 semantics spec-correct?
- Authority (our door delegates 100% to rmcp):
  - POST missing either Accept type → 406 tower.rs:1716–1735 (spec: client
    MUST include Accept with both; server 406 acceptable).
  - GET needs text/event-stream in Accept → 406 tower.rs:1578–1591.
  - Missing session id (post-init) → **400** tower.rs:1631 / :2094; spec:
    "Servers that require a session ID SHOULD respond … with HTTP 400".
    rmcp's inline comment claims SHOULD-400 ✔.
  - Unknown/terminated session id → 404 tower.rs:1651 / :1787; spec MUST ✔.
  - Other methods → 405 + Allow header ("GET, POST, DELETE" under legacy,
    tower.rs:1547–1551) ✔.
- **RUNNABLE pin** (none of these four are tested anywhere in the repo —
  grep showed no 406/404/400-status asserts in tests/http_door*.rs): a small
  matrix script: malformed Accept → 406; POST without sid → 400; POST with
  bogus sid → 404; PUT/PATCH → 405. All from ONE spawned door. PASS = 4/4.
  Missing pins = findings (area: tests, minor).

### A6. Host-header guard vs ephemeral port + `--host localhost` (design gap flag)
- Code: `src/mcp/http.rs:106–108` binds via `format!("{host}:{port}")`
  (`tokio::TcpListener::bind` → `ToSocketAddrs`), but rmcp's allow-list is
  static `["localhost","127.0.0.1","::1"]` (tower.rs:195) and
  `parse_host_header` normalizes host+port, port optional match
  (tower.rs:819–823).
- Consequence to verify **RUNNABLE**: `rism mcp --transport http
  --host localhost --port 0` → binds (probably) 127.0.0.1:P; requests with
  `Host: localhost:P` allowed; BUT a client using the ready-line URL
  `http://127.0.0.1:P/mcp` is also allowed (in list) ✔. Now `--host ::1`
  ready-line prints `[::1]:P` — `Host: [::1]:P` — does `normalize_authority`
  (tower.rs:791–797) strip brackets so "::1" equals the allowed "::1"?
  **Probe**: initialize against an `--host ::1` door with `Host: [::1]:P` →
  200 vs 403. If 403: the door binds fine but is unusable via its own
  printed URL — real usability finding (area: mcp-protocol/rust, major if
  reproduced; our code neither configures nor documents the interaction).
- Same class: `--host localhost` on a machine where getaddrinfo returns
  `::1` first → binds IPv6 only; a client hitting `http://127.0.0.1:P` gets
  connection-refused while `Host` check is moot. THEORY (environment-dependent), SOURCE-READ for the bracket case.

---

## B. Bind policy correctness

### B1. is_loopback_host TOCTOU / DNS rebinding at check time
- Code: `src/mcp/http.rs:43–54` — `host.parse::<IpAddr>()` else
  `to_socket_addrs()`; "no non-loopback address in the set" ⇒ loopback.
  Then `src/mcp/http.rs:106` re-resolves the SAME name for the actual bind.
- Two resolvers, two moments: the gate uses **blocking std**
  `to_socket_addrs` (getaddrinfo) on the async server thread (Rust-quality
  note: a multi-hundred-ms DNS stall blocks the runtime; minor), and the
  bind re-resolves — an attacker controlling DNS between the two answers
  (TTL-0 record) can pass the check with a loopback A record and get bound
  with a routable one. Also: the check accepts when a name returns BOTH
  loopback and non-loopback? No — `!addrs.any(|a| !a.ip().is_loopback())`
  requires ALL loopback, but the BIND may pick only one address
  (`bind("host:port")` binds the FIRST resolved address per tokio's
  ToSocketAddrs semantics; `TcpListener::bind` is single-addr).
  **THEORY/SOURCE-READ**: no realistic local attacker needs DNS; still, the
  mitigation the spec names ("bind only to localhost rather than
  all network interfaces", Security Warning) is undermined for *named*
  hosts. Recommend (verdict lane, non-breaking): resolve once, then bind
  the *checked* IP (or bind-by-literal only).
- **RUNNABLE**: on the probe box, `--host localhost` works, `--host 127.0.0.1`
  works, `--host 0.0.0.0` refused (already covered by
  `tests/cli_transport.rs:117–127` with 203.0.113.7), and document the
  race as THEORY with a hosts-file demo ONLY if the adversarial lane can do
  it read-only in its own worktree (no /etc changes — flag as not-runnable
  safely; source citation suffices).

### B2. The ::1 path
- Code: same gate: `"::1".parse::<IpAddr>().is_loopback()` = true ✔
  (`src/mcp/http.rs:225` unit-pinned). Bind `format!("[::1]:{port}")`? NO —
  http.rs:106 formats `"[::1]:3000"`? It formats `format!("{}:{}", "::1", port)`
  = `"::1:3000"` which **is an invalid SocketAddr string** → bind error?
  **RUNNABLE (coder, cheap, potentially major)**:
  `rism mcp --transport http --host ::1 --port 0` → does it start, print a
  ready line, and answer `[::1]:P/mcp` initialize? Note tokio's bind takes
  ToSocketAddrs: `"::1:3000"` may still parse via getaddrinfo (it does —
  it's a valid "host:service" form? ambiguous for IPv6) — MEASURE, don't
  assume. Also `display_addr` (http.rs:213) brackets correctly in the ready
  line. If bind fails while the policy said "loopback OK": UX inconsistency
  worth a row. Cross-check rmcp guard: see A6 probe.

### B3. Failure UX names the flag
- Code: `src/mcp/http.rs:66–71` Error::Config text: contains
  `--allow-all-interfaces` + host + "NO authentication". Pinned by
  `tests/cli_transport.rs:117–127` (203.0.113.7). ✔ SOURCE-READ PASS.

### B4. Bind-before-vs-after negotiate
- Code: `src/mcp/http.rs:103–122` — policy check → bind → (banner) →
  `serve_http_on` → `RismMcp::new` (negotiate) at http.rs:146 → then axum
  serve. So: address-in-use fails fast with NO IRIS contact ✔ (comment at
  http.rs:104–105). One wrinkle: `serve_http` binds AFTER `init_logging_stderr`
  but the ready line (http.rs:167) prints only after `RismMcp::new` — a
  dead/slow IRIS delays the ready line while the socket ALREADY accepts
  (backlog queues). **RUNNABLE**: with `RISM_IRIS_BASE_URL` pointed at a
  blackhole IP (e.g. `http://10.255.255.1:8099` — TCP SYN with reqwest
  connect timeout = settings.timeout_secs 30 s default,
  src/settings.rs:97; http.rs:67–68 uses it), spawn door; poll the port for
  accepts during the 30 s window; if connections are accepted (SYN/ACK +
  hang) before the ready line, probe/test harnesses that wait on the ready
  line are fine but mcp_probe's 10 s budget (packaging/mcp_probe.sh:166–171,
  40×0.25 s) could time out on a slow IRIS → row for docs vs behavior (G).
  Note: localhost:1 refuses instantly (stateless test relies on this,
  tests/http_door_stateless.rs:99 comment "instant-refused").

---

## C. Factory semantics

### C1. How often does rmcp call service_factory?
- Authority: tower.rs:1086–1090 `call()` clones the service per HTTP request;
  `get_service()` sites: tower.rs:1323 (tool_schema cache miss — NOT per
  request), :1469 (session-store restore — config.session_store is None,
  dead path), :1857 (per-request-negotiated direct serve), :1890
  (initialize → new session), :1983 (stateless per-request). Under
  `legacy_session_mode(true)` + session_store None, the live paths are
  :1890 (once per initialize) and :1983 only for inline `_meta`-versioned
  requests (stateless lifecycle!). **SOURCE-READ answer: per session for
  handshake traffic; per request for SEP-2575 inline requests** — the
  "per request?" half of TASK.md C is YES for the inline lifecycle, which
  matters because RismMcp::new is NOT in the factory (clone only,
  http.rs:147). ✔ no renegotiation: factory = `move || Ok(handle.clone())`,
  handle built once at http.rs:146.
- Clone cost: RismMcp = IrisClient(Arc<Inner>, src/iris/http.rs:12–15) +
  ToolRouter whose Clone rebuilds HashMaps of Arc'd routes
  (RMCP/handler/server/router/tool.rs:325–333, 361) — shallow but allocates
  ~25 entries per call. Fine; note only.
- **RUNNABLE**: with RUST_LOG=info + a live IRIS, N=10 inlines / 5 sessions
  → grep stderr for "negotiated Atelier API v" occurrences. PASS = exactly 1
  (handle built once). FAIL if >1. This is the sharpest C-oracle cheaply.

### C2. Shared state across handler instances
- jobs registry is `static JOBS: OnceLock<Mutex<Registry>>`
  (src/tools/jobs.rs:134–137) + MAX_RUNNING=16 (jobs.rs:31) — process-wide,
  so every clone shares it ✔ (tasks/get from session B can see session A's
  task: intended? SEP-2663 says task ids are bearer tokens; cross-session
  visibility is the documented model, cite mcp.md §11).
- Debug-session state: `src/iris/dbgp.rs` — verify (SOURCE-READ for
  refactorer) whether an attached debugger session is per-RismMcp instance
  (would fork state across clones) or keyed globally. TASK.md C names it:
  check fields that `#[derive(Clone)]` (src/mcp/mod.rs:42) deep-copies vs
  Arc-shares — RismMcp has exactly 2 fields; if IrisClient::Inner holds a
  HashMap of debug connections behind Mutex-in-Arc, coherence ✔.

### C3. Dual door (stdio + http) in ONE process
- Code: `src/main.rs:266–306` — one transport per invocation; no path
  serves both. **SOURCE-READ answer to "possible/should":** not shipped; the
  jobs registry is static so even a future dual-door shares the 16-cap
  correctly; the only split state would be rmcp session maps. Design-gap
  flag ONLY if docs claim parity Prism has (Prism serve.py is single-door
  too — checked: ~/Projects/ERNI/Prism/src/prism/cli/commands/serve.py).

---

## D. Lifecycle

### D1. SIGTERM/Ctrl+C wiring — graceful or instant drop?
- Code: `src/mcp/http.rs:149–153` fresh `CancellationToken`; :160 `ct` clone;
  :172–179 `select!` — shutdown future awaits signal, then
  `with_graceful_shutdown(async { shutdown.await; ct.cancel(); })`; the
  `keepalive` arm exists only so `ct` isn't dropped (rmcp drops the token's
  last clone → cancelled() fires? No: tokio_util CancellationToken's
  `cancelled()` on a dropped parent token does NOT fire; the arm prevents a
  dropped-token warning — judge necessity, minor).
- Ordering fact to cite: axum's `with_graceful_shutdown` stops accepting
  NEW connections when its future resolves; in-flight sessions are killed
  when serve returns + futures drop (rmcp per-stream child tokens derive
  from OUR ct at tower.rs:1165 etc. — cancelled only AFTER the signal
  future resolves, i.e. before axum completes shutdown ✔ correct order:
  cancel sessions, then exit).
- **RUNNABLE (coder)**: SIGTERM mid-`tools/call` (long SQL against docker
  IRIS, e.g. `SLEEP`-style wait via `execute_command background=true` then
  SIGTERM): observe (a) process exit code, (b) whether the running
  background JOB is interrupted (jobs.rs cancel path) or orphaned — the
  registry is in-process, so exit ends the tail; is a task left "running"
  in nobody's books? (tasks/get after death = 404/connection-refused: fine,
  but docs/SPEC claim "SIGTERM-quit" honesty is only "exits", not "drains").
  FAIL criterion candidate: exit code != 0 on SIGTERM, or hang > timeout_secs.
  Ctrl+C parity probe same shape.

### D2. Ready-line race (port 0 printed before accepts)
- Code: ready line printed at http.rs:167–170; `axum::serve(listener,…)` is
  constructed at :175 — but the LISTENER was already `bind()`-ed at :107;
  Linux accepts into the backlog from bind() even before `listen()`… tokio's
  bind DOES call listen immediately. So connections succeed BEFORE the
  ready line if built after RismMcp::new's negotiate (see B4) — the printed
  line is NOT a "now accepting" fence, it's "socket listening + handler
  built". Race window = between bind and eprintln (the 30 s blackhole case
  in B4). Tests don't care (ready-line-gated). **THEORY**: adversarial lane
  probes the window (connect-before-ready-line) and judges whether printing
  the ready line BEFORE handler build would be more honest — a doc/UX row at
  most (the socket is genuinely listening; requests just queue in
  handshake/serve tasks).

### D3. stdout purity + log-banner parity vs stdio door
- Code: http door: `init_logging_stderr()` (http.rs:101 → mod.rs:622–634,
  RUST_LOG wins, default info, stderr, no ansi) + two eprintln!s
  (http.rs:112 warning, :167 ready) — all stderr ✔. stdio door logs
  "rism MCP server ready (stdio)" via `tracing::info!` (mod.rs:646) but the
  HTTP door logs readiness via raw `eprintln!` (http.rs:167) — banner
  FORMAT asymmetry (one timestamped by tracing's formatter, one bare); also
  the stdio door emits NO ready line, the http door does. Parity claim in
  docs (G) vs measured text. **RUNNABLE**: capture both doors' stderr, diff
  the ready/banner lines; assert door stdout empty under http (tests already
  do, http_door.rs:198–201 with `child.kill()` — a SIGKILL; the graceful
  path's stdout purity is D1-probe territory).

---

## E. Error / edge surface

### E1. Dead-IRIS: every tool ANSWERS over HTTP, incl. execute_command background=true
- Code: negotiate best-effort (src/iris/http.rs:196–210 — errors swallowed);
  tools map errors to isError:true (e.g. mod.rs:76–79); task-mode path
  mod.rs:436–496 starts jobs against the dead client — `jobs::start`
  (src/tools/jobs.rs:197) spawns a tokio task doing terminal I/O; its
  failure must surface via tasks/get as `failed` (tasks.rs status mapping
  lines ~85–105) not a hang.
- Shipped coverage: `tests/http_door_stateless.rs` covers dead-IRIS
  execute_sql (line 131–134) but NOT background=true over HTTP.
  **RUNNABLE (coder)**: against base_url http://127.0.0.1:1 door, one
  session, `tools/call execute_command {command:"x", background:true}` with
  the INLINE `_meta` tasks declaration (documentation/mcp.md §11 line
  ~372–384: handshake sessions can never be tasks-live on rmcp 3.4.1 —
  `2026-06-30` falls back to `2025-11-25`, so a plain 2025-03-26
  initialize session MUST get Rail-A refusal text, not a task); then
  assert: (a) non-declaring session + background=true → tool-level ERROR
  naming "Tasks extension" (Rail A, mod.rs:443–456); (b) inline-declared →
  task seed returned, then `tasks/get` reaches terminal `failed`/`completed`
  within timeout_secs + slack. PASS = answers, never hang.

### E2. Unknown session id → 404; malformed Accept → 406; no-sid → 400
- Covered by A5 matrix probes (RUNNABLE). Add the E-specific row: `tools/list`
  with a session that was only ever used for a REJECTED request (400/406) —
  no session should exist (rmcp creates at initialize only, tower.rs:1890).

### E3. Concurrent list/call while a background task runs
- Registry ops take `Mutex` (std::sync::Mutex, jobs.rs:146–149, 251, 318)
  **inside async fn?** — check lock-across-await discipline: SOURCE-READ
  jobs.rs:197–260 — if a `std::sync::MutexGuard` is held across `.await`
  that's a genuine Rust-quality finding (blocking lock in async). The
  `_oplock`+guard tuple return (jobs.rs:157–158) suggests guards escaped
  functions (drop at return = no await held?) — adversarial lane: prove with
  `tokio_clippy`-style read of every `.await` between lock and drop.
- **RUNNABLE**: while a background job streams (docker IRIS, `execute_command
  background=true` inline task), hammer 50× tools/list + 10× tasks/get
  concurrently; PASS = all answer < 2 s, job output intact, no deadlock.

---

## F. Rust quality of the new code

F1. **unwrap/expect policy** — workspace denies
  `unwrap_used`/`expect_used` (Cargo.toml:56–57). New code: `http.rs` has
  none outside tests (grep: only `#[cfg(test)]` asserts); `main.rs:274`
  `unreachable!` after an irrefutable-let guard on the same enum (exits
  only via panic-if-refactored; house rule check: other dispatch sites use
  `unreachable!("matched above")` main.rs:60,123 — consistent precedent,
  nit at most). Criterion: `cargo clippy --all-targets -- -D warnings` 0
  (VALIDATION.md already logs a run — coder re-run only if their tree
  diverges). **SOURCE-READ/RUNNABLE-cheap**.

F2. **Error::Config(String) stringly-typed** (src/error.rs:85–87) vs §5's
  precise-variants rule (documentation/rust-bestpractices.md §5,
  rust-bestpractices.md:222–229) — the new bind-refusal + transport errors
  (http.rs:66, 84–92) chose the string variant. Existing precedent count:
  grep `Error::Config` 28 uses — so it's house-consistent; log as nit
  unless a machine-readable consumer exists (installer exit codes doc?
  `Error::is_user_error` error.rs:104 keys off the variant — meaning the
  string IS load-bearing for exit-code mapping; verify classification is
  still correct: bind-refusal should be user error ✔).

F3. **Blocking getaddrinfo on async runtime** — http.rs:51
  `to_socket_addrs` (std, synchronous DNS) called from `check_bind_policy`
  inside an async fn's poll (serve_http). Rust best practice:
  `tokio::net::lookup_host`. Stalls the worker up to resolver timeout on a
  slow DNS. Severity: minor (startup only, single call). SOURCE-READ ✔
  (line 51). Probe: not worth it; cite rule
  (tokio docs / house §8 "Blocking calls in handlers" mirrors
  documentation/mcp.md §8 pitfall 2, line ~240: "Use tokio/async
  everywhere").

F4. **Arc/Clone discipline + lock patterns** — factory clone (C1) ✔;
  `serve_http_on` takes listener by value ✔; `LocalSessionManager` behind
  Arc ✔ (required by tower.rs:1135–1146). select! arm naming `keepalive`
  (http.rs:173) is misleading (it's the cancellation token, not keep-alive)
  — nit, naming only.

F5. **Test design — fixed ports**: `tests/cli_transport.rs:53,81` uses
  127.0.0.1:39999 — connect-after-failure assertion: if ANY other process
  (or another cargo test binary running http_door concurrently!) listens
  there, the test fails spuriously → CI flake risk. Also two http_door
  tests run in parallel threads but bind :0 ✔ (no ephemeral-port race —
  port 0 never collides; the race TASK.md posits doesn't apply, cite
  http_door_stateless.rs:103–106). TASK.md's "3111" does not exist in the
  tree (grep empty) — record as not-applicable. Criterion **RUNNABLE**:
  `while true; do nc -lp 39999 >/dev/null; done` in one shell →
  `cargo test --test cli_transport` must FAIL → proves the flake channel
  (adversarial lane; safe, own worktree).

F6. **Cancellation safety across .await** — serve_http_on holds no locks;
  `RismMcp::new(settings).await?` (http.rs:146) is drop-safe. rmcp child
  tokens per stream (tower.rs:1165+). PASS unless adversarial lane finds a
  spawn without token linkage: `spawn_session_worker` (tower.rs:1350)
  spawns per-session tasks detached from OUR ct until close — cancel
  ordering covered in D1. SOURCE-READ.

---

## G. Parity / claims audit

G1. **Bare `rism mcp` untouched** — dispatch: main.rs:36–40 → serve_mcp →
  Stdio arm (main.rs:289–293) calls `rism::mcp::serve` (mod.rs:637) — same
  body as 9a1ddd7 except logging refactor to `init_logging_stderr` (mod.rs
  622–634, identical filter/writer/ansi as the old inline block — diff
  shows pure move) ✔ SOURCE-READ. Env that could change default:
  `RISM_MCP_TRANSPORT` ONLY affects `rism mcp` flags (cli/mod.rs:115),
  config.toml `mcp_transport` default "stdio" (settings.rs:102).
  **RUNNABLE**: `env -i … rism mcp </dev/null` byte-diff stdout vs 9a1ddd7
  binary (SPEC row C claims this — verify a byte-diff was actually done,
  not just "answers JSON-RPC" as tests/cli_transport.rs:89–103 does →
  "byte-identical" is a DOC claim (docs/cli-reference.md:153
  "byte-identical to every released version") the suite only proves
  semantically → test-design finding candidate: pin claim strength).

G2. **Prism parity** — path `/mcp` ✔ (http.rs:164 nest_service "/mcp" vs
  Prism serve.py:48 `http://localhost:PORT/mcp`); port 3000 ✔
  (settings.rs:103 vs serve.py:9 DEFAULT_PORT=3000); alias map ✔ — ours
  (settings.rs:27–36) accepts stdio/http/streamable-http/streamable_http;
  Prism's ALSO accepts `sse` (serve.py:12–16 shows http/streamable-http/
  streamable_http; confirm whether Prism's transport help lists 'sse'
  — serve.py:35 says "stdio, or 'sse'" → our docs claim "aliases" parity;
  we REJECT sse (settings.rs test ~line 300 'sse' → None) — that's a
  documented divergence TASK.md G wants: honest? docs never promise sse ✔,
  but the "Prism parity" comment at settings.rs:12–14 oversells if it
  implies the full alias map. Nit with cite.
  **RUNNABLE**: `rism mcp --help` text vs docs/cli-reference.md table —
  flag names/defaults one-for-one (docs say env `RISM_MCP_TRANSPORT` ✔
  cli/mod.rs:115; docs say `--host` default 127.0.0.1 ✔).

G3. **+18.5% size claim** — testing-notes/VALIDATION.md:42: chain-tip
  11,115,648 B at abf02c2 vs base 9,378,168 B (coder ledger +17.7%). Two
  deltas for "the same" diff (04bc98c vs abf02c2 — abf02c2 ADDS
  testing-notes SPEC/VALIDATION commits = same binary; the 0.07 MB gap is
  build variance per the note). **RUNNABLE (coder)**: `cargo build
  --release` at cb51ecf in own worktree, measure bytes; PASS = claim within
  ±1%; note reproducibility caveats (LTO/codegen settings — cite Cargo.toml
  profile). If cb51ecf binary differs from abf02c2 by more than variance
  (the two follow-up commits ab59614..cb51ecf are the door itself + hyphen
  fix), the VALIDATION ledger may be stale — re-measure, don't re-argue.

G4. **mcp_probe HTTP-leg honesty** — packaging/mcp_probe.sh:163–203.
  Weak spots to judge (RUNNABLE by reading + one replay):
  (a) `tools/list` counted via `grep -o '"inputSchema"'` over an SSE body —
  counts 25 only if the response fits ONE cat of the stream: if rmcp frames
  each tool with `"inputSchema"` keys split across lines (pretty vs compact
  JSON — rmcp serializes compact; compact one-line data: frame ⇒ count
  works; but a 2-frame SSE still sums correctly ✔);
  (b) the leg NEVER asserts initialize's negotiated protocolVersion — fine,
  (c) NEVER sends/needs MCP-Protocol-Version header on follow-ups — rmcp
  tolerates absence (tower.rs:285+ only validates when present) — so the
  probe wouldn't notice a header-contract regression: nit,
  (d) SIGTERM leg: `wait $http_pid 2>/dev/null || true` (probe line 201) —
  **exit status of the door is NOT asserted** despite the echo claiming
  "SIGTERM exit 0"! Finding: probe honesty (area: tests, minor) —
  `kill -TERM` then `wait` result is swallowed by `|| true`. FAIL criterion
  = echo vs code mismatch, provable by making the door panic on SIGTERM in
  the adversarial lane (worktree-only patch, no repo change) → probe still
  prints OK ⇒ vacuous assertion.

G5. **Docs claims vs behavior** — docs/getting-started.md (~line 113):
  "Works with Claude Desktop/Cursor" — plausibility claim, unverifiable
  here (nit; no citation). docs/cli-reference.md ready-line format claim
  "actual port printed on the stderr ready line" ✔ http.rs:167.
  configuration.md env table ✔ (no `RISM_MCP_HOST` env row while
  `--host` has NO env= (cli/mod.rs:121–124) — docs correctly omit it ✔,
  but asymmetry vs RISM_MCP_PORT deserves a nit row (parity of env tier)).

---

## Design-gap list (questions the shipped code cannot answer → verdict candidates)

1. **A3/Origin**: spec Security "MUST validate Origin" — our config never
   enables it (`validate_empty_origin_allowlist` stays false, tower.rs:197)
   and under `--allow-all-interfaces` even the Host guard is off. Auth-less
   door + no Origin validation = the exact attack the spec warning names.
   (Non-breaking fix exists: one builder call.)
2. **A6/B2**: IPv6 `--host ::1` ready-line vs rmcp static host allow-list
   bracket normalization — unverifiable without the probe; if 403, the door
   documents a bind it cannot serve.
3. **A4/G4d**: DELETE-then-404 and SIGTERM-exit-0 assertions are comments in
   tests/probe, not asserts — the suite does not actually pin two
   headline contracts.
4. **B1**: TOCTOU between policy resolution and bind is architecturally
   un-answerable by the current code shape (single resolve+bind needed).
5. **C3**: dual stdio+http door — not implemented; fine, but TASK.md's
   question "should it be" is a design note, not a finding.

## Probe inventory for the coder lane (cheap → live)

- P1 A5 status matrix (406/400/404/405) — one ephemeral door, curl. [10 min]
- P2 A4 DELETE → 202 + POST-after-DELETE → 404. [5 min]
- P3 A2 `cargo tree -p rmcp -e features` + 200-id uniqueness sweep. [5 min]
- P4 A6/B2 `--host ::1` end-to-end (bind, ready line, initialize). [10 min]
- P5 C1 RUST_LOG=info, 5 sessions + 10 inline requests,
  `grep -c "negotiated Atelier"` == 1. [10 min, docker IRIS up]
- P6 E1 dead-IRIS background=true Rail-A + inline-task terminal state. [15 min]
- P7 D1 SIGTERM mid-task: exit code + stderr tail. [10 min]
- P8 G1 bare stdio byte-diff vs 9a1ddd7 binary (if baseline artifact builds
  cheaply in coder worktree). [20 min]
- P9 G3 release-binary size re-measure at cb51ecf. [build-time]
- P10 G4 probe replay under `env -i` per constitution.
Record all as MEASURED FACTS in testing-notes/REVIEW_FACTS.md with
command+exit+trimmed output per row.

---

## Plan corrections at close-out (specifier, after architect consolidation dc07a20)

The final findings table (testing-notes/REVIEW.md) superseded plan
expectations in four places; recorded here so the plan is not read as ground
truth after the fact:

1. **A5 — POST without session id is 422, not 400.** My plan's oracle
   ("missing session id → 400, tower.rs:1631") is correct only for the GET/
   DELETE handlers; under legacy session mode the POST branch (tower.rs:1766+)
   treats a missing sid as a sessionless initialize attempt and fails
   message-shape with 422 (measured by both downstream lanes; spec says
   SHOULD-400). Finding P-03, rmcp-owned.
2. **Design-gap 1 (Origin) CONFIRMED as F-01, the single fix-before-merge
   blocker** — `enforce_origin_validation()` is never called anywhere in
   src/mcp/http.rs (only `with_legacy_session_mode` :152 + conditional
   `disable_allowed_hosts` :158), so rmcp's empty-allowlist short-circuit
   (tower.rs:932–936) leaves Origin unvalidated on every bind.
3. **Design-gap 2 (A6/B2 ::1) RESOLVED PASS** — brackets normalize
   (tower.rs:780–781); the "documents a bind it cannot serve" risk did not
   reproduce.
4. **Design-gap 4 (B1 TOCTOU) SUBSUMED by F-02** — the resolver branch in
   `is_loopback_host` is dead code (portless `to_socket_addrs` always errors),
   so the gate never resolves names at all; there is no two-moment race, and
   loopback *named* hosts are refused outright.
5. **F/TASK.md "port 3111" does not exist in the tree** (grep empty) — the
   only fixed-port flake risk is cli_transport's 39999 (F5; proven by mutation
   + squat experiments as P-05).

No question A–G was left without an owner: every item ended as a measured
fact, a finding row, or a labeled design note in REVIEW.md.
