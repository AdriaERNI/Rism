# R2 FACTS — coder lane (implementation + live evidence)

Task: rism-r2-fixes. Base: PR #18 tip cb51ecf + specifier 0f656c0.
All fixes are in src/mcp/http.rs, src/settings.rs, tests/*, packaging/
mcp_probe.sh, docs/cli-reference.md (one sentence). No frozen surface
touched (25 tools, wire names, stdio path — proven green by the suite +
probe tails below).

## 1. F-01 Origin acceptance matrix — LIVE, real binary, docker rism-iris backend

Door: `env -i HOME=$HOME PATH=/usr/bin:/bin ./target/debug/rism mcp
--transport http --port 0` → ready line `listening on http://127.0.0.1:41133/mcp`.
`code()` = `curl -s -o /dev/null -w '%{http_code}' -X POST $U -H
'Content-Type: application/json' -H 'Accept: application/json,
text/event-stream' [extra headers] -d '<initialize 2025-06-18>'`.

Required rows (§2 of R2_PLAN):

| row | request | expected | MEASURED |
|---|---|---|---|
| (a) | `Origin: https://evil.test` | 403 | **403** |
| (b) | no Origin (plain curl) | 200 | **200** |
| (c) | `Origin: http://127.0.0.1:41133` | 200 | **200** |

Extra rows (free, same door, same run):

| request | measured |
|---|---|
| `Origin: http://localhost:41133` | 200 |
| `Origin: http://LOCALHOST:41133` (host case) | 200 |
| `Origin: HTTP://127.0.0.1:41133` (scheme case) | 200 |
| `Origin: http://127.0.0.1:41133/` (trailing slash) | 200 |
| `Origin: null` | 403 |
| `Origin:` present but EMPTY (curl `-H 'Origin;'`) | 403 |
| `Host: evil.example`, no Origin | 403 (Host guard) |
| `Origin: http://127.0.0.1:41134` (wrong port) | 403 |

rmcp parse_origin_value("") ⇒ None ⇒ 403 "Invalid Origin header"
(tower.rs:840) — NOTE: this CONTRADICTS plan §2's claim that `-H 'Origin;'`
sends no header; measured here it goes on the wire with an empty value
(plan's row (b) is the plain `code` call, which was 200). SIGTERM after the
run: exit 0.

Posture shipped (config-only, per plan §3 F-01): `.with_allowed_origins(
allowed_origins_for(addr, all_interfaces)).enforce_origin_validation()`.
Loopback bind ⇒ `["http://localhost:{port}", "http://127.0.0.1:{port}",
"http://[::1]:{port}"]` (explicit ports; `[::1]` listed so a door bound on
IPv6 loopback answers to the browser origin string — rmcp parses IPv6
values bracketed, tower.rs:851 normalize_host strips them). allow-all bind
⇒ EMPTY list + flag ⇒ every PRESENT Origin 403, absent Origin passes
(tower.rs:939-941 ⇒ curl/SDK/probe unaffected). Host guard untouched
(still disabled under --allow-all-interfaces only).

§2's allow-all rows measured live (`--host 192.168.122.219
--allow-all-interfaces`, --port 0 ⇒ 34399): (a) evil-Origin **403**,
(b) no-Origin **200**, (c) same-origin **403** = the chosen
deny-every-present posture, and `Host: evil.example` **200** (guard off,
documented opt-out). SIGTERM exit 0.

SETTINGS-KNOB DECISION (plan's optional ≤15-line knob): **OUT**. A real
plumb (Settings field + Default + apply_env/config parse + HttpServerConfig
field + CLI merge + doc) is ~30+ lines across 3 files — over the budget,
and no consumer needs it (no browser client of this door exists; extra
origins would only weaken the posture). `allowed_origins_for` is the single
source of the list.

`git grep -n Origin -- src/ tests/ packaging/`: matches ONLY config/test
wiring + comments (src/mcp/http.rs:175-181, 244-249, 298;
tests/http_door_status_matrix.rs) — enforcement is config-level, no custom
header middleware anywhere.

## 2. F-02 named-loopback bind — LIVE

`--host ip6-localhost` (/etc/hosts: `::1 ip6-localhost`): passes
check_bind_policy via `tokio::net::lookup_host((host, 0))` and BINDS —
pinned two ways: unit `named_loopback_hosts_really_resolve` (guarded) +
integration `named_loopback_host_binds_without_optout` (real binary, ready
line, exit path). Manual probe: spawn printed the ready line on `[::1]:P`
(test does exactly this; P-07 moved the line so it prints at bind anyway).
`--host no.such.host.invalid` still refuses before bind (Err ⇒ false ⇒
bind error names it — unit pin updated to async, no longer vacuous).

## 3. Test commits per fix

- P-04: `tests/http_door_stateless.rs` DELETE leg now asserts 202 AND the
  follow-up POST on the deleted sid ⇒ 404 (9cd5d61).
- P-05a: cli_transport must-fail-connect probe uses bind-`:0`-then-drop
  port (f06897c).
- P-05b/c: NEW `tests/http_door_status_matrix.rs`: 406 (POST json-only
  Accept + GET no-sid), 400 (GET no-sid es-Accept; DELETE no-sid), 404
  (unknown sid POST), 405 (PUT), 415 (text/plain POST), 413 (4 MiB + 1 KiB
  body — DEFAULT_MAX_REQUEST_BODY_BYTES, tower.rs:55), F-01 rows (evil
  Origin 403, absent 200 = initialize itself, same-origin 200), plus the
  guard-toggle pin: Host evil ⇒ 403 on default door vs 200 on
  allow-all-posture door (`serve_http_on(_, _, true)`) — and Origin evil
  ⇒ 403 on BOTH. Raw tokio sockets so Host/Origin hit the wire verbatim.
- P-06: mcp_probe.sh SIGTERM leg = `kill -TERM; if ! wait; then FAIL; exit
  1` (fd00e17). Synthetic M7-shape proof: child trapping TERM + exit 3 ⇒
  leg takes the FAIL branch (exit 1).
- P-07: ready line moved to immediately-after-bind, byte-identical content
  (5b96dc4); M8 pin `ready_line_precedes_handler_negotiate` (fa54eff):
  blackhole IRIS (accepts, never answers) + `--url`, ready line asserted
  <3 s while the 30 s negotiate is pending. Mutation run: with the line
  back at its old position the test FAILED at 30.03 s (RED ✓).
- P-10: settings.rs doc-comment reword (6d59635).

Mutation spot-checks done by coder (refactorer re-runs the full table):
M1 (drop the two Origin chain calls) ⇒ BOTH matrix tests FAILED (RED ✓);
M2 (loopback list forced empty) ⇒ same-origin row FAILED (RED ✓); M8 ✓
(above). M3/M4/M5/M6/M7 legs are pinned by the tests named above
(M7 by the synthetic proof since it targets probe, not tests).

## 4. Full gates tail (final tree, after fmt)

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets` (stable 1.97.0, -D warnings run clean: 0
  warnings): 0.
- `rustup run 1.99.0 cargo clippy --all-targets`: 0 (after swapping two
  `assert!(..is_empty())` for `assert_eq!(.., Vec::<String>::new())` —
  new pedantic `assert_is_empty` lint).
- `cargo test -q`: lib 75 ok; api 13 ok; cli_transport 7 ok; http_door 2
  ok (1 ignored); http_door_stateless 2 ok; http_door_status_matrix 2 ok;
  tool_matrix/others ok — 0 failed everywhere.
- `cargo test -q -- --ignored` vs docker rism-iris (Up, healthy; creds from
  ~/.config/rism/config.toml, never retyped): background_jobs 17 ok,
  http_door live binary 1 ok, tool_matrix 17 ok + tails — 0 failed. Note:
  first batched run had 1 flake of `quick_task_completes_with_the_sync_
  result` (timing-based ObjectScript loop under full-suite load; passed
  standalone AND on the quiet full re-run).
- `env -i HOME=$HOME PATH=... bash packaging/mcp_probe.sh`: 4 OK lines,
  exit 0, incl. "HTTP door answered (25 tools, SQL clean), SIGTERM exit 0"
  — now an ASSERTED fact (P-06). The probe sends NO Origin and passed
  unmodified ⇒ F-01 compat row (b) holds on the packaged contract too.

## 5. Process hygiene

All servers/tests/probes spawned by this lane were killed/joined (post-run
`ps` shows zero rism doors owned by this session). PRE-EXISTING stale doors
flagged by the R1 lane still alive (7 procs, started ~01:24-01:43Z, 16.5 h
before this lane began): PIDs 185441 186024 189248 189375 198632 198968
199628 — left untouched (not this lane's children), re-flagged to the
coordinator.

## 6. Ledger (final)

F-01 FIXED(35379f12d7)
F-02 FIXED(2e637fb8ef) — includes P-08 (async resolver, no blocking DNS path)
P-04 FIXED(9cd5d61ac4)
P-05 FIXED(f06897cc32)
P-06 FIXED(fd00e17be0)
P-07 FIXED(5b96dc4c61 + pin fa54eff)
P-10 FIXED(6d59635)
