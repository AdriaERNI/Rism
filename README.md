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
`get_server_info`. All namespace-scoped ones accept an optional `namespace`
override.

## Tests

`cargo test` runs unit tests plus wiremock integration tests (`tests/api.rs`)
that lock every verified Atelier wire quirk.
