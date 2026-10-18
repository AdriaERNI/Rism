# AGENTS.md

Guidance for AI agents (and humans) working on this repository.
Format: https://agents.md — canonical repo instructions for coding agents.

## What this is

Rism is a Rust rewrite of Prism: a CLI **and** MCP server for InterSystems
IRIS development over the Atelier REST API (zero server-side uploads).
30 tools, one core in `src/tools/`, two doors (`rism <cmd>` CLI,
`rism mcp` stdio or streamable-HTTP). Depth: [docs/index.md](docs/index.md).

## Build & test

Exactly what CI gates (`ci.yml` lint/test jobs — copy the commands verbatim):

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

~100 tests run server-free; 35 live tests are `#[ignore]`d and need IRIS:

```bash
cargo test --test background_jobs -- --ignored
cargo test --test tool_matrix -- --ignored
```

Local IRIS (compose image `intersystemsdc/iris-community:2025.3`):

```bash
docker compose up -d
curl -s -o /dev/null -w '%{http_code}' -u _SYSTEM:SYS \
  http://localhost:52773/api/atelier/    # wait for 200
./target/debug/rism info
```

Docs (pages.yml gate):

```bash
mkdocs build --strict                    # broken links / bad nav fail
python3 scripts/gen-llms.py              # regenerates docs/llms*.txt into docs/
```

Clippy note: GitHub's current stable (1.99-era) flags lints that older
pinned toolchains miss (e.g. `unused_async`, renamed
`unused_async_trait_impl` on nightly — see the allow-comment in
`src/mcp/mod.rs`); run clippy on your **latest** stable before pushing.

## Architecture rules

- `src/tools/` is framework-free: each capability exists exactly once
  there, no clap/rmcp/axum types. `src/cli/` and `src/mcp/` are thin
  adapters over it.
- Wire names are frozen: the 30 `#[tool]` names in `src/mcp/mod.rs` and
  their JSON field names. Adding/removing/renaming one silently is a
  contract break — the CI probe counts tools via `inputSchema` matches.
- Lints are lints: `#![forbid(unsafe_code)]`, `deny(clippy::unwrap_used,
  clippy::expect_used, ...)` (`Cargo.toml` `[lints]`). Handle errors.
- MSRV 1.85, edition 2024 (watch the edition traps: unsafe attributes,
  RPIT lifetime capture).
- The stdio door's byte stream is frozen — `rism mcp` with no flags must
  stay byte-identical to every released version. The streamable-HTTP door
  is the opt-in `--transport http` addition.
- MCP Tasks (SEP-2663) replaced dedicated job tools: long work is
  `execute_command background=true` → `tasks/get` / `tasks/cancel`, never
  new tools.
- Security guards are load-bearing and pinned by wiremock zero-HTTP tests:
  every door that interpolates a `namespace` into a URL must pass
  `names::guard_namespace` BEFORE any request (sql/documents/compile/testing/
  debug doors + the exec REPL config frame); document names from the server
  pass the traversal filter before follow-up fetches (`find_references`'
  macro stage — a second, server-sourced `get_doc` is still a door); every
  argument interpolated into a DBGP text frame must pass `wire_safe` in
  `src/iris/dbgp.rs` (newline/quote in an arg = command injection on the
  debug wire). New tools/paths get the same guards — CI refuses otherwise.
- Debugger semantics: `debug_stop` DETACHES without aborting; resumed or
  attached targets must survive client teardown (tx-id-routed wait +
  detach path). The DBGP agent slot is single-session: live debug tests run
  serially (`--test-threads=1` in CI); a crashed harness poisons the slot
  until the server restarts.

## Windows installer & Microsoft Store (installer/)

- `installer/rism.iss` (Inno Setup 6) is the Store-submitted artifact; the
  PR's `windows-installer.yml` job (silent install → upgrade → uninstall →
  absence census on a hosted runner) is the arbiter for any .iss change.
- Canonical silent parameter string — ONE value, identical in
  `docs/installer-exit-codes.md`, `docs/windows-store.md` and the CI install
  legs: `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /ALLUSERS`
  (uninstall takes the 3-switch form; `/SP-` + `/ALLUSERS` are setup-only).
- Exit-code contract (see docs): 0 ok, 1 second instance refused
  (`SetupMutex=Global\RismSetupMutex` — wired; silent → exit 1 no changes,
  interactive → close-all-instances box), 2/5 cancel paths, 7 disk, 8 reboot,
  100 = upgrade/re-install over existing (via `GetCustomSetupExitCode`).
  Keep every value unique per Store scenario.
- Wizard images are GENERATED, never hand-edited: `scripts/gen-wizard-images.py`
  (run: regenerates `wizard-image.bmp` 606×1158 + `wizard-small.png` 240×240;
  `--check`: verifies committed assets — Inno letterboxes anything that
  violates the 164:314 / square ratios, so check before committing).
- Store facts that must stay TRUE in docs: Microsoft does NOT re-sign
  EXE/MSI submissions (MSIX-only) — publisher-side Trusted-Root-chain
  signing is required for the EXE path; the package URL is versioned and
  immutable after submission.

## Releases & tags

- Stable: Git Flow (release branch → `main` → tag `vX.Y.Z` on the new tip →
  sync). Pre-releases: tag `vX.Y.Z-suffix` (e.g. `v0.2.3-beta1`) on
  `development` — CI's tag build publishes a GitHub **prerelease** (any `-`
  in the tag name triggers it); binaries/installer take the version from
  Cargo.toml (suffix lives only in the tag/release name).
- Tag suffixes must not leak into Linux package versions verbatim: the
  linux-packages leg derives per-manager forms (deb/rpm `~suffix`, arch
  dotless-suffix — pacman ranks `~` ABOVE the final, dpkg below) and
  `bench/version_order_check.sh` (run by CI, extracts ci.yml's derivation
  block so they cannot drift) asserts `previous < prerelease < final` on
  real artifacts. Keep both in sync or the check fails loudly.

## Security notes (hints, not enforcement)

- The HTTP door has **no authentication**. Default bind is loopback;
  a non-loopback `--host` is refused before any socket is created unless
  `--allow-all-interfaces` is passed.
- Origin validation is port-pinned to the door's own origin on loopback
  binds; with `--allow-all-interfaces` the allowlist is empty, which
  denies **every present** Origin (absent-Origin native clients pass).
- Host-header guard protects loopback binds; the allow-all flag disables
  it with the bind it guards.

## PR rules

- `development` is the daily branch and is protected; `main` only receives
  release merges. Never push to either directly; open PRs (Git Flow:
  [docs/development.md](docs/development.md)).
- Conventional commit types matter — git-cliff builds release notes from
  `feat:`/`fix:` and filters the rest.
- Docs changes must keep `mkdocs build --strict` green; claims in docs
  must match code (a drift gate runs on Windows CI).

## Deep links

- Tools/parameters: [docs/mcp-tools.md](docs/mcp-tools.md)
- Flags/commands: [docs/cli-reference.md](docs/cli-reference.md)
- Settings/env: [docs/configuration.md](docs/configuration.md)
- Client wiring (Claude, Cursor, VS Code): [docs/ai-agents.md](docs/ai-agents.md)
- CI map + release flow: [docs/development.md](docs/development.md)
