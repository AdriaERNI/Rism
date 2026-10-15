# CLI Reference

Every command works against the configured IRIS instance
([Configuration](configuration.md)). Common flags on nearly all commands:

| Flag | Meaning |
|---|---|
| `--url <URL>` | override `iris_base_url` for this call |
| `--namespace <NS>` | override the default namespace |
| `--format <FMT>` | `table` (default) or `json` — `json` is the parse-friendly output |
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

## Code search

### `rism search <PATTERN>`
Server-side text/regex search over source documents (`*.cls`, `*.mac`,
`*.int` unless masked). Regex by default; `--no-regex` switches to a
literal full-text scan (body text — comments, string literals — is only
visible in this mode on IRIS 2025.3; regex mode scans member/attribute
names — see [Code Search](code-search.md)).

```bash
rism search "ValidateOrder" --documents "*.cls"
rism search "TODO: rework" --no-regex --max 50 --format json
```

Flags: `--documents <LIST>` (repeatable or comma list), `--no-regex`,
`--system` (include system docs), `--generated`, `--case-sensitive`,
`--whole-word`, `--max <N>` (default `search_max_results` = 200), plus
the common `--url` / `--namespace` / `--format`. Table output lists
doc, member/attr, line, and matched text; `--format json` mirrors the
`search_code` MCP result exactly.

## Class graph

### `rism graph subclasses <CLASS> [--depth N]`
Bounded BFS over `%Dictionary` Super edges (`depth` default
`graph_depth` = 10, max 64; 1 = direct children only). JSON mirrors
`find_subclasses`.

### `rism graph members <CLASS> [--kind K] [--include-inherited]`
Own (or inherited-inclusive) compiled members. `--kind` ∈ `all`
(default) / `methods` / `properties` / `parameters` / `xdata`.

### `rism graph deps <CLASS> [--direction D] [--depth N]`
Dependency edges: `--direction` ∈ `depends_on` / `used_by` / `both`
(default `both`); `--depth` bounds the `used_by` impact walk (default 1,
max 64). Class names pass the same allowlist as the MCP tools — an
unsafe name is refused before any request.

```bash
rism graph subclasses EnsLib.HL7.Adapter.28 --depth 3 --format json
rism graph deps MyApp.Service --direction used_by --depth 2
```

## Documents

### `rism doc list`
List documents. Flags: `--filter <PATTERN>` (SQL LIKE on names, e.g. `My.%`),
`--filetypes <LIST>`, `--count <N>`.

```bash
rism doc list --filter "MyApp.%" --filetypes cls --count 50
```

### `rism doc get <NAME>`
Fetch source. Flags: `--format` (`table`/`json`), `--save <PATH>` (write to a local
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
Exit code: `0` when every test passes, `1` on failures — CI-friendly.

```bash
rism test run MyApp.MyClassTest
rism test run MyApp.MyClassTest --method TestHello
```

### `rism test list`
Discover `%UnitTest` classes. Flag: `--filter <PREFIX>` — a **starts-with** prefix
(matched with SQL `%STARTSWITH`); do not append a `%` — wildcards are rejected by
validation.

### `rism test results`
Stored results history, newest first. Flags: `--limit <N>`, `--class <NAME>` filter.

## Debugger

One-shot helpers around the DBGP session engine (details in [Debugger](debugger.md)):

```bash
rism debug ps                              # list debuggable processes
rism debug run '##class(MyApp.Svc).Main()' --stop-on-entry
rism debug attach <pid>
```

`debug run` flags: `--stop-on-entry`, `--class <CLS>`, `--method <M>`,
`--offset <N>` (breakpoint line offset in the method, default 1),
`--max-stops <N>` (max stops before forcing completion, default 50), plus the
target as a positional expression.

## Host tools

Local-file and local-shell tools for agents that edit code on the machine running
Rism. `read_file` / `list_files` (`rism cat` / `rism ls`) require
`RISM_WORKSPACE` / `workspace_root` and refuse to run without it (they never
default to `/`); traversal outside the root is refused. `rism shell` runs
regardless — it executes in the current working directory unless the MCP
`run_shell` `cwd` parameter says otherwise.

```bash
rism shell "cargo test"                    # bash here, PowerShell on Windows
rism shell --timeout 600 "cargo build"     # per-command timeout (default 30 s, max 3600 s)
rism cat src/main.rs
rism ls --pattern 'src/**/*.rs'            # --pattern takes the glob; the positional is a directory
```

## Monitoring

### `rism monitor`
Live metrics snapshot with per-category scores (overall + cpu/memory/disk/process),
units included. `--raw` adds the raw metric vector to the scored table.

## MCP server

### `rism mcp`
Serve JSON-RPC 2.0 (all 30 tools). Used by MCP clients. No flags = **stdio**,
byte-identical to every released version. `RISM_DEBUG_TOOLS=0` removes the 9
`debug_*` tools from discovery.

| Flag | Default | Meaning |
|---|---|---|
| `--transport <stdio\|http>` | `stdio` | Door to serve. `http` = streamable-HTTP at `/mcp`; aliases `streamable-http` / `streamable_http`, any case (env: `RISM_MCP_TRANSPORT`). `sse` is **rejected**, never silently mapped (CLI exit 2; a config-file `sse` fails startup naming the file) |
| `--port <N>` | `3000` | TCP port for the http door; `0` = ephemeral — the actual port is printed on the stderr ready line (env: `RISM_MCP_PORT`) |
| `--host <H>` | `127.0.0.1` | Bind host for the http door; non-loopback requires `--allow-all-interfaces`. Hosts that *resolve* to loopback (`localhost`, `ip6-localhost`, `::1`, …) bind without the flag — the resolved answer, not the string, decides |
| `--allow-all-interfaces` | off | Deliberate opt-out that binds a non-loopback host; also denies every present `Origin` and drops the Host-header guard — see the danger box below (prints a warning banner) |

The ready line (`rism MCP server ready (http) - listening on
http://<addr>/mcp`) is printed **at socket-bind time**, before the IRIS
version negotiation: a slow or dead IRIS never delays it, and every tool
call still answers (errors) rather than hanging the door. SIGTERM / Ctrl+C
shut the door down cleanly with the process exit code harnesses expect.

`--port`/`--host` with the stdio transport print one stderr warning and are
ignored. Precedence: CLI flag > env > `config.toml` (`mcp_transport`,
`mcp_port`, `mcp_host`) > defaults. There is **no `RISM_MCP_HOST`** env —
`--host` is the only env-less one.

!!! danger "No auth on the HTTP door"
    `rism mcp --transport http` has **no authentication**: every process that
    can reach the bound address can drive all 30 tools, including `run_shell`,
    file tools, and the debugger. Bind loopback only unless you fully own the
    network boundary. Origin is validated when present (RFC 6454 match on the
    door's own origin — the exact bound port is pinned); native clients may
    omit the header. With `--allow-all-interfaces` there is no knowable
    same-origin, so **every present Origin is denied** (`403`) while
    absent-Origin native clients keep passing, and the Host-header guard
    (DNS-rebinding protection, active on loopback binds) is switched off with
    the same flag.

```bash
rism mcp                                            # stdio (default, unchanged)
rism mcp --transport http --port 3000               # streamable-HTTP on 127.0.0.1:3000/mcp
RISM_MCP_TRANSPORT=http RISM_MCP_PORT=0 rism mcp    # env form, ephemeral port
```

## Exit codes

`0` success · `1` runtime/server error (and `rism test run` on failures) ·
`2` CLI misuse (unknown flag, invalid value — clap contract).
