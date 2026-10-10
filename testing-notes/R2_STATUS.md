# R2 STATUS — architect gate (final)

Task: rism-r2-fixes. Chain: specifier `0f656c0` → coder (`35379f1..0c0f532779`)
→ refactorer `d74f345e22` (merged into this worktree FF; HEAD at gate time).
Base under review: `cb51ecf`. All gates below ran HERE, in this merged tree —
this run is the authority, not the lanes' own green results.

## 1. Diff review vs cb51ecf

`git diff --stat cb51ecf..HEAD`: 14 files, +2436/−42. Contents:

- `src/mcp/http.rs` — F-01 (Origin allowlist + `enforce_origin_validation`,
  `allowed_origins_for`), F-02 (async `is_loopback_host` via
  `tokio::net::lookup_host` — old `to_socket_addrs` branch was dead code),
  P-07 (ready line moved UP, before the handler negotiate). Plus unit pins.
- `src/settings.rs` — P-10 (alias-map comment reword; `sse` rejected, not
  mapped). No behavior change.
- `docs/cli-reference.md` — F-01 doc lines (Origin validated when present).
- `packaging/mcp_probe.sh` — P-06 (SIGTERM exit asserted for real; old
  `wait ... || true` swallowed the status).
- `tests/http_door_stateless.rs` — P-04 (DELETE 202 + dead-sid 404 pins),
  P-07 pin (`ready_line_precedes_handler_negotiate`, blackhole IRIS).
- `tests/cli_transport.rs` — F-02 integration pins (named loopback binds
  without opt-out; refuse-before-bind).
- `tests/http_door_status_matrix.rs` (NEW) — P-05(a/b/c): HTTP status matrix,
  guard/origin posture toggle with bind, ephemeral ports; + the
  `origin_bypass_matrix_denies_every_browser_shape` pin added by the
  adversarial lane.
- `testing-notes/*` — R1/R2 lane notes (plan, facts, attacks, review).

Nothing outside those buckets: no artifacts, no stray files, no `.swarmforge`
payloads committed. src diff at `d74f345e22` is byte-identical to the coder
tip (refactorer changed tests/docs only) — confirmed by
`git diff 0c0f532779..d74f345e22 -- src/` being empty in the attacks notes.

## 2. Gate suite (this worktree, HEAD = d74f345e22)

| gate | result |
|---|---|
| `cargo fmt --check` | clean (rc 0) |
| `cargo clippy --all-targets -- -D warnings` (default stable 1.97) | 0 warnings, `Finished` rc 0 |
| `rustup run 1.99.0 cargo clippy --all-targets -- -D warnings` | 0 warnings, rc 0 |
| `cargo test -q` | all suites ok — 75+13+1+7+2+3 passed, **0 failed** (17+1 ignored) |
| `cargo test -q -- --ignored` (live, docker rism-iris Up+healthy, `/api/atelier/` → 200 first) | 17 passed (http_door live), 1 passed (binary roundtrip), 17 passed (tool_matrix) — **0 failed** |
| `packaging/mcp_probe.sh` via `env -i HOME=$HOME PATH=target/release:...` (release binary, sha256 `9097ba85...`) | 4 OK lines, rc 0, incl. `SIGTERM exit 0` leg |

Post-run stray check: `ps aux | grep target/{debug,release}/rism` → 0.

## 3. Independent Origin acceptance matrix (architect re-run)

Own harness (raw TCP sockets, separate from coder/refactorer scripts), real
binary `target/debug/rism mcp --transport http --port 0`
(sha256 `f77f429c...`), docker rism-iris backend, `env -i`. Raw output:

```
# binary: target/debug/rism
# sha256: f77f429c483981363fcaab8eb02ef8c0bf32face3dedc9a43d7b68c38e488e6a
# --- loopback bind (default) ---
# door pid=871214 port=45491
OK A1 Origin: https://evil.test                               -> HTTP/1.1 403 Forbidden  [expect 403]
OK A2 Origin absent (native client)                           -> HTTP/1.1 200 OK  [expect 200]
OK A3 Origin: http://127.0.0.1:PORT                           -> HTTP/1.1 200 OK  [expect 200]
OK A4 Origin: http://LOCALHOST:PORT                           -> HTTP/1.1 200 OK  [expect 200]
OK A5 Origin: HTTP://127.0.0.1:PORT                           -> HTTP/1.1 200 OK  [expect 200]
OK A6 Origin: http://localhost:PORT                           -> HTTP/1.1 200 OK  [expect 200]
OK A7 Origin: http://[::1]:PORT                               -> HTTP/1.1 200 OK  [expect 200]
OK A8 Origin: http://::1:PORT (no brackets)                   -> HTTP/1.1 403 Forbidden  [expect 403]
OK A9 same-origin + trailing slash                            -> HTTP/1.1 200 OK  [expect 200]
OK A10 same-origin + path appended                            -> HTTP/1.1 200 OK  [expect 200]
OK A11 Origin: (present, EMPTY)                               -> HTTP/1.1 403 Forbidden  [expect 403]
OK A13 Origin: null                                           -> HTTP/1.1 403 Forbidden  [expect 403]
OK A15 Origin: undefined                                      -> HTTP/1.1 403 Forbidden  [expect 403]
OK A16 Origin: file:///                                       -> HTTP/1.1 403 Forbidden  [expect 403]
OK A17 Origin: chrome-extension://abcdefghij                  -> HTTP/1.1 403 Forbidden  [expect 403]
OK A18 Origin: 127.0.0.1:PORT (schemeless)                    -> HTTP/1.1 403 Forbidden  [expect 403]
OK A19 Origin: http://127.0.0.1 (portless)                    -> HTTP/1.1 403 Forbidden  [expect 403]
OK A20 Origin: http://127.0.0.1:PORT+1                        -> HTTP/1.1 403 Forbidden  [expect 403]
OK A21 Origin: https://127.0.0.1:PORT                         -> HTTP/1.1 403 Forbidden  [expect 403]
OK A22 Origin: http://lοcalhost:PORT (homoglyph)              -> HTTP/1.1 403 Forbidden  [expect 403]
OK A23 Origin: http://evil@127.0.0.1:PORT (userinfo)          -> HTTP/1.1 200 OK  [expect 200]
OK A24 Origin: http://127.0.0.1.:PORT (trailing dot)          -> HTTP/1.1 403 Forbidden  [expect 403]
OK A25 dup Origin allowed THEN evil                           -> HTTP/1.1 200 OK  [expect 200]
OK A26 dup Origin evil THEN allowed                           -> HTTP/1.1 403 Forbidden  [expect 403]
OK A28 evil Origin on GET stream leg                          -> HTTP/1.1 403 Forbidden  [expect 403]
OK A29 evil Origin on DELETE leg                              -> HTTP/1.1 403 Forbidden  [expect 403]
OK A30 evil Origin on PUT                                     -> HTTP/1.1 403 Forbidden  [expect 403]
OK B1 Host: evil.example, no Origin                           -> HTTP/1.1 403 Forbidden  [expect 403]
OK B4 Host: localhost:PORT + same-origin                      -> HTTP/1.1 200 OK  [expect 200]
# SIGTERM exit: 0
# --- allow-all bind (--allow-all-interfaces, loopback host) ---
# door pid=871222 port=37253
   C1 Origin: https://evil.test (present)                     -> HTTP/1.1 403 Forbidden
   C1b Origin: http://127.0.0.1:PORT (present)                -> HTTP/1.1 403 Forbidden
   C2 Origin absent                                           -> HTTP/1.1 200 OK
   C3 Host: evil.example, no Origin                           -> HTTP/1.1 200 OK
# SIGTERM exit: 0
```

Every row matches expectation: all 18 evil/deny shapes 403 on the loopback
bind; the documented-200 rows (A23 userinfo, A25 duplicate-first-wins,
native absent-Origin A2, same-origin variants) reproduce exactly the
refactorer's adjudication — no new breach. Allow-all bind keeps the shipped
posture: deny-every-PRESENT-Origin (C1/C1b 403), absent-Origin 200, Host
guard off (C3). SIGTERM exit 0 on both doors; 0 strays after. Zero-breach:
CONFIRMED independently.

## 4. LEDGER

| fix | what | status |
|---|---|---|
| F-01 | Origin validation on the HTTP door (rmcp allowlist + `enforce_origin_validation`, config-only; docs ride in the same commit) | FIXED(35379f12d7); settings knob DEFERRED — see note |
| F-02 | `is_loopback_host` async `lookup_host` (dead-code branch replaced) | FIXED(2e637fb8ef) |
| P-04 | DELETE 202 + dead-session 404 pins | FIXED(9cd5d61ac4) |
| P-05 | status matrix, guard/origin toggle, ephemeral ports (a/b/c) | FIXED(f06897cc32) + bypass-matrix pin (d74f345e22) |
| P-06 | mcp_probe SIGTERM real exit assertion | FIXED(fd00e17148) |
| P-07 | ready line before handler negotiate | FIXED(5b96dc4c61) + blackhole pin (fa54effe5d) |
| P-10 | alias-map comment reword (`sse` rejected, not mapped) | FIXED(6d5963509d) |

F-01 settings-knob sub-item: DEFERRED — deliberate scope call recorded in
R2_PLAN §3/R2_FACTS: a real plumb is ~30+ lines across 3 files and no
consumer exists (no browser client of this door); `allowed_origins_for` is
the single source. The config-only posture ships; the knob can land later
without changing the wire.

Mutation matrix M1–M8 (adversarial lane): all RED on revert — no pin
vacuous. Re-verified indirectly here: the full test suite at HEAD contains
those pins and they pass against the real server.

## 5. VERDICT

**PR-READY.** All 7 fixes land with pins; fmt/clippy (1.97 + 1.99) clean;
unit + live integration + packaging probe green in this merged tree;
Origin posture independently re-verified with zero breaches; diff contains
only fixes/tests/docs. Fold `d74f345e22` onto PR #18.