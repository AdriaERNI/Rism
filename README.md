# Rism

Rism is **Prism with Rust**.

Prism is an MCP server and CLI for InterSystems IRIS development (SQL, documents,
compilation, debugging, testing, and ObjectScript execution via the Atelier REST
API). Rism is its Rust rewrite.

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

Settings resolve env → `~/.rism/settings.json` → defaults (`_SYSTEM/SYS` @
`http://localhost:52773`, namespace `USER`, API version auto-negotiated).

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
that lock every verified Atelier wire quirk.
