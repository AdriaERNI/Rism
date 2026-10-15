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
- Wire names are frozen: the 29 `#[tool]` names in `src/mcp/mod.rs` and
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
