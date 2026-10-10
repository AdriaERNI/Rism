# W1 SPEC — streamable-HTTP MCP transport door (task: rism-win-http)

Specifier pass over `feat/windows-mcp-http` @ 9a1ddd7. All "ground facts"
re-verified against code and the vendored rmcp 3.4.1 source (paths below are
`~/.cargo/registry/src/*/rmcp-3.4.1/`). Corrections at the end.

## 1. CLI surface (src/cli/mod.rs + src/main.rs)

`Commands::Mcp` becomes a struct variant (catch-all arms `Self::Mcp` in
`to_execute_sql`/`to_doc` → `Self::Mcp { .. }`; main.rs line-35 `matches!` →
`if let Commands::Mcp { .. }` pattern to read fields):

```
rism mcp [--transport <stdio|http>] [--port <N>] [--host <H>] [--allow-all-interfaces]
```

- `--transport`: clap `ValueEnum` `McpTransport { Stdio, Http }`,
  `#[value(alias = "streamable-http", alias = "streamable_http")]` on Http,
  `ignore_case = true` (Prism parity: `http`→streamable-http + both spellings).
  Unknown value → clap's standard `error: invalid value 'x' for '--transport
  <TRANSPORT>'` + possible-values list, exit 2. Never a panic.
- `env = "RISM_MCP_TRANSPORT"` / `env = "RISM_MCP_PORT"` on the flags (same
  pattern as global `--url`); do NOT add RISM_MCP_* to `settings.rs::apply_env`
  — clap covers CLI > env, then main merges `flag.or(settings field)` for the
  config.toml tier. Bad env value = clap error (deterministic, same as --url's).
- `--port` default 3000 (used only by http). `0` = ephemeral: bind `:0`, then
  the ready line on STDERR prints the ACTUAL port:
  `rism MCP server ready (http) — listening on http://<host>:<port>/mcp`
  (probe/tests parse this line; stdout stays JSON-RPC-pure on stdio, empty on
  http).
- `--host` default `127.0.0.1`.
- NO flags at all → stdio, byte-for-byte shipped behavior (row C).
- `--port/--host` given with stdio transport: one stderr warning
  (`rism mcp: --port/--host are ignored with the stdio transport`), proceed as stdio.

## 2. Wiring (src/mcp)

Keep `serve(settings)` = stdio untouched; add `serve_http(settings, HttpConfig)`.

- `RismMcp` is ALREADY `#[derive(Clone)]` and cheap: `IrisClient` is
  `Arc<Inner>` (src/iris/http.rs:13) and `ToolRouter<S>` has a blanket
  `impl<S> Clone` (rmcp router/tool.rs:361). NO Arc refactor needed.
  Per-request factory: construct `RismMcp::new(settings)` ONCE before serving
  (its `negotiate_version()` is a best-effort GET, never fatal), then
  `StreamableHttpService::new(move || Ok(handle.clone()), Arc::new(LocalSessionManager::default()), cfg)`.
- rmcp feature to add: `transport-streamable-http-server` (pulls
  server-side-http: uuid/rand/http/http-body/http-body-util/bytes/sse-stream/
  tower-service/base64 + tokio-stream). Cargo.lock check (this pass):
  http/http-body/http-body-util/tower-service/uuid/tokio-stream/tokio-util
  already resolve via existing deps; genuinely NEW to the graph = `sse-stream`
  (rmcp-private) + axum (added by us, §4). Expect a modest size delta.
- The service is a tower `Service<Request>` — NOT an axum dependency of rmcp
  (axum there is dev-only). Rism must add `axum = "0.8"` as a real dep and
  mirror rmcp's OWN proven test pattern (tests/test_streamable_http_protocol_version.rs:32-53):
  `axum::Router::new().nest_service("/mcp", service)` + `axum::serve(listener, router)`.
  (Alternative hyper-only glue rejected: more code, zero size win vs axum-core.)
- Config: `StreamableHttpServerConfig::default()` + explicit
  `.with_cancellation_token(ct.clone())`. Keep `legacy_session_mode = true`
  (stateful `Mcp-Session-Id` sessions — what TASK.md row D expects) and rmcp's
  default sse_keep_alive (15 s). `allowed_hosts` default =
  [localhost, 127.0.0.1, ::1] host-header guard: keep it when binding loopback;
  when `--allow-all-interfaces` (or a non-loopback --host with the flag) →
  `.disable_allowed_hosts()` (or `with_allowed_hosts(host, "localhost")`) and
  print a loud stderr WARNING banner: no auth on this door, every local (or
  network, with the flag) process can drive all 25 tools.
- Bind 0.0.0.0 / non-loopback host WITHOUT `--allow-all-interfaces`: refuse at
  startup, stderr error, non-zero exit. Port already in use: clean io-error
  exit, never a panic, never a hang.
- Graceful shutdown: `tokio::select!` over
  `axum::serve(..).with_graceful_shutdown(ct.cancelled_owned())`,
  `tokio::signal::ctrl_c()` (works on Windows — row D Ctrl+C), and
  `#[cfg(unix)]` SIGTERM via `signal(Terminate)` (already enabled: tokio
  feature "signal"). On signal: `ct.cancel()` (kills sessions), await serve
  exit, return Ok — process exits 0, no zombie.
- Logging: `serve_http` inits the SAME stderr-only tracing_subscriber as stdio
  `serve` (with_ansi(false), env filter, default "info"). Logs NEVER on stdout
  on either door.

## 3. Settings / config.toml additions (src/settings.rs)

- New fields (serde(default) already covers old files):
  `mcp_transport: String` = "stdio", `mcp_port: u16` = 3000,
  `mcp_host: String` = "127.0.0.1".
- Resolution helper (pure fn, unit-tested — no set_var, edition-2024 rule):
  `McpTransport::resolve(cli_opt: Option<Self>, config: &str) -> Result<.., String>`
  with the SAME alias map. A config value that is not a known transport
  (`mcp_transport = "sse"`) → startup error naming the file path and value,
  non-zero exit. Precedence: CLI flag > RISM_MCP_* env > config.toml > default.
- Validation of host binding stays in main/serve_http (needs the flag).

## 4. Deps & size duty

- SIZE LEDGER (coder, this pass): release binary 9,378,168 B at baseline
  9a1ddd7 -> 11,038,792 B at 04bc98c = +1,660,624 B (+17.7%), single flat
  Linux build; accepted per §4 rule (no feature gate). Coordinator pastes
  into the PR body.
- Cargo.toml: `rmcp = { features += "transport-streamable-http-server" }`,
  new `axum = "0.8"`, plus `tokio-util` (CancellationToken — rmcp re-exports?
  no: it's a direct rmcp dep; Rism must depend on `tokio-util` too, or drive
  shutdown via an oneshot from a signal future — PREFER `tokio-util = "1"`
  explicit dep for CancellationToken).
- MSRV: Rism declares 1.85 and clippy `incompatible_msrv` gates ITS code;
  note rmcp 3.4.1 itself declares rust-version 1.88 (already true today —
  stdio already pulls it). No new constraint; stay off >1.85 std APIs in our
  code; any String truncation floors to a char boundary (rism-dev-workflows).
- Binary size: coder measures `cargo build --release` size before/after
  (baseline = current tip build on this machine), records the delta here in a
  commit amending SPEC.md's §7 ledger line; coordinator pastes into PR body.
  Decision rule: NO cargo feature gate — the door compiles unconditionally
  (simpler matrix, matches "opt-in behavior, not opt-in build").

## 5. Test plan (coder MUST land all; contract tests exercise the GENERAL case)

T1. `tests/http_door.rs` (#[ignore] live): spawn built binary
    `rism mcp --transport http --port 0` (reuse tests/support harness spawn
    style), parse the stderr ready-line for the real port, then reqwest
    against `http://127.0.0.1:<port>/mcp`: POST initialize (Accept:
    application/json, text/event-stream) → capture `mcp-session-id` header →
    notifications/initialized → tools/list (count 25 `"inputSchema"` markers)
    → tools/call execute_sql `select 1 as ok` → assert `"isError":false` and
    ok column. Kill via child kill; assert exit + no leftover listener
    (EADDRINUSE-free rebind). Runs in CI live-smoke with docker IRIS and on VM
    row D.
T2. Non-ignored in-process door test (src/mcp unit test or tests/http_door_stateless.rs):
    build `RismMcp` with default Settings pointed at an unused loopback port
    (no IRIS needed — `negotiate_version` is best-effort), serve on
    `127.0.0.1:0` via the SAME serve_http wiring (factory path covered), drive
    initialize + tools/list round-trip; one tools/call execute_sql against the
    dead URL must ANSWER with a tool-level error (never hang — general
    error-path rule). This pins the Clone-per-request factory + session header.
T3. stdio regression: existing probe (`packaging/mcp_probe.sh`) + assert_cmd
    spawn of bare `rism mcp` feeding one initialize line (no port opened;
    stdout pure). A `--transport stdio --port 39999` case asserts the warning
    and that no listener exists on 39999.
T4. Unit tests: `McpTransport` alias resolution (all spellings + case
    insensitivity + rejection), clap parse of `--transport bogus` exit code 2
    (assert_cmd), config-file precedence matrix (pure resolve fn), bind
    0.0.0.0 refusal, `--allow-all-interfaces` warning text present.
T5. `packaging/mcp_probe.sh`: append an HTTP leg (port 0 + stderr ready-line
    parse, curl or bash /dev/tcp — the probe must stay zero-dep: use a
    `python3`-free raw HTTP POST via exec 3<>/dev/tcp if feasible, else
    document and gate on curl presence) — initialize → tools/list count 25 →
    one call. Keep the stdio leg exactly as is.

## 6. Docs (docs-drift gate: tool-count claims stay 25; no new tools)

- `docs/cli-reference.md` §`rism mcp`: flag table + examples, "no auth on the
  HTTP door — bind loopback only", RISM_MCP_TRANSPORT/RISM_MCP_PORT.
- `docs/getting-started.md`: remote-MCP-client snippet pair
  (`{"mcpServers":{"rism":{"url":"http://localhost:3000/mcp"}}}` for
  Claude/Cursor) + Windows config-file note (%APPDATA%\rism\config.toml).
- `docs/configuration.md`: new config keys + env names (keep %APPDATA%/Linux
  path rows intact — gate checks them verbatim).

## 7. Acceptance-matrix owners (A–G)

- A (installer/PATH), B (CLI door), E (config precedence on Windows), F
  (uninstall), C-VM, D-VM: COORDINATOR's QA lanes on the VM (coder/refactorer
  NEVER run vagrant/winrm). Chain ships the binary; ledger rows stay OPEN
  until VM evidence exists in
  `/home/hermes/.hermes/cache/scratch/rism-win/evidence/`.
- C (stdio contract): mcp_probe.sh + T3 — architect gate.
- D (HTTP contract, Linux side): T1/T2 — architect gate (--ignored legs with
  docker IRIS up: `cd repo && docker compose up -d`, creds
  ~/.config/rism/config.toml).
- G (fmt/clippy/test/probe/gates green, PR clean): architect; coordinator
  folds chain → feat/windows-mcp-http → CI.

## 8. Non-goals

No new MCP tools, no renames (wire names FROZEN), no auth on the HTTP door, no
SSE-only legacy door, no Prism REST gateway work, no cargo feature-gating of
the door, no changes to the 25-tool surface, stdio default behavior untouchable.

## 9. Ground-fact corrections found this pass

1. "likely needs an Arc-shared state refactor of the handler" — REFUTED:
   `RismMcp` is already Clone with Arc-cheap state (§2). Zero refactor.
2. rmcp exposes NO axum integration as a dep — axum must be ADDED to Rism
   (0.8, rmcp's own test pattern). project.prompt's "tower/axum transitive
   weight" is right; the exact wiring is §2.
3. Evidence path conflict (TASK.md `.swarmforge/evidence/` vs constitution
   scratch dir): constitution wins —
   `/home/hermes/.hermes/cache/scratch/rism-win/evidence/`.
4. TASK.md says SPEC must NOT be committed; this role prompt commits
   `testing-notes/SPEC.md` (constitution allows testing-notes/ where a role
   prompt says so). Role prompt wins; repo code diff stays pure.
5. rmcp 3.4.1 declares rust-version 1.88 > Rism's 1.85 — already the status
   quo on stdio; nothing W1 changes, but do not "fix" the 1.85 declaration.
