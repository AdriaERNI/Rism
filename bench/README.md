# Bench — code search + graph, public corpus (issue #17, E_SPEC §6)

First benchmark in this repo. It measures the SHIPPED PRODUCT: every
measurement shells out to `target/release/rism` (CLI) — no test harness
shortcuts — and compares against `rg` (ripgrep) on the identical exported
source tree.

## Prereqs

- IRIS dev container up (or any IRIS with Atelier API v2+), reachable via
  the usual settings precedence (`RISM_IRIS_*` env or `~/.config/rism/config.toml`).
- `cargo build --release` (the script uses `target/release/rism` only).
- `rg` on PATH (baseline numbers).
- Outbound HTTPS to `codeload.github.com` (corpus fetch; cached in
  `bench/.work/repos/` so reruns are offline).

## Run

```bash
cargo build --release
python3 bench/run_bench.py            # full protocol; writes bench/RESULTS.md
python3 bench/run_bench.py --keep     # skip cleanup (debugging only)
```

Exit code 0 = every phase completed and cleanup left ZERO `Bench*` docs in
USER. The run prints `CLEANED: N docs deleted` at the end. SIGINT/SIGTERM
trigger the same cleanup path (`finally` + signal handler) — a killed run
never leaves corpus docs on the server.

## Files

| file | role |
|---|---|
| `run_bench.py` | orchestrator: fetch → transform → load → measure → RESULTS → cleanup |
| `inject_markers.py` | deterministic 20-marker transformation + `ground_truth.csv` |
| `graph_truth.csv` | hand-written (spec-verified) inheritance ground truth |
| `CORPUS.md` | repos, SHAs, bytes, license + per-run additions |
| `RESULTS.md` | every measurement table, written by the last full run |

Corpus downloads, the transformed export, and all intermediates live under
`bench/.work/` (gitignored). NEVER commit corpus files.
