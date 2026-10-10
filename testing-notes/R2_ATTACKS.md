# R2 ATTACKS — refactorer lane (adversarial verification of the coder's fixes)

Task: rism-r2-fixes. Merged coder tip `0c0f532779` (fast-forward onto
`ffedb2a`; lane base cb51ecf). Every row below was run LIVE against the
real binary (`target/debug/rism mcp --transport http --port 0`, docker
rism-iris backend) with raw-socket frames so Host/Origin hit the wire
verbatim, unless stated otherwise. Doors were killed by PID after each
run (exit 0 observed); none spawned by this lane survive.

Verdict legend: REFUTED = the attack failed, fix holds. CONFIRMED(bypass)
= a hole. DOCUMENTED = accepted behavior with an upstream-cause analysis
and no security vector (pinned so a future flip shows up).

## 1. Origin bypass matrix — loopback bind (default door)

| # | attack (Origin value / shape) | expected | MEASURED | verdict |
|---|---|---|---|---|
| A1 | `https://evil.test` (POST init) | 403 | 403 not-allowed | REFUTED |
| A2 | absent Origin (native client) | 200 | 200 | REFUTED (compat row (b) holds) |
| A3 | `http://127.0.0.1:PORT` same-origin | 200 | 200 | REFUTED |
| A4 | `http://LOCALHOST:PORT` (host case) | 200 | 200 | REFUTED |
| A5 | `HTTP://127.0.0.1:PORT` (scheme case) | 200 | 200 | REFUTED |
| A6 | `http://localhost:PORT` (alt name) | 200 | 200 | REFUTED |
| A7 | `http://[::1]:PORT` (v6 bracket) | 200 | 200 | REFUTED |
| A8 | `http://::1:PORT` (v6 NO brackets) | deny | 403 invalid | REFUTED |
| A9 | same-origin + trailing slash | 200 | 200 | REFUTED |
| A10 | same-origin + path appended | (parse) | 200 (authority excludes path) | REFUTED |
| A11 | `Origin:` (present, EMPTY value) | deny | 403 invalid | REFUTED |
| A12 | `Origin: ` (empty w/ space, trimmed) | deny | 403 invalid | REFUTED |
| A13 | `null` | deny | 403 not-allowed | REFUTED |
| A14 | `NULL` / `Null` | deny | 403 not-allowed | REFUTED |
| A15 | `undefined` | deny | 403 invalid | REFUTED |
| A16 | `file:///` | deny | 403 invalid (no authority) | REFUTED |
| A17 | `chrome-extension://abcdefghij` | deny | 403 not-allowed | REFUTED |
| A18 | `127.0.0.1:PORT` (schemeless) | deny | 403 invalid | REFUTED |
| A19 | `http://127.0.0.1` (PORTLESS default) | deny | 403 not-allowed | REFUTED — the allowlist carries explicit ports (plan §1(d)); a portless Origin never matches. Browsers omit the port only for ws/wss-default schemes, never for http on a custom port, so no vector. |
| A20 | `http://127.0.0.1:PORT+1` (wrong port) | deny | 403 not-allowed | REFUTED |
| A21 | `https://127.0.0.1:PORT` (scheme mismatch) | deny | 403 | REFUTED |
| A22 | `http://lοcalhost:PORT` (GR omicron homoglyph) | deny | 403 invalid (non-ASCII byte fails Uri) | REFUTED |
| A23 | `Origin: http://evil@127.0.0.1:PORT` (userinfo) | deny? | **200** | DOCUMENTED — `http::uri::Authority::host()` strips userinfo (`authority.rs:429` `rsplit('@')`), host == `127.0.0.1`, port matches ⇒ allowed. NO browser vector: the Fetch spec serializes Origin as scheme/host/port only — a page can never put userinfo in an Origin header. A raw-socket attacker who can set arbitrary headers already doesn't need Origin. Homoglyph/IDN hosts FAIL parsing (A22) — the mismatch is deliberate per rmcp: byte-compare, no IDNA. |
| A24 | `http://127.0.0.1.:PORT` (trailing dot) | deny | 403 not-allowed | REFUTED |
| A25 | DUPLICATE Origin: allowed THEN evil | deny | **200** | DOCUMENTED — rmcp consults `headers.get(ORIGIN)` = FIRST value only (header-value joining only happens within one line); the second header line is invisible to the check. NO browser vector: fetch sets exactly one Origin slot; duplicates are impossible from a page. Pinned as 200 (see pin section) so a future rmcp that AND-mingles duplicates flips visibly. Evil-FIRST duplicate ⇒ 403 (A26). |
| A26 | DUPLICATE Origin: evil THEN allowed | deny | 403 not-allowed | REFUTED |
| A27 | header NAME case: `origin:`/`ORIGIN:`/`origIN:` evil | deny | 403 ×4 | REFUTED — hyper's HeaderMap is case-insensitive per RFC 9110 §5.1 |
| A28 | evil Origin on GET-stream leg (no sid) | 403 not 400 | 403 | REFUTED — gate runs before routing |
| A29 | evil Origin on DELETE leg (no sid) | 403 not 400 | 403 | REFUTED |
| A30 | evil Origin on PUT (non-door method) | 403 not 405 | 403 | REFUTED |
| A31 | evil Accept (406 shape) + evil Origin | deny wins | 403 | REFUTED — header gate precedes content negotiation |
| A32 | established-session legs: tools/list + userinfo Origin | (A23 shape) | 200 | DOCUMENTED (same cause; enforcement is per-request, session state irrelevant) |
| A33 | tools/list + evil Origin WITH valid sid | 403 | 403 | REFUTED |
| A34 | DELETE + userinfo Origin (valid sid) | (A23 shape) | 202 | DOCUMENTED |

## 2. Host × Origin combos — loopback bind

| # | Host | Origin | MEASURED | verdict |
|---|---|---|---|---|
| B1 | `evil.example` | absent | 403 Host-guard | REFUTED |
| B2 | `evil.example` | same-origin | 403 Host-guard (Host checked FIRST) | REFUTED |
| B3 | `127.0.0.1` | evil | 403 Origin | REFUTED |
| B4 | `localhost:PORT` | same-origin | 200 | REFUTED |
| B5 | `127.0.0.1:PORT` | `http://localhost:PORT` | 200 | REFUTED |
| B6 | `evil.example:80` | evil | 403 Host-guard | REFUTED |

rmcp default `allowed_hosts = ["localhost","127.0.0.1","::1"]` — Host
authorities are port-AGNOSTIC (rmcp tower.rs `host_is_allowed`: entry
without port matches any port). The Host guard therefore accepts
`localhost:ANY`, but Origin is port-STRICT (A20), so a cross-port page
still denies on the Origin leg. Not a hole in the F-01 posture; noted.

## 3. allow-all bind posture (`--host <LAN> --allow-all-interfaces`, run
here as loopback bind + `--allow-all-interfaces`, the same
`serve_http_on(_,_,true)` wiring the tests use)

| # | request | MEASURED | note |
|---|---|---|---|
| C1 | any PRESENT Origin (24 shapes incl. all of A1–A26 variants tried here) | 403 | deny-every-present = shipped posture; coherent: no same-origin string is knowable on a LAN bind and there are no browser consumers |
| C2 | absent Origin | 200 | curl/SDK/probe unaffected |
| C3 | `Host: evil.example`, no Origin | 200 | guard off — the documented opt-out |
| C4 | evil Host + present evil Origin | 403 | Origin leg still denies |

## 4. IPv6 door (`--host ip6-localhost`) — same battery

| # | request | MEASURED |
|---|---|---|
| D1 | absent Origin | 200 |
| D2 | `https://evil.test` | 403 |
| D3 | `http://[::1]:PORT` | 200 (bracket-strip in normalize_host) |
| D4 | `Host: localhost:PORT` | 200 (Host guard name-based) |
| D5 | `Host: [::1]` (portless) | 200 (port-agnostic, §2 note) |
| D6 | ready line form | `listening on http://[::1]:PORT/mcp` — bracketed, URL-correct |

## 5. F-02 bind-gate behavior

- `--host ip6-localhost`: passes gate async, BINDS `::1` (ready line on
  `[::1]:PORT`), no `--allow-all-interfaces` nag, TCP-connectable on ::1.
  CONFIRMED green, both by the shipped pins (`named_loopback_hosts_really_
  resolve` unit + `named_loopback_host_binds_without_optout` integration)
  and by a live manual probe.
- `--host no.such.host.invalid`: refuses BEFORE bind — process exits rc 1
  with the `refusing to bind ... NO authentication` message, and the port
  probed had NO listener afterwards (socket to that port: refused).
  refuse-before-bind CONFIRMED green.
- `--allow-all-interfaces`: warning banner + serve (unchanged).

## 6. Compat flows (P-06/P-07 parsers hold)

- `packaging/mcp_probe.sh` (built binary on PATH): 4 OK lines, rc 0,
  incl. the now-asserted `SIGTERM exit 0` leg; the ready-line parse
  (`grep -o 'listening on http://[^ ]*'` + `cut -d' ' -f3`) still finds
  the line at its NEW position — nothing greps the OLD position.
- `tests/http_door.rs` / `cli_transport.rs` split on `"listening on "`:
  position-independent, green in the full suite.
- stdio door frozen: `rism mcp` stdio leg of the probe + api tests green.

## 7. Mutation matrix (revert each fix in the working tree; NEVER committed; `git checkout --` after each)

| # | mutation | command | result |
|---|---|---|---|
| M1 | drop `.with_allowed_origins(..).enforce_origin_validation()` from the config chain | `cargo test -q --test http_door_status_matrix` | **RED** — all 3 tests fail (status_matrix evil-Origin flips to 200, posture-toggle allow-all Origin leg flips, NEW bypass pin fails) ✓ non-vacuous |
| M2 | `allowed_origins_for` → always empty list | same | **RED** — `status_matrix_is_pinned` (same-origin row) + NEW bypass pin (group B) fail ✓ |
| M3 | `is_loopback_host` → old dead `to_socket_addrs` sync branch | `cargo test -q --lib` + `--test cli_transport` | **RED** — `named_loopback_hosts_really_resolve` + `named_loopback_host_binds_without_optout` both fail ✓ |
| M4 | invert the P-04 pins (DELETE 202→404, dead-sid 404→202) | `cargo test -q --test http_door_stateless` | **RED** — `http_door_factory_sessions_and_error_path` fails on the flipped asserts ✓ (the live door really answers 202/404) |
| M5 | delete the `disable_allowed_hosts` branch | `cargo test -q --test http_door_status_matrix` | **RED** — `host_guard_and_origin_posture_toggle_with_bind` fails ("allow-all door lets any Host through") ✓ |
| M6 | PUT pin 405→404 in the test body | `cargo test -q --test http_door_status_matrix` | **RED** — status_matrix PUT assert fails ✓ (server truly answers 405) |
| M7 | old `wait || true` shape vs `if ! wait` (bash repro: child traps TERM, exits 3) | direct bash legs | OLD shape prints OK despite exit 3 (the hole), NEW leg takes the FAIL branch (exit 1) ✓ shape-proven on the probe's exact idiom, `packaging/mcp_probe.sh:203` |
| M8 | ready line moved back AFTER `RismMcp::new` | `cargo test -q --test http_door_stateless` | **RED** — `ready_line_precedes_handler_negotiate` fails at 30.02 s ("must precede the negotiate (30 s)") ✓ |

All 8 mutations go RED; no pin is vacuous (including the NEW bypass pin,
which itself fails under M1 and M2 — verified).

## 8. New pins added by this lane

- `tests/http_door_status_matrix.rs ::
  origin_bypass_matrix_denies_every_browser_shape` — groups A/B/D/E/F
  (the browser-reachable deny set, the same-origin allow spellings, the
  method-leg gate-order rows, the two DOCUMENTED upstream quirks pinned
  AT their measured values with why-comments, the Host×Origin combo, and
  the header-name case sweep). Mutation-checked against M1/M2 (RED).

## 9. Findings summary

- NO bypass CONFIRMED. Every browser-reachable cross-origin shape denies
  403 on both bind postures and on every method leg; the absent-Origin
  native path is untouched (200) — probe + curl + SDK flows verified.
- Two header-shape quirks are upstream `http`-crate/rmcp parsing
  (userinfo stripped; duplicate Origin = first value decides) with zero
  browser-reachability; accepted, pinned at measured values, documented
  above (§1 A23/A25) — NOT design holes; no code change warranted. A
  future rmcp change to either semantic flips a pin.
- Posture judged COHERENT per handoff note 1: loopback = RFC-6454
  same-origin set with explicit ports; allow-all = deny-every-present.
  The deferred settings knob was judged: not needed for coherence
  (no browser consumer; extra origins only weaken the posture).
- F-02: both legs confirmed green live, refuse-before-bind proven with a
  post-refusal connect probe. P-07 parsers all hold.
- Mutation matrix: M1–M8 all RED as claimed (coder's M1/M2/M8 spot-
  checks reproduce; M3–M7 newly verified end-to-end by this lane).

## 10. Process hygiene

Doors spawned by this lane (driver scripts + one backgrounded probe
door): all killed by PID-by-port, exit 0 observed, ports verified
closed. The 7 stale R1-era doors (PIDs 185441 186024 189248 189375
198632 198968 199628) are the coordinator's problem per handoff note 3 —
untouched.
