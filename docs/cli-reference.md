# CLI Reference

Every command works against the configured IRIS instance
([Configuration](configuration.md)). Common flags on nearly all commands:

| Flag | Meaning |
|---|---|
| `--url <URL>` | override `iris_base_url` for this call |
| `--namespace <NS>` | override the default namespace |
| `--format <FMT>` | `text` (default) or `json` — parse-friendly output |
| `-V, --version` | print version |

## Connection

### `rism info`
Server info + connectivity smoke test (product, Atelier API version, namespaces).
No arguments.

## SQL

### `rism sql <QUERY>`
Run one SQL statement.

```bash
rism sql "SELECT TOP 10 Name FROM %Dictionary.CompiledClass"
rism sql --max-rows 50 --format json "SELECT * FROM INFORMATION_SCHEMA.TABLES"
```

Flags: `--max-rows <N>` (default 1000).

## Documents

### `rism doc list`
List documents. Flags: `--filter <PREFIX>` (starts-with on name), `--filetypes <LIST>`,
`--count <N>`.

```bash
rism doc list --filter MyApp --filetypes cls --count 50
```

### `rism doc get <NAME>`
Fetch source. Flags: `--format` (`text`/`json`), `--save <PATH>` (write to a local
file instead of stdout — pairs with `doc put` for read→edit→put loops).

```bash
rism doc get MyApp.Hello.cls --save ./MyApp.Hello.cls
```

### `rism doc put <NAME> --file <PATH>`
Upload **without** compiling (Prism parity: compile is explicit). Flags: `--format`,
`--no-ignore-conflict` (fail on timestamp conflict instead of overwriting).

```bash
rism doc put MyApp.Hello.cls --file ./MyApp.Hello.cls
```

### `rism doc compile <NAME> --file <PATH>`
Upload **and** compile in one call. Flags as `put`, plus `--flags <LIST>` for compile
flags.

### `rism doc delete <NAME>`
Delete a document from the server.

### `rism compile <NAMES>...`
Compile documents that are **already on the server** — no upload.

```bash
rism compile MyApp.Hello.cls MyApp.Other.cls
```

## Terminal / ObjectScript

### `rism exec <COMMAND>`
Run an ObjectScript **command** in an IRIS terminal session over the terminal
WebSocket. Quoting note: single-quote the argument so `$` sequences survive to IRIS;
escape a real double quote as `\"` inside it.

```bash
rism exec 'write "1+1=", 1+1'
rism exec 'do ##class(MyApp.Hello).Greet()'
```

Flags: `--format`.

## Unit tests

### `rism test run <CLASS>`
Run a `%UnitTest` procedure block via the manager (nothing is uploaded).
Flags: `--method <NAME>` (single test method), `--timeout <SECS>`.

```bash
rism test run MyApp.MyClassTest
rism test run MyApp.MyClassTest --method TestHello
```

### `rism test list`
Discover `%UnitTest` classes. Flag: `--filter <PREFIX>` — a **starts-with** prefix
(SQL LIKE `%` is added internally; do not append one).

### `rism test results`
Stored results history, newest first. Flags: `--limit <N>`, `--class <NAME>` filter.

## Debugger

One-shot helpers around the DBGP session engine (details in [Debugger](debugger.md)):

```bash
rism debug ps                              # list debuggable processes
rism debug run '##class(MyApp.Svc).Main()' --stop-on-entry
rism debug attach <pid>
```

`debug run` flags: `--stop-on-entry`, `--class <CLS>`, `--method <M>`, plus the
target as a positional expression.

## Host tools

Local-file and local-shell tools for agents that edit code on the machine running
Rism. Active only with `RISM_WORKSPACE` / `workspace_root` set; traversal outside
the root is refused.

```bash
rism shell "cargo test" --cwd ./proj       # bash here, PowerShell on Windows
rism cat src/main.rs
rism ls "src/**/*.rs"
```

## Monitoring

### `rism monitor`
Live metrics snapshot with per-category scores (overall + cpu/memory/disk/process),
units included. `--raw` prints the raw metric vector instead of the scored table.

## MCP server

### `rism mcp`
Serve JSON-RPC 2.0 over stdio (all 25 tools). Used by MCP clients; no flags.
`RISM_DEBUG_TOOLS=0` removes the 9 `debug_*` tools from discovery.
