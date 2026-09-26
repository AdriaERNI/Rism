# Changelog

All notable changes to Rism. Versioning follows
[Git Flow](development.md): `release/vX.Y.Z` → `main` → tag.

## [Unreleased]

### Added
- **Linux packages**: native `.deb`, `.rpm`, and pacman (`.pkg.tar.zst`)
  artifacts built from one `packaging/nfpm.yaml` (nfpm) on the release tag,
  installed from the GitHub Release. The Linux binary moved to the fully
  static `x86_64-unknown-linux-musl` target — no glibc floor, runs on any
  supported distro. A new `linux-packages.yml` contract proves
  install → upgrade → uninstall + config preservation inside real
  `debian:12`, `ubuntu:24.04`, `fedora:42`, and `archlinux:latest`
  containers. See [Linux Packages](linux-packages.md).

## [0.1.0] — 2026-09-26

### Added
- **Core**: MCP server (`rism mcp`, rmcp over stdio) + full CLI (`rism <cmd>`)
  sharing one implementation — 25 tools over the Atelier REST API, zero
  server-side code uploads.
- **Documents**: `list/get/put/delete`, upload-without-compile (`put_document`),
  upload+compile (`put_and_compile`), compile-in-place (`compile_documents`).
- **SQL & terminal**: `execute_sql` (capped, JSON or table), `execute_command`
  over the terminal WebSocket.
- **Unit tests**: `run_tests` / `list_tests` / `get_test_results` via
  `%UnitTest.Manager`.
- **Debugger**: DBGP over the Atelier WebSocket — 9 `debug_*` tools +
  `rism debug run|ps|attach`; frame reassembly, `\n`-terminated commands,
  stack-before-context ordering, 4× connect retry. `RISM_DEBUG_TOOLS=0` hides
  them from discovery.
- **Monitoring**: `monitor_system` scored snapshot (overall + cpu/memory/disk/
  process, units in output) + `rism monitor`.
- **Host tools**: `read_file` / `list_files` / `run_shell` rooted at
  `RISM_WORKSPACE` with traversal refusal (PowerShell on Windows).
- **Windows**: Inno Setup installer (`installer/rism.iss`), per-machine
  install + PATH task; CI job installs/upgrades/uninstalls it on
  `windows-latest` against a live IRIS.
- **CI**: lint (fmt + clippy `-D warnings`), unit tests, fresh-IRIS live smoke,
  tagged release builds (Linux/Windows + installer), GitHub Pages.
- **Docs**: this site (`docs/`, mkdocs-material).

### Parity notes
- Log format is byte-compatible with Prism's (`src/logfmt.rs`).
- `test list --filter` is a starts-with prefix (no `%` wildcards) — same
  behavior as Prism's discovery query.

<!-- links for the current dev cycle -->
[0.1.0]: https://github.com/AdriaERNI/Rism/releases/tag/v0.1.0
