# Getting Started

## Requirements

- A reachable **InterSystems IRIS** (or HealthShare/IRIS for Health) instance with the
  Atelier API enabled. Rism's CI smoke-tests against **IRIS Community 2025.3**;
  any version exposing `/api/atelier/` v1–v4 works.
- Credentials for an account with `%All` or at least Studio/Atelier web-service
  permissions (`%Service_Studio`, `%Service_CSP`).

Rism itself needs nothing else: a single binary, no runtime, no database client,
no ODBC/DSN.

## Install

=== From a release ===

Download the archive for your platform from the
[releases page](https://github.com/AdriaERNI/Rism/releases) and put `rism` on your
PATH:

- **Linux / macOS:** `rism-<version>-<os>.tar.gz` → extract, `chmod +x rism`
- **Windows:** `rism-<version>-windows.zip` → extract, or install with
  `rism-<version>-setup.exe` (adds to PATH, Start-menu entry; see
  [Windows Store](windows-store.md))

=== From source ===

```bash
git clone https://github.com/AdriaERNI/Rism
cd Rism
cargo install --path .
```

Rust 1.85+ (edition 2024) is the only build-time requirement.

## First connection

Set three environment variables (or write the config file — see
[Configuration](configuration.md)):

```bash
export RISM_IRIS_BASE_URL=http://localhost:52773   # your IRIS web server
export RISM_IRIS_USERNAME=_SYSTEM
export RISM_IRIS_PASSWORD=***   # read from the environment only, never the config file
```

Verify:

```console
$ rism info
IRIS for UNIX (Ubuntu Server ... 2025.3)  ... on x86-64
Atelier API: v4
Namespaces: USER, %SYS, ...
```

That is a full round trip: discovery (`/api/atelier/`), auth (Basic), and namespace
listing. If it prints, everything else works.

## Local IRIS with Docker (optional)

A `compose.yaml` ships in the repo for a throwaway instance:

```bash
docker compose up -d          # intersystemsdc/iris-community:2025.3, web on :52773
docker exec -it rism-iris iris session iris -U USER
```

The web server listens on **52773** in the Community image (mapped 1:1 by the
compose file and both CI jobs), so `http://localhost:52773` is the base URL. The
CI job uses the exact same image, so behavior you see locally is what CI verifies.

## A first session

```bash
rism doc put MyApp.Hello.cls --file hello.cls   # upload, do NOT compile yet
rism compile MyApp.Hello.cls                     # compile when ready
rism exec 'write "hi"'                           # run ObjectScript in the terminal
rism sql "select top 3 * from %SYS.ProcessQuery" # query anything
rism doc delete MyApp.Hello.cls                  # clean up
```

## Use with an AI agent (MCP)

Run `rism mcp` as a stdio server in any MCP client:

```json
{
  "mcpServers": {
    "rism": {
      "command": "rism",
      "args": ["mcp"],
      "env": {
        "RISM_IRIS_BASE_URL": "http://localhost:52773",
        "RISM_IRIS_PASSWORD": "<from your secret manager>"
      }
    }
  }
}
```

The agent then sees all 25 tools ([MCP Tools](mcp-tools.md)). Secrets travel via the
client's `env` block — never paste passwords into the chat with your agent.

## Where to go next

- [Configuration](configuration.md) — precedence, file reference
- [CLI Reference](cli-reference.md) — every flag
- [Debugger](debugger.md) — stepping through ObjectScript
