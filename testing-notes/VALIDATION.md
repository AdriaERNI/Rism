# W1 VALIDATION — architect gate (chain close)

Chain tip merged: refactorer `abf02c2` (= coder `04bc98c` + size-ledger
`3d6a873` + spec `f0d0634`) onto base `9a1ddd7`, worktree branch
`swarmforge-architect`. Diff reviewed in full vs 9a1ddd7 (15 files,
+1416/-18): CLI surface, bind policy, ready-line, settings precedence,
docs, tests, probe — spec-compliant per SPEC.md; no new MCP tools, no
renames, no agent artifacts outside `testing-notes/` (swarmforge/ is
gitignored; SPEC.md commit is sanctioned by SPEC §9.4).

## Defect ledger (rows A–G)

| Row | Status | Evidence / owner |
|---|---|---|
| A installer/PATH | OPEN — VM lane (coordinator QA) | needs CI artifact of chain tip |
| B CLI door | OPEN — VM lane (coordinator QA) | needs CI artifact of chain tip |
| C stdio contract | PASSING (Linux gate) | `packaging/mcp_probe.sh` stdio legs green + `tests/cli_transport.rs` (bare `rism mcp` = shipped stdio, net-flag warning, no listener on --port) |
| D HTTP contract | PASSING (Linux side) | `tests/http_door_stateless.rs` (non-ignored) + `tests/http_door.rs --ignored` vs docker IRIS + probe HTTP leg (25 tools, SIGTERM exit 0). VM leg OPEN — coordinator QA |
| E config precedence | PASSING (Linux side) | unit tests in `src/settings.rs` + `src/mcp/http.rs` (`resolve_transport` names file+value; CLI>env>config>default). VM leg OPEN |
| F uninstall | OPEN — VM lane (coordinator QA) | unchanged by this diff (installer/rism.iss untouched) |
| G gates/PR clean | PASSING (this file) | gate tails below |

## Gate runs (2026-10-10, this worktree)

- `cargo fmt --check` → exit 0, no diff.
- `cargo clippy --all-targets -q -- -D warnings` → exit 0 (stable 1.97.0).
- `rustup run 1.99.0 cargo clippy --all-targets -q -- -D warnings` → exit 0
  (newest local stable; CI-parity check, catches future-std `incompatible_msrv` traps).
- `cargo test -q` → all binaries ok; notable: `http_door_factory_sessions_and_error_path`
  1 passed; cli_transport 6 passed; 17 live tests filtered (ignored).
- docker IRIS `rism-iris` Up (healthy); `curl -u _SYSTEM:SYS
  http://localhost:52773/api/atelier/` → 200.
- `env -i HOME=$HOME PATH=$PWD/target/debug:/usr/bin:/bin bash packaging/mcp_probe.sh`
  → 4x "MCP probe OK" incl. new HTTP leg ("25 tools, SQL clean, SIGTERM exit 0"), exit 0.
- `cargo test -q -- --ignored` → 17 passed (live matrix incl. `http_door.rs`
  live-binary roundtrip), 0 failed, exit 0.

## Size ledger (independent re-measure, this worktree, Linux release)

`cargo build --release` at chain tip abf02c2 measured here (Linux, this
worktree): 11,115,648 B. Coder baseline at 9a1ddd7 (SPEC §4): 9,378,168 B →
delta +1,737,480 B (+18.5%), consistent with the coder's +1,660,624 B
(+17.7%) ledger at 04bc98c (build variance ~0.07 MB). No feature gate, per
SPEC §4 decision rule.

## Spec-compliance notes (no action needed; for the coordinator/PR body)

1. `legacy_session_mode(true)` is pinned explicitly; rmcp 3.4.1 default is
   also true (tower.rs:192) — deliberate anti-flip pin, matches SPEC §2.
2. Host-header guard: kept (localhost/127.0.0.1/::1) on loopback binds,
   disabled only under `--allow-all-interfaces` — matches SPEC §2 and the
   banner/rule in constitution (HTTP is opt-in, no-auth stated on stderr
   + docs + `--help`).
3. `RismMcp` Clone factory confirmed against vendored rmcp source; no Arc
   refactor was needed (SPEC §9.1 stands).
4. rmcp declares rust-version 1.88 vs Rism MSRV 1.85 — pre-existing
   (stdio already pulls rmcp), unchanged, do not "fix" (SPEC §9.5).
5. CI matrix: no additions required — the existing live-smoke leg already
   runs mcp_probe.sh (now incl. HTTP) and --ignored legs run in the
   architect gate; Windows installer leg builds the new binary unchanged.

## Handoffs to coordinator

- Fold chain tip onto `feat/windows-mcp-http`, push, open PR; paste the
  size ledger line above and rows A/B/D-VM/F as the VM acceptance matrix
  to execute with the NEW CI artifact (never the 0.2.1 staged baseline —
  it lacks the door).
