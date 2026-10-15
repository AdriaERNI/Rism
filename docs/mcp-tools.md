# MCP Tools

Rism serves **30 tools** over JSON-RPC 2.0 on two doors (`rism mcp`):
**stdio** (default) and **streamable-HTTP** (`rism mcp --transport http`,
at `/mcp`). Parameters marked
`*` are required. Namespace-scoped tools accept an optional `namespace`
(override the configured default) unless noted — `get_server_info` and the
host tools (`run_shell`, `read_file`, `list_files`) take none. Every tool's behavior is identical to the CLI path — same
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
`destructiveHint: true`, `idempotentHint: false`) so clients can show names
and decide confirmation policy
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
- `tasks/update` always answers with a JSON-RPC error: rism tasks never
  request input (a terminal `read` inside a background job is auto-answered
  with an empty line), so there is no outstanding input request to deliver.
- Task ids are unguessable by protocol MUST: shape `rism-{pid:hex}-{16 hex
  random}` — never a sequence.

## HTTP door wire contract

The streamable-HTTP door (`rism mcp --transport http`) is plain HTTP at
`/mcp`; harnesses can rely on this status matrix (pinned by
`tests/http_door_status_matrix.rs`), evaluated in guard order
(method → Accept/Content-Type → session → Origin/Host → body size):

| Situation | Status |
|---|---|
| POST without `Accept: application/json, text/event-stream` | `406` |
| GET/DELETE without `Mcp-Session-Id` | `400` |
| POST/GET with an unknown session id | `404` |
| DELETE with an unknown session id | `202` (idempotent terminate) |
| PUT | `405` |
| POST with a non-JSON `Content-Type` | `415` |
| Body > 4 MiB | `413` (rmcp default cap) |
| Present `Origin` not matching the door's own origin | `403` |

Sessions are stateful (`Mcp-Session-Id` issued at `initialize`) and idle out
after **300 s** keep-alive; a SIGTERM/Ctrl+C shuts the door down with a clean
process exit. No authentication exists on this door — treat the bound address
as the trust boundary ([CLI Reference](cli-reference.md) danger box).

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

Operate on the machine **running Rism**. `read_file` / `list_files` are
rooted at `workspace_root` / `RISM_WORKSPACE`, refuse to run when no root is
configured (they never default to `/`), and reject traversal outside the
root. `run_shell` is not root-gated: it starts in the workspace root if set,
otherwise the process working directory.

| Tool | Params |
|---|---|
| `read_file` | `path`\* — text; binary detected and rejected |
| `list_files` | `path`, `pattern` (glob), `max_results` |
| `run_shell` | `command`\*, `cwd`, `timeout_secs` (default 30 s, clamped to [1, 3600]) — `bash` on Unix, PowerShell on Windows |

## Code search

### `search_code` — `query`*

Server-side text/regex search over source documents via the Atelier
`action/search` endpoint — no local checkout needed. `query` is
URL-encoded and treated as a regex by default; `regex: false` switches to
a literal full-text scan. Optional: `namespace`, `documents` (masks,
default `["*.cls","*.mac","*.int","*.inc"]`), `include_system`,
`include_generated`, `case_sensitive` / `whole_word` (sent only when
set), `max_results` (default `search_max_results` = 200; the server
honours higher values on IRIS 2025.3 and flags `truncated` when it
stopped at `max`). Hits come back grouped per
document with member/attribute/line attribution (`line` normalized to a
number; label-style tokens land in `label`). **Scan semantics**
(probed on IRIS 2025.3): regex mode scans class *metadata* (member and
attribute names); body text — comments, string literals — is only found
in literal mode (`regex: false`). See [Code Search](code-search.md).

## Class graph

`%Dictionary`-backed reference graph (SQL + a parse-on-fetch scanner — no
new endpoints).
Class names are validated before any HTTP call: an unsafe name is
refused with a config error and zero requests made. The `namespace`
parameter rides the URL path, so it is validated by the same rule
(letters/digits/`_`/`-`, optional leading `%`) — an unsafe namespace is
also refused before any request.

### `find_subclasses` — `class`*

Bounded BFS over `ClassDefinition.Super` edges. `depth` counts levels
(1 = direct children only; default `graph_depth` = 10, max 64). Each hit
carries `{name, super, level}` (level = BFS distance); the queried class
itself is excluded; `truncated` marks a depth/budget-capped walk.

### `class_members` — `class`*

Own compiled members from `%Dictionary.CompiledMethod/Property/
Parameter/XData/Query/Trigger/Index`: methods (with `class_method`,
`sql_proc`, `formal_spec`, `return_type`, `origin`), properties (`type`,
`collection`), parameters (`type` — the raw default is not exposed),
XData names, queries (`formal_spec`, `sql_name`, `sql_view`, `type`),
triggers (`event`, `time`, `code_mode`), indices (`type`, `properties`).
`kind=all` covers every structural table plus queries/triggers/indices;
`kind` also narrows to `queries` / `triggers` / `indices` /
`procedures` (the last reads `CompiledMethod WHERE SqlProc = 1` — no
`CompiledProcedure` table exists on 2025.3). `include_inherited: true`
drops the `Origin = class` filter (inherited rows then carry their declaring
class in `origin`, when the dictionary reports one). A name with no
`%Dictionary` row — typo or alias (`%RegisteredObject` →
`%Library.RegisteredObject`) — is an error, not zero counts.

### `find_references` — `target`*

Call-graph references to `"Package.Class.Member"`, `"Package.Class"`, or
`"^Routine"`. IRIS 2025.3 has no server cross-reference store, so the tool
narrows candidate docs with a literal `action/search` + `%Dictionary` SQL
over the embedded-code columns, fetches each doc ONCE, and classifies call
sites with a built-in source scanner: resolved `edges`
(`{doc, member, line, text, via}` — `via` ∈ `class_call`, `self_call`,
`routine_do`, `super`, `computed`, `trigger`) plus an honest `unresolved`
bucket (`oref_call`, `dynamic_string`, `macro` hints) that is never
silently dropped. `include_system` (slower — a system-wide scan measured
above the 2 s gate) and `include_generated` (compiled `.int` docs:
macro-expanded call sites but heavy generated noise) default to false;
`max_results` defaults to `search_max_results`. A class/member target with
no dictionary row answers an honest not-found. Routine edges are found by
`^Routine` targets only — a member query never contains a `Do ^` edge by
construction. See [Code Search](code-search.md) for what the scanner
catches and what stays impossible.

### `class_dependencies` — `class`*

Dependency edges in either direction. `direction` ∈ `depends_on` /
`used_by` / `both` (default `both`). `depends_on` is the 1-hop
dictionary view: the raw `DependsOn` field split + classified
(`class` / `routine` / `other`), merged with the exact `Super` list
(a superclass is a dependency even when `DependsOn` omits it — probed:
on 2025.3 `DependsOn` is empty for pure-inheritance classes).
`used_by` walks `DependsOn`/`Super` back-references (bounded BFS,
`depth` — default 1, max 64 — applies to this walk only). It is an
IMPACT HINT, not complete: 2025.3 does not record body call sites
(`##class(X).M()`) in `DependsOn`, so classes that only CALL X are
missed — `find_references` is the complete body-call-site answer; the
literal `search '##class(X)' --no-regex` remains the fast text check.

## Typical agent flows

**Edit a class:** `get_document` → (agent edits) → `put_and_compile` →
`execute_sql "SELECT … FROM %Compiler.State"` or `compile_documents` to verify.

**Debug a failure:** `run_tests` → on failure, `debug_start` with the failing
method as `target` + `stop_on_entry=true` → `debug_stack` / `debug_variables` /
`debug_inspect` at each stop → `debug_stop`.

**Local dev loop:** `read_file` the working copy → `put_document` → iterate →
`compile_documents` when green.
