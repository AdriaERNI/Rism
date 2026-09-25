# Debugger

Rism drives IRIS's **DBGP** debugger through the Atelier API — over a WebSocket
tunnel to `/api/atelier/…/debug`. No GUI, no Visual Studio, no server-side debug
agents beyond what IRIS ships. Behavior is 1:1 with Prism's debugger.

## Concepts

- A **debug session** wraps one DBGP connection: Rism asks IRIS to launch (or
  attach to) a job, then speaks DBGP commands (`breakpoint_set`, `run`,
  `step_over`, `context_get`, `property_get`, `stack_get`, …).
- Sessions have an **id** (returned by `debug_start`/`debug_attach`); the other
  eight `debug_*` tools take `session_id`.
- A stop suspends the target job. Always end sessions with `debug_stop` /
  `debug_step action=run` — a crashed client that leaves a session open pauses
  the job until it times out. (If a job is orphaned this way, `docker restart`
  or terminating the job is the reliable recovery — `debug_stop` from a new
  session does not reclaim the old one.)

## CLI: run to completion with stops

```bash
rism debug ps                              # candidate jobs (pid, namespace, state)
rism debug run '##class(MyApp.Svc).Main()' --stop-on-entry
```

Prints each stop (`stop 1 at MyApp.Svc.cls:12`) and a summary — the fast way to
see where execution actually goes.

```bash
rism debug attach <pid>                    # snapshot current location, resume
```

## MCP: step-by-step control

```jsonc
// start at entry with a breakpoint
debug_start { "target": "##class(MyApp.Svc).Main()", "stop_on_entry": true,
              "breakpoints": [{ "class": "MyApp.Svc", "method": "Check",
                                 "offset": 3 }] }
// → { "session_id": "1", "stop": { "file": "MyApp.Svc.cls", "line": 1 } }

debug_stack     { "session_id": "1" }
debug_variables { "session_id": "1", "context": "private" }
debug_inspect   { "session_id": "1", "expression": "$zobjclass(\"%Library.String\")" }
debug_step      { "session_id": "1", "action": "step_over" }
debug_breakpoints { "session_id": "1", "action": "enable", "id": 1 }
debug_stop      { "session_id": "1" }
```

## Protocol notes (what Rism handles for you)

- Frames arrive as `len|base64` chunks and are reassembled before XML parsing.
- Every WS text command is `\n`-terminated — IRIS's DBGP agent never replies
  otherwise.
- `context_get` **crashes the agent** unless `stack_get`/`feature_set` ran first;
  Rism always establishes the stack before reading variables.
- Connects are retried (4×) against transient `#6713` session-slot races.
- `xmltree` owns the parsed reply, so extracted values outlive the parse call.

## Caveats

- One active debug session per connection; `debug_stop` frees it.
- Stepping through compiled `%SYS` classes works but stops are server-side —
  keep sessions short on production jobs.
