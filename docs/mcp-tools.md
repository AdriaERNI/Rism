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

Terminal tools ship MCP **annotations** (`title`, behavior hints), so clients
can show names and decide confirmation policy without guessing:
`execute_command` is `destructiveHint` — ObjectScript can mutate data.
Program/runtime errors (`<SYNTAX>`, `<NOROUTINE>`, `<INTERRUPT>`) come back
as terminal **output** with a successful call, exactly as a real terminal
echoes them — read `output`, not just `isError`.

### `execute_sql` — `query`*
Optional `max_rows`.

### `execute_command` — `command`*
ObjectScript command in a terminal session over the WebSocket. Optional
`timeout_secs`. Commands containing `read` are answered with an empty line
(nothing is typing); Ctrl+C semantics do not apply — use `timeout_secs`.

#### Background execution (MCP Tasks)

For long-running work (loops, batch methods, imports) that would blow a
request timeout, pass `background: true`: instead of blocking, the call
answers immediately with a **task handle** (`resultType: "task"`, a
`taskId`, status `working`). This is the MCP Tasks extension
(SEP-2663) — a protocol mechanism, not extra tools:

- **Poll** with `tasks/get` (`taskId`): status (`working` → `completed` /
  `cancelled` / `failed`) plus a `statusMessage` with the streamed
  character count while running. Finished output is served in the
  `result` of a `completed` task — byte-identical to what the synchronous
  call would have returned.
- **Stop** with `tasks/cancel` (`taskId`): sends the terminal protocol's
  `interrupt` — a real server-side break: the ObjectScript child unwinds
  with `<INTERRUPT>` within milliseconds and the partial state stays as
  executed (same mechanism as the VS Code lite terminal's Ctrl+C).
  Cancelling a finished task is a safe no-op.
- A task's output lives 30 minutes (advertised as `ttlMs`); after that the
  id is expired and `tasks/get` answers with a clean error. Up to 16
  concurrent background tasks; the 17th start is refused actionably.
- `background=true` requires a client that declared the Tasks extension in
  `initialize`. Non-declaring clients get an honest tool-level error
  telling them to run synchronously — never a silent timeout.

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
