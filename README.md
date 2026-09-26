<div align="center">
  <img src="logo.svg" width="200" alt="Rism Logo" />

  # Rism

  **Rism is Prism with Rust.**

  [![CI](https://github.com/AdriaERNI/Rism/actions/workflows/ci.yml/badge.svg)](https://github.com/AdriaERNI/Rism/actions/workflows/ci.yml)
  [![Docs](https://github.com/AdriaERNI/Rism/actions/workflows/pages.yml/badge.svg)](https://adriaerni.github.io/Rism/)
  ![MSRV](https://img.shields.io/badge/rust-1.85%2B-blue)
  ![License](https://img.shields.io/badge/license-AGPL--3.0-blue)
  ![IRIS](https://img.shields.io/badge/IRIS-2025.3-orange)
</div>

Rism is **Prism with Rust** — an MCP server and CLI for InterSystems IRIS
with **25 tools** and zero server-side helper code.

Prism is an MCP server and CLI for InterSystems IRIS development (SQL, documents,
compilation, debugging, testing, and ObjectScript execution via the Atelier REST
API). Rism is its Rust rewrite.

## Documentation

Full docs live on **GitHub Pages**: <https://adriaerni.github.io/Rism/> —
getting started, configuration, CLI reference, all 25 MCP tools, debugger,
and the [Windows Store guide](https://adriaerni.github.io/Rism/windows-store/).
Sources in [`docs/`](docs/), built with mkdocs-material (CI-enforced, `--strict`).

## Development environment

A local IRIS Community instance runs via Docker Compose:

```bash
docker compose up -d
```

- Image: `intersystemsdc/iris-community:2025.3` (same tag Prism CI uses)
- Atelier REST API: <http://localhost:52773/api/atelier/>
- Credentials: `_SYSTEM` / `SYS`
- Container name: `rism-iris` (default IRIS port; stop any other local IRIS
  instance first — e.g. Prism's — to free `52773`).

Shut down with `docker compose down` (add `-v` to wipe the data volume).

## Usage

Every capability exists exactly once in `src/tools/` and is exposed through
both doors — CLI and MCP.

### CLI

```bash
rism sql "SELECT Name FROM %Dictionary.ClassDefinition"
rism --format json sql --max-rows 10 "SELECT * FROM %Library.RoutineMgr_StudioOpenDialog"
rism doc list --filter "My.%" --filetypes CLS,RTN
rism doc get My.Class.cls
rism doc compile My.Class.cls --file src/My.Class.cls     # put + compile
cat src/My.Class.cls | rism doc compile My.Class.cls --file -
rism doc put My.Class.cls --file src/My.Class.cls         # upload, no compile
rism doc delete My.Class.cls
rism compile My.Class.cls My.Other.cls                    # compile, no upload
rism exec 'write $ZVERSION,!'                             # ObjectScript via WS terminal
rism test run MyApp.Tests.Calculator                      # %UnitTest (nothing uploaded)
rism test list --filter MyApp
rism test results --limit 5
rism monitor                                          # scored load snapshot
rism debug run '##class(Pkg.Cls).Add(1,2)' --stop-on-entry   # scripted debug
rism debug ps                                               # attach targets
rism debug attach <pid>                                     # peek a live job
rism shell 'git status'                               # local host shell
rism cat src/x.cls                                    # workspace file read
rism ls --pattern '**/*.cls'                          # workspace listing
rism doc get My.Class.cls --save ./local.cls          # server -> local file
rism info                                                 # server version/namespaces
rism --url http://host:52773 --namespace %SYS sql "SELECT 1"
```

Settings resolve env (`RISM_IRIS_*`, `RISM_WORKSPACE`, `RISM_DEBUG_TOOLS=0`
to hide the debugger tools) → `$XDG_CONFIG_HOME/rism/config.toml`
(`~/.config/rism/config.toml`) → defaults (`_SYSTEM/SYS` @
`http://localhost:52773`, namespace `USER`, API version auto-negotiated).
Set `RUST_LOG=debug` for Prism-style REQUEST/RESPONSE tool-call logs on
stderr.

### MCP (stdio)

```bash
rism mcp
```

Tools: `execute_sql`, `list_documents`, `get_document`, `put_document`,
`put_and_compile`, `delete_document`, `compile_documents`, `execute_command`,
`run_tests`, `list_tests`, `get_test_results`, `monitor_system`,
`get_server_info`, `run_shell`, `read_file`, `list_files`, and the debugger
suite (`debug_start`, `debug_attach`, `debug_list_processes`, `debug_step`,
`debug_variables`, `debug_inspect`, `debug_stack`, `debug_breakpoints`,
`debug_stop`). All
namespace-scoped ones accept an optional `namespace` override. Host-side
tools (`run_shell`, `read_file`, `list_files`) act on the machine Rism runs
on; file tools are rooted at `RISM_WORKSPACE` with traversal blocked.

Rism never uploads helper code to the server — everything, including unit
testing, goes through the Atelier REST API and its terminal WebSocket.

## Tests

`cargo test` runs unit tests plus wiremock integration tests (`tests/api.rs`)
that lock every verified Atelier wire quirk. CI additionally smoke-tests the
full tool surface end-to-end against a **fresh** `iris-community:2025.3`
service container (everything it creates is `RismCI.*` and deleted after).

## Releases

Tags `v*` build Linux/Windows binaries and open a GitHub Release from
CI. Stable releases follow Git Flow (`release/vX.Y.Z` from `development` →
PR to `main` → tag); `-pre` tags publish as prereleases.
