# MCP Tools

Rism serves **25 tools** over JSON-RPC 2.0 / stdio (`rism mcp`). Parameters marked
`*` are required. All tools accept an optional `namespace` (override the configured
default) unless noted. Every tool's behavior is identical to the CLI path — same
core, two doors.

Set `RISM_DEBUG_TOOLS=0` to remove the 9 `debug_*` tools from `tools/list`
(attaching pauses live IRIS jobs; some deployments disable them).

## Server & diagnostics

### `get_server_info`
Product version, Atelier API version, namespace list. Also the connectivity probe.
No parameters.

## Documents

### `list_documents`
| Param | Notes |
|---|---|
| `filter` | name **prefix** (starts-with) |
| `filetypes` | e.g. `["cls"]` |
| `count` | max results |

### `get_document` — `name`*
Source as text lines + metadata (ts, owner, size).

### `put_document` — `name`*, `content`*
Upload **without** compiling — the content comes from the client, nothing is ever
generated on the server. Optional: `flags`, `ignore_conflict`.

### `put_and_compile` — `name`*, `content`*
Upload then compile in one round trip.

### `compile_documents` — `names`*
Compile documents **already on the server** (zero uploads — this is the tool for
CI-clean servers). Optional `flags`.

### `delete_document` — `name`*

## SQL & terminal

### `execute_sql` — `query`*
Optional `max_rows`.

### `execute_command` — `command`*
ObjectScript command in a terminal session over the WebSocket. Optional
`timeout_secs`.

## Unit tests

### `run_tests` — `test_class`*
Via `%UnitTest.Manager`, no code uploaded. Optional `test_method`, `timeout_secs`.

### `list_tests`
Discover `%UnitTest` classes. `filter` is a **prefix** — `%` wildcards are rejected
by validation (they would be interpreted literally).

### `get_test_results`
Stored history, newest first. Optional `test_class`, `max_runs`.

## Debugger (9 tools)

Full DBGP control without leaving the API session — see [Debugger](debugger.md).

| Tool | Params |
|---|---|
| `debug_list_processes` | `system` (include system-wide jobs) |
| `debug_start` | `target`\*, `stop_on_entry`, `breakpoints` |
| `debug_attach` | `pid`\* |
| `debug_step` | `session_id`\*, `action` (`step_into`/`step_over`/`step_out`/`run`/`break`/`stop`) |
| `debug_stack` | `session_id`\* |
| `debug_variables` | `session_id`\*, `context` (`private`/`public`/`class`), `stack_level` |
| `debug_inspect` | `session_id`\*, `expression`\* |
| `debug_breakpoints` | `session_id`\*, `action` (`list`/`set`/`remove`/`enable`/`disable`), `breakpoint`, `id` |
| `debug_stop` | `session_id`\* |

## Monitoring

### `monitor_system`
Scored snapshot: overall + cpu/memory/disk/process, with units and per-category
detail. `include_raw_metrics` returns the raw vector too.

## Host file tools (agent-side)

Operate on the machine **running Rism**, rooted at `workspace_root` /
`RISM_WORKSPACE`. Path traversal outside the root is refused. Disabled when no
root is configured.

| Tool | Params |
|---|---|
| `read_file` | `path`\* — text; binary detected and rejected |
| `list_files` | `path`, `pattern` (glob), `max_results` |
| `run_shell` | `command`\*, `cwd`, `timeout_secs` — `bash` on Unix, PowerShell on Windows |

## Typical agent flows

**Edit a class:** `get_document` → (agent edits) → `put_and_compile` →
`execute_sql "SELECT … FROM %Compiler.State"` or `compile_documents` to verify.

**Debug a failure:** `run_tests` → on failure, `debug_start` with the failing
method as `target` + `stop_on_entry=true` → `debug_stack` / `debug_variables` /
`debug_inspect` at each stop → `debug_stop`.

**Local dev loop:** `read_file` the working copy → `put_document` → iterate →
`compile_documents` when green.
