# R2 PLAN — specifier lane (Origin answers from rmcp source + fix sketches)

Authority for WHAT: `testing-notes/REVIEW.md` (swarmforge-architect ref).
Authority for HOW: vendored `rmcp-3.4.1` (per Cargo.lock; paths under
`~/.cargo/registry/src/index.crates.io-*/rmcp-3.4.1/`, file
`src/transport/streamable_http_server/tower.rs` unless stated). No Rust
written here; coder owns implementation.

## 1. F-01 Origin-compat answers (quoted from rmcp 3.4.1 source)

### (a) Exact API to enable Origin validation
Builder methods on `StreamableHttpServerConfig` (all consume `self`):

- tower.rs:218–224
  ```rust
  pub fn with_allowed_origins(
      mut self,
      allowed_origins: impl IntoIterator<Item = impl Into<String>>,
  ) -> Self {
      self.allowed_origins = allowed_origins.into_iter().map(Into::into).collect();
      self
  }
  ```
- tower.rs:226–229
  ```rust
  pub fn enforce_origin_validation(mut self) -> Self {
      self.validate_empty_origin_allowlist = true;
      self
  }
  ```
- tower.rs:231–234 `disable_allowed_origins()` clears the list AND the flag
  (the current de-facto state via `Default` at tower.rs:196–197).

The gate itself (tower.rs:936):
```rust
if !config.validate_empty_origin_allowlist && config.allowed_origins.is_empty() {
    return Ok(());
}
```
⇒ validation runs when EITHER the list is non-empty (so
`with_allowed_origins([...])` alone enables it) OR the flag is set
(`enforce_origin_validation()` activates validation even with an EMPTY list,
which then rejects every present Origin, since an empty list matches
nothing). Field is private: `validate_empty_origin_allowlist: bool`
(tower.rs:124); default `false` (tower.rs:197). Doc comment tower.rs:113–119:
"Defaults to an empty list, which disables Origin validation … missing-`Origin`
requests still pass. … Call `enforce_origin_validation` to enable validation
with an empty list, rejecting every present Origin value."

### (b) Absent-Origin request when enforcement is ON — ACCEPTED
tower.rs:939–941:
```rust
let Some(origin_header) = headers.get(http::header::ORIGIN) else {
    return Ok(());
};
```
An absent `Origin` header passes unconditionally, enforcement flag or not.
**Compat verdict: the fix does NOT break curl / MCP SDK / mcp_probe.sh
clients (they send no Origin). TASK.md's conditional fallback
(`validate_empty_origin_allowlist=true` "accordingly") is therefore only
needed as the deny-present-Origin posture; no matrix row breaks.**
Check runs on every method before routing — `handle()` calls
`validate_dns_rebinding_headers` at tower.rs:1540, which checks Host
first, then calls `validate_origin_header` (tower.rs:928).

### (c) What `validate_empty_origin_allowlist` actually gates
ONLY the short-circuit at tower.rs:936 — i.e. whether an EMPTY
`allowed_origins` list means "skip validation entirely" (false, the
default = F-01's hole) or "validate and therefore reject every present
Origin" (true). Once validation runs, the flag plays no further role;
list membership does the work. Rejection response: 403
`forbidden_response` (tower.rs:772–778; messages at :951 "Forbidden:
Invalid Origin header" for unparseable values, :960 "Forbidden: Origin
header is not allowed" for valid-but-unlisted).

### (d) Same-origin matching semantics
`parse_origin_value` (tower.rs:838–854): trims; empty string ⇒ `None`
(⇒ 403 malformed); case-insensitive `"null"` ⇒ `NormalizedOrigin::Null`;
otherwise `http::Uri::try_from(value)` MUST yield both a scheme and an
authority — scheme lowercased, host via `normalize_host`
(tower.rs:779–783: strips `[` `]`, lowercases), port `Option<u16>`.
Anything unparseable (no scheme, IPv6 literal WITHOUT brackets,
control chars, non-UTF-8 ⇒ separate 403 at :947) ⇒ 403.

`origin_is_allowed` (tower.rs:856–874):
```rust
a_scheme == o_scheme && a_host == o_host && (a_port.is_none() || a_port == o_port)
```
plus `Null` matches only `Null`. So: **scheme strict (http ≠ https),
host strict after lowercase + bracket-strip (case-INsensitive, IDNA not
applied — byte compare), port STRICT if the allowlist entry carries a
port, port-AGNOSTIC if it omits one** (so `"http://localhost"` would
match any localhost port — prefer explicit ports). A trailing `/` on an
Origin value parses (authority excludes the path) and matches; browsers
never send one but adversarial lane should confirm.

**Implication for our fix (matrix row (c) same-origin ⇒ 200):**
`enforce_origin_validation()` alone with an empty list 403s
`Origin: http://127.0.0.1:PORT` — so the loopback door must ship a
NON-EMPTY same-origin list computed from the bound address; the flag is
belt-and-suspenders (and is the whole posture for the allow-all bind).

## 2. Origin acceptance-matrix probe commands (coder runs; facts in R2_FACTS.md)

Bring up the door on `--port 0`, parse the real port from the stderr ready
line (`RISM_PORT` below). BASE = `POST /mcp` initialize:

```bash
U="http://127.0.0.1:$RISM_PORT/mcp"
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}'
code() { curl -s -o /dev/null -w '%{http_code}' -X POST "$U" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' "$@" -d "$INIT"; }

code -H 'Origin: https://evil.test'                       # (a) expect 403
code                                                     # (b) no Origin → expect 200
code -H "Origin: http://127.0.0.1:$RISM_PORT"            # (c) same-origin → expect 200
```
Note `-H 'Origin;'` (semicolon) sends NO header — that is row (b)'s shape;
curl otherwise adds Origin never by itself, so plain curl == row (b).
Extra rows for the adversarial lane (design, refactorer executes):
`Origin: HTTP://127.0.0.1:$P` (scheme case), `Origin: http://LOCALHOST:$P`,
`Origin:` (empty ⇒ 403 per parse_origin_value empty→None), `Origin: null`
(⇒ 403 unless "null" listed), `Origin: http://127.0.0.1:$P/` (trailing
slash), `Origin: http://[::1]:$P` via ::1 bind, Host-good+Origin-evil and
Host-evil+Origin-good combos.
Under `--allow-all-interfaces` the same three rows run against the LAN IP:
(a) 403, (b) 200, (c) per chosen posture (§3 F-01 note).

## 3. Per-fix implementation sketch (file:line at worktree tip db2daa3ade)

- **F-01** — `src/mcp/http.rs:149–159` (config site inside `serve_http_on`).
  After `addr` is known (http.rs:136–138), build
  `origins = ["http://localhost:{port}", "http://127.0.0.1:{port}",
  "http://[::1]:{port}"]` when the bind is loopback (port = `addr.port()`);
  chain `.with_allowed_origins(origins).enforce_origin_validation()` before
  `.with_cancellation_token(...)` so BOTH loopback and `--allow-all-interfaces`
  (list empty + flag ⇒ deny-every-present-Origin, matrix §2 posture;
  Host guard stays disabled there, http.rs:154–158 unchanged) get Origin
  enforcement. WHY comment + doc sentence in `docs/mcp.md` HTTP section:
  "Origin is validated when present (RFC 6454 match on the door's own
  origin); native clients may omit the header." Optional settings knob
  `mcp.http_allowed_origins: Vec<String>` appended to the computed list
  — Settings/Deserialize in `src/settings.rs`, plumbed as a param into
  `serve_http_on` — ONLY if ≤15 lines; otherwise defer (coder's call,
  record it in R2_FACTS.md).
- **F-02 (+P-08)** — `src/mcp/http.rs:43–52` `is_loopback_host`: replace the
  dead `to_socket_addrs` branch (:49–51) — make the fn async
  `tokio::net::lookup_host(format!("{host}:0")).await` (or add
  `is_loopback_host_async` and call from `check_bind_policy` http.rs:62,
  which is already on the async path before bind (:106)). `Ok(addrs)` ⇒
  non-empty AND all `.ip().is_loopback()`; `Err` ⇒ false (refuse, error
  names it). Update the vacuous unit test at http.rs:~230 to resolve
  real names (`ip6-localhost` must pass on this host — /etc/hosts maps it
  to ::1; add a guarded assert skipping `Err` DNS on offline CI).
  Signature change ripples only to `check_bind_policy`'s call (:63) and
  tests — grep showed no other callers.
- **P-04** — `tests/http_door_stateless.rs:136–142`: capture the DELETE
  response, `assert_eq!(status, 202)`; then POST `tools/list` with the
  deleted sid ⇒ `assert 404`.
- **P-05** — (a) `tests/cli_transport.rs:~50–85`: bind a
  `TcpListener::("127.0.0.1:0")`, take `local_addr().port()`, DROP the
  listener, use that number in the "nothing listened" assert (replaces
  fixed 39999 at :53/:81). (b) new status-matrix test (extend
  `tests/http_door.rs` or a `tests/http_door_status_matrix.rs`): 406 wrong
  Accept (GET no-sid `application/json`), 400 missing-sid GET(es)/DELETE,
  404 unknown-sid, 405 PUT, 415 wrong CT, 413 >4 MiB body (DEFAULT_MAX_
  REQUEST_BODY_BYTES = 4 MiB, tower.rs:55), + F-01 rows: evil Origin 403,
  no Origin 200, same-origin 200. (c) guard-toggle pin: same door served
  via `serve_http_on(listener, settings, true)` vs `false` —
  `Host: evil.example` ⇒ 200 vs 403 (mutation A must go RED).
- **P-06** — `packaging/mcp_probe.sh:199–203`: replace
  `wait $http_pid 2>/dev/null || true` + hardcoded echo with
  `kill -TERM $http_pid; if ! wait $http_pid; then echo "MCP probe: HTTP door SIGTERM exit != 0"; exit 1; fi`
  before the OK line (keep the trap/EXIT cleanup ordering).
- **P-07** — `src/mcp/http.rs:166–170`: move the `eprintln!` ready line UP
  to immediately after `addr` is resolved (:131–134), BEFORE
  `RismMcp::new(settings).await` (:146 — the negotiate that can take ≤30 s
  on a dead IRIS). Byte-identical content (parsers hold); WHY comment:
  "the socket accepts from bind; harnesses gate on this line, not on
  handler readiness."
- **P-10** — `src/settings.rs:11–13`: comment reword
  "Prism-compatible aliases (minus deprecated `sse`)". Comment-only.
- NOT in scope: P-03 (upstream rmcp note, coordinator), P-09/P-11/P-12/P-13.

## 4. Mutation-revert checklist (adversarial lane; each revert MUST go RED)

| # | Mutation (revert the fix) | Test that must fail |
|---|---------------------------|---------------------|
| M1 | drop `.with_allowed_origins(..)`+`.enforce_origin_validation()` (F-01) | status-matrix evil-Origin row (403→200) |
| M2 | keep chain but loopback list EMPTY under loopback bind | same-origin row (c) 200→403 |
| M3 | `is_loopback_host`: back to `to_socket_addrs` sync branch (F-02) | named-loopback test (`ip6-localhost` passes gate) |
| M4 | http_door_stateless: DELETE asserts removed (P-04) | that test's 202/404 asserts |
| M5 | delete `disable_allowed_hosts` branch under allow-all (P-05c) | guard-toggle pin |
| M6 | pin any 4xx wrong in the matrix test body (e.g. 405→404 PUT) | status-matrix PUT row |
| M7 | mcp_probe.sh: reintroduce `|| true` + run door that traps TERM & exits 3 (or stub `wait`) | probe SIGTERM row FAILs |
| M8 | ready line back after `RismMcp::new` with blackhole base_url (P-07) | probe/ready-line-before-negotiate timing test |

## 5. Lane facts the coder must record (R2_FACTS.md)
Matrix §2 (a)(b)(c) raw codes; `mcp_probe.sh` full green run;
`--host ip6-localhost` bind success; `git grep Origin` shows enforcement
config-only; settings-knob decision (in/out) with line count.
