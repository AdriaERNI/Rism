# MCP Tools

Rism serves **25 tools** over JSON-RPC 2.0 / stdio (`rism mcp`). Parameters marked
`*` are required. All tools accept an optional `namespace` (override the configured
default) unless noted. Every tool's behavior is identical to the CLI path — same
core, two doors. One documented exception: document writes enforce conflicts by
default on the MCP door (`ignore_conflict: false`), while the CLI defaults to
ignoring them — pass `ignore_conflict: true` to match `rism doc put`'s behavior.
The stricter default is deliberate: an agent cannot interactively resolve a
clobbered server copy.

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

`execute_command` ships an MCP **annotation** (`title: "Run ObjectScript"`,
`destructiveHint`) so clients can show names and decide confirmation policy
without guessing — ObjectScript can mutate data.
Program/runtime errors (`<SYNTAX>`, `<NOROUTINE>`, `<INTERRUPT>`) come back
as terminal **output** with a successful call, exactly as a real terminal
echoes them — read `output`, not just `isError`.

### `execute_sql` — `query`*
Optional `max_rows`.

### `execute_command` — `command`*
ObjectScript command in a terminal session over the WebSocket. Optional
`timeout_secs` (clamped to 7 days — an oversized or absurd value never
hangs; `0` means immediate timeout on this synchronous door, "use the
default" on the background door). Commands containing `read` are answered
with an empty line (nothing is typing); Ctrl+C semantics do not apply —
use `timeout_secs`.

#### Background execution (MCP Tasks)

For long-running work (loops, batch methods, imports) that would blow a
request timeout, pass `background: true`: instead of blocking, the call
answers immediately with a **task handle** (`resultType: "task"`, a
`taskId`, status `working`). This is the MCP Tasks extension
(SEP-2663) — a protocol mechanism, not extra tools:

- **Poll** with `tasks/get` (`taskId`): status (`working` → `completed` /
  `cancelled` / `failed`) plus a `statusMessage` with the streamed byte
  count while running (`lastUpdatedAt` reflects state changes, not streamed
  frames, so it stays at `createdAt` while a task works). Finished output
  is served in the `result` of a `completed` task — identical to what the
  synchronous call would have returned (same terminal outcome object, so
  frame joins and truncation markers match byte for byte).
- **Stop** with `tasks/cancel` (`taskId`): sends the terminal protocol's
  `interrupt` — a real server-side break: the ObjectScript child unwinds
  with `<INTERRUPT>` within milliseconds and the partial state stays as
  executed (same mechanism as the VS Code lite terminal's Ctrl+C).
  Cancelling a finished task is a safe no-op: `tasks/cancel` acknowledges
  (intent signal only); the `-32602` error is reserved for unknown/expired
  ids.
- Finished tasks are retained **30 minutes from completion** (`ttlMs` is
  advertised from creation per SEP-2663, so the server may retain slightly
  longer than the advertised value); after that the id is expired and
  `tasks/get` answers with a clean error. Concretely: retention is measured
  from completion, so a task that ran past the TTL window is retained
  LONGER than `ttlMs` advertises — the server never discards a task earlier
  than `createdAt + ttlMs` (the MUST half), and a running task is bounded
  only by its own `timeout_secs` (≤ 7 days). A running task ends at its
  `timeout_secs` wall-clock cap regardless — the background default is
  1 hour (higher than the sync default; a task is detached precisely
  because it outlives request timeouts). An explicit `timeout_secs: 0`
  means "use the default", not an instant timeout; any other explicit
  value skips the 1-hour floor. All values — explicit or default — are
  clamped to 7 days (`MAX_TIMEOUT_SECS`), so an absurd `timeout_secs`
  can never hang the server (issue #16). Up to 16 concurrent background
  tasks; the 17th start is refused actionably.
- `background=true` requires a client that declared the Tasks extension
  (`io.modelcontextprotocol/tasks`) for a protocol version that defines it
  — 2026-06-30 or later (SEP-2663). With the classic `initialize`
  handshake the SDK in use negotiates at most 2025-11-25, under which the
  extension key MUST be treated as absent; declare it per-request instead
  (`_meta` with `io.modelcontextprotocol/protocolVersion: "2026-07-28"`
  and the client capabilities — the SEP-2575 inline lifecycle).
  Non-declaring clients get an honest tool-level error telling them to run
  synchronously — never a silent timeout.

## Unit tests

### `run_tests` — `test_class`*
Via `%UnitTest.Manager`, no code uploaded. Optional `test_method`,
`timeout_secs` (clamped to 7 days).

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
