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

### `rism exec` (no argument) — interactive terminal
A persistent REPL over one terminal session: variables and `do` state carry
between lines, exactly like the native IRIS terminal. Line editing is
bash-like: **↑/↓ history**, **Ctrl+R** reverse search, **Ctrl+D** exit,
**Ctrl+C** interrupts the running command **server-side** (protocol
interrupt — the ObjectScript child unwinds with `<INTERRUPT>` in
milliseconds, exactly like a native terminal break; `exit`/`quit` also
leave). A `read` inside a command prompts you interactively (like a real
terminal); with piped stdin the answer is simply the next line. History
persists across runs in `<config dir>/rism/terminal_history.txt`
(`%APPDATA%\rism\` on Windows); duplicates and space-prefixed lines are not
recorded, so ` command` hides a line from history, same as bash.

Piped stdin (`echo 'write 1' | rism exec`) runs the same loop non-interactively:
streaming output, no editing/history — CI-safe.

Flags: `--timeout <SECS>` (per command), `--namespace`, `--format`.

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
Serve JSON-RPC 2.0 (all 25 tools). Used by MCP clients. No flags = **stdio**,
byte-identical to every released version. `RISM_DEBUG_TOOLS=0` removes the 9
`debug_*` tools from discovery.

| Flag | Default | Meaning |
|---|---|---|
| `--transport <stdio\|http>` | `stdio` | Door to serve. `http` = streamable-HTTP at `/mcp`; aliases `streamable-http` / `streamable_http`, any case (env: `RISM_MCP_TRANSPORT`) |
| `--port <N>` | `3000` | TCP port for the http door; `0` = ephemeral — the actual port is printed on the stderr ready line (env: `RISM_MCP_PORT`) |
| `--host <H>` | `127.0.0.1` | Bind host for the http door; non-loopback requires `--allow-all-interfaces` |
| `--allow-all-interfaces` | off | Deliberate opt-out that binds a non-loopback host (prints a warning banner) |

`--port`/`--host` with the stdio transport print one stderr warning and are
ignored. Precedence: CLI flag > env > `config.toml` (`mcp_transport`,
`mcp_port`, `mcp_host`) > defaults.

!!! danger "No auth on the HTTP door"
    `rism mcp --transport http` has **no authentication**: every process that
    can reach the bound address can drive all 25 tools, including `run_shell`,
    file tools, and the debugger. Bind loopback only unless you fully own the
    network boundary.

```bash
rism mcp                                            # stdio (default, unchanged)
rism mcp --transport http --port 3000               # streamable-HTTP on 127.0.0.1:3000/mcp
RISM_MCP_TRANSPORT=http RISM_MCP_PORT=0 rism mcp    # env form, ephemeral port
```
