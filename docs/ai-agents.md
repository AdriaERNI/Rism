# AI Agents

How to wire Rism into an AI-agent client, what the agent sees on the wire,
and the security posture of each door. Rism serves the same
[30 tools](mcp-tools.md) over both MCP doors — the client decides which one
it spawns or reaches.

## What Rism exposes

- **30 tools** over JSON-RPC 2.0 — documents, SQL, terminal, tests,
  debugger, monitoring, host files ([MCP Tools](mcp-tools.md)).
- **Two doors**: `rism mcp` (stdio, the default) and
  `rism mcp --transport http` (streamable-HTTP at `/mcp`).
- **MCP Tasks** (SEP-2663) for long-running work — `tasks/get` /
  `tasks/cancel` request handlers, not extra tools.
- **Server instructions** sent at `initialize` — see below.

## Wire-up recipes

### Claude Desktop

Edit `claude_desktop_config.json` (macOS:
`~/Library/Application Support/Claude/`, Windows:
`%APPDATA%\Claude\`). Stdio:

```json
{
  "mcpServers": {
    "rism": {
      "command": "rism",
      "args": ["mcp"],
      "env": { "RISM_IRIS_BASE_URL": "http://localhost:52773",
               "RISM_IRIS_PASSWORD": "<from your secret manager>" }
    }
  }
}
```

### Claude Code

```bash
claude mcp add rism -- rism mcp
# or the HTTP door (long-running server, multiple projects):
rism mcp --transport http --port 3000 &
claude mcp add --transport http rism http://localhost:3000/mcp
```

### Cursor

`.cursor/mcp.json` (project) or the MCP settings panel:

```json
{
  "mcpServers": {
    "rism": { "command": "rism", "args": ["mcp"] }
  }
}
```

### VS Code (Copilot / any MCP extension)

`.vscode/mcp.json`:

```json
{
  "servers": {
    "rism": { "command": "rism", "args": ["mcp"], "type": "stdio" }
  }
}
```

### HTTP URL form (any streamable-HTTP-capable client)

```json
{
  "mcpServers": {
    "rism": { "url": "http://127.0.0.1:3000/mcp" }
  }
}
```

Credentials never go in client config on the HTTP door: the server process
keeps its own (`RISM_IRIS_*` env or the config file —
[Configuration](configuration.md)). Stdio configs may pass an `env` block.

Log noise: Rism's logs go to **stderr without ANSI colors** on both MCP
doors (stdout carries protocol bytes only, HTTP stdout is empty), so a
client's stdout parser never sees log text.

## Server instructions

`initialize` returns an `instructions` string — the server's one-shot
briefing to the model: what each tool is for, the recommended workflow
order, the background-task pattern, and the server's constraints. Keep it
under 2 048 characters — some clients truncate it. The authoritative copy
is the code — `src/mcp/mod.rs` (`with_instructions`) — and it ships verbatim
in every `initialize` response; this page does not restate it (it would
drift).

## Tool annotations

`execute_command` ships `title: "Run ObjectScript"`,
`destructiveHint: true`, `idempotentHint: false`: ObjectScript can mutate
data, so clients should surface it as needing confirmation. Rism itself
never blocks a call on annotations — they are hints for the client's
policy, as are the server instructions.

## Background tasks: client support matrix

`background: true` works only when the client declares the Tasks extension
for a protocol version that defines it (≥ 2026-06-30, SEP-2663). Most SDKs
today negotiate ≤ 2025-11-25 on the classic `initialize` handshake, where
the extension key is inert. The portable form is per-request `_meta`
(SEP-2575 inline lifecycle) declaring
`io.modelcontextprotocol/protocolVersion: "2026-07-28"` plus the client
capabilities. A non-declaring client gets an honest error telling it to
run synchronously with a safe `timeout_secs` — never a silent timeout.

## Security: the HTTP door

The streamable-HTTP door has **no authentication**. Defaults make it safe
for one machine (bind `127.0.0.1`, Origin validated against the door's own
origin with the port pinned, non-loopback binds refused before any socket
is created). With `--allow-all-interfaces` every **present** Origin is
denied (there is no knowable same-origin on a LAN address) while
absent-Origin native clients keep passing — treat the bound address as
trusted-only and read the danger box in
[CLI Reference](cli-reference.md) before widening it.

Sessions on the HTTP door are stateful (`Mcp-Session-Id`); timeouts, body
limits, and refusal codes are in
[MCP Tools](mcp-tools.md#http-door-wire-contract) — the single authoritative
matrix.

Choose HTTP over stdio when several clients share one server process or
the client cannot spawn processes; choose stdio when you want per-project
credential `env` blocks and zero listening ports.

## Machine-readable docs

CI publishes LLM-friendly dumps at the site root:

- <https://adriaerni.github.io/Rism/llms.txt> — index: every page, one summary line each
- <https://adriaerni.github.io/Rism/llms-full.txt> — every docs page concatenated

Both are generated from these sources by `scripts/gen-llms.py` (deterministic,
stdlib-only, runnable locally); never edit them by hand.

## Where to go next

- [Getting Started](getting-started.md) — first connection
- [MCP Tools](mcp-tools.md) — every tool and parameter
- Repo-level agent guidance: `AGENTS.md` at the repository root
