# Rism

**Prism with Rust** — a fast, single-binary companion for **InterSystems IRIS**: a
command-line tool and an **MCP server** (Model Context Protocol, for AI agents) that
talk to your IRIS instance over the **Atelier REST API**. No server-side code is ever
uploaded by Rism itself, no agents, no `%Studio` globals — everything travels through
the documented REST + WebSocket endpoints your IRIS already exposes.

![CI](https://github.com/AdriaERNI/Rism/actions/workflows/ci.yml/badge.svg)
![Release](https://img.shields.io/github/v/tag/AdriaERNI/Rism?label=release)
![license: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)

## Why Rism

| | Rism |
|---|---|
| **Distribution** | one static binary (Linux, Windows, macOS) — no Python, no runtime installs |
| **Two doors, one core** | `rism <command>` for humans and scripts; `rism mcp` for AI clients (Claude Desktop, Hermes, any MCP client) |
| **Zero footprint on the server** | every tool rides the Atelier API (`/api/atelier/…`) and the terminal WebSocket — nothing is installed inside IRIS |
| **Prism parity** | behavior-matched port of [Prism](https://github.com/AdriaERNI/Prism)'s 25 tools, log format, and debugger, in ~8k lines of Rust |
| **Debugger without Studio** | full DBGP control (breakpoints, stepping, variable inspection) from the CLI or an agent session — no GUI needed |

## 25 tools at a glance

Documents · SQL · terminal · tests · debugger · monitoring · host files. See
[MCP Tools](mcp-tools.md) for the full reference with parameters and examples.

## Quick start

```bash
# 1. point Rism at your IRIS (env vars or a config file — see Configuration)
export RISM_IRIS_BASE_URL=http://localhost:52773
export RISM_IRIS_USERNAME=_SYSTEM
export RISM_IRIS_PASSWORD=***   # never stored in the config file

# 2. smoke test
rism info                      # version + namespaces => connectivity OK

# 3. work
rism sql "SELECT TOP 5 Name FROM %Dictionary.CompiledClass WHERE NameSpace='USER'"
rism doc compile MyPackage.MyClass.cls
rism test run MyPackage.MyClassTest
rism debug run '##class(MyPackage.MyClass).Main()' --stop-on-entry
rism monitor                   # live metrics + load score
```

Serving as an MCP server for an AI client is one line in your client config:

```json
{ "mcpServers": { "rism": { "command": "rism", "args": ["mcp"] } } }
```

## Where to go next

- [Getting Started](getting-started.md) — install, first connection, compose.yaml for a local IRIS
- [Configuration](configuration.md) — config file, environment variables, precedence
- [CLI Reference](cli-reference.md) — every command and flag
- [MCP Tools](mcp-tools.md) — all 25 tools with parameters
- [Debugger](debugger.md) — DBGP sessions, breakpoints, stepping
- [Development](development.md) — build, test, CI, release flow
- [Windows Store](windows-store.md) — installer, Store submission, readiness checklist
