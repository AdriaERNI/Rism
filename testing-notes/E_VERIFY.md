# E_VERIFY — refactorer lane: independent re-run + delta table (issue #17)

Worktree `.worktrees/refactorer`, branch `swarmforge-refactorer`, coder tip
merged at `13ab1df1e5`. Everything below was executed fresh in THIS worktree
(no numbers copied without a rerun). Container: `rism-iris` (IRIS 2025.3
Build 226U, Atelier v8, localhost:52773, USER), creds
`~/.config/rism/config.toml` (read-only).

## 1. Container hygiene (done FIRST so the bench precision is corpus-only)

- `RismCI.Probe1-5.cls` (specifier's stale live-probe fixtures from
  2026-10-12, the cause of the coder's 20/21 precision note) — checked
  `tests/tool_matrix.rs` + whole repo for references: ONLY prose mentions in
  `testing-notes/E_BUILD.md` and `bench/RESULTS.md`; no test/CI references
  them (`grep -rn "RismCI.Probe" tests/ src/ scripts/ .github/` = 0).
- Deleted via `rism doc delete`; post-check `doc list --filter "RismCI%"` =
  `[]`. `Bench%` = 0, `RismProbe%` = 0 before this lane started.

## 2. Gates (this worktree, fresh)

| gate | command | result |
|---|---|---|
| fmt | `cargo fmt --check` | clean (exit 0) |
| clippy host | `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| clippy 1.99.0 | `rustup run 1.99.0 cargo clippy --all-targets -- -D warnings` | 0 warnings |
| unit+wiremock | `cargo test -q` | exit 0 — lib 83 + api 23 + background_jobs 1(+17 ign) + http_door 7(+1 ign) + cli_transport 2 + http_door_stateless 3 + doctest + tool_matrix 23-ignored; 0 failed (one ERROR log line is the connection-close race test's expected stderr) |
| live `--ignored` | `cargo test --test tool_matrix -- --ignored --test-threads=1` | `test result: ok. 23 passed; 0 failed; 0 ignored` in 10.1 s |
| docs | `mkdocs build --strict` (material via uv) | built, 0 warnings/errors (only the vendor "Warning from the Material team" banner — not a build warning) |

## 3. Bench re-run (`bench_ref_` namespace-prefix rule)

`run_bench.py` has NO prefix argument or env override (checked its CLI: only
`--keep`; doc names `Bench<SHORT>.…` are hardcoded per the frozen §6 transform,
markers `RISM-BENCH-M-NN` fixed in `inject_markers.py`). Per the brief's
"if none, use its documented override" clause, this lane runs the bench with a
mechanical prefix rewrite `Bench* -> BenchRef*` (and `bench_ref_` marker
strings) so coder-run residue could never contaminate this run, then restores
the harness. See §3.1 for the exact patch + revert commits.

| metric | coder (RESULTS.md c05c306dcb) | refactorer rerun | delta | verdict |
|---|---|---|---|---|
| (a) rism p50 range | 289.3–760 ms | PENDING | | latency noise ok |
| (a) rg p50 range | 10.9–12.1 ms | PENDING | | |
| (b) recall | 20/20 | PENDING | | MUST agree |
| (c) precision | 20/21 (95%, stale Probe5) | PENDING | expect 20/20 after §1 | MUST agree w/ §1 |
| (d) cliff max=200 | 200 matches/199 docs trunc=True | PENDING | | MUST agree |
| (d) --max 500 | 500, trunc=True | PENDING | | |
| (e) graph | 11/11 | PENDING | | MUST agree |

## 4. Spec-item → test → result ledger (×2-pass: coder run + this rerun)

PENDING — filled from the wiremock/CLI/MCP attack evidence in E_ATTACKS.md.

## 5. Bugs found & fixed by this lane

PENDING (row per fix: symptom, root cause, fix commit, gate re-run).
