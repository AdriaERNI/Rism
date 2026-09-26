# Configuration

Rism resolves settings with a simple precedence chain:

```
environment variables  >  config file  >  built-in defaults
```

## Config file

Location (per OS):

| OS | Path |
|---|---|
| Linux | `~/.config/rism/config.toml` (honors `$XDG_CONFIG_HOME`) |
| Windows | `%APPDATA%\rism\config.toml` |

Example:

```toml
iris_base_url  = "http://localhost:52773"
iris_username  = "_SYSTEM"
iris_namespace = "USER"
iris_api_version = 0        # 0 = negotiate from GET /api/atelier/
timeout_secs   = 30
sql_max_rows   = 1000
terminal_max_output_chars = 100000
workspace_root = "~/code"   # root for read_file/list_files/run_shell
debug_tools_enabled = true
```

!!! warning "The password does not live here"
    `iris_password` is deliberately **skipped** by the config parser
    (`#[serde(skip)]`). It can only come from `RISM_IRIS_PASSWORD`. A password in
    the file is silently ignored — by design, so config files are safe to
    commit, sync, or paste into bug reports. The same applies to `rism`'s own
    display/logging: it redacts the password everywhere.

## Environment variables

| Variable | Field | Default |
|---|---|---|
| `RISM_IRIS_BASE_URL` | Atelier base URL | `http://localhost:52773` |
| `RISM_IRIS_USERNAME` | username | `_SYSTEM` |
| `RISM_IRIS_PASSWORD` | password | `SYS` *(IRIS factory default)* |
| `RISM_IRIS_NAMESPACE` | default namespace | `USER` |
| `RISM_IRIS_API_VERSION` | API prefix (0 = negotiate) | `0` |
| `RISM_WORKSPACE` | root for host file tools | *(unset = disabled)* |
| `RISM_DEBUG_TOOLS` | `0` hides the 9 `debug_*` tools | `1` |

`RISM_WORKSPACE` also applies to the CLI (`rism cat`, `rism ls`, `rism shell`);
without it those tools refuse to run rather than defaulting to `/`.

## Defaults

Everything unset gives: `http://localhost:52773`, `_SYSTEM`/`SYS`, `USER`, API
negotiation, 30 s timeout, SQL capped at 1 000 rows, terminal output capped at
100 000 chars, debugger enabled, host file tools disabled until you set a
workspace root.

## Per-invocation overrides

CLI connection flags beat both file and env for a single command:

```bash
rism --url https://iris-prod.internal:443 --namespace PROD info
```

MCP tools accept an optional `namespace` parameter per call for the same effect.

## Multiple environments

Because precedence is env > file, keep one config file with your dev instance and
wrap prod in an env block (shell profile, systemd unit, MCP client `env`). On
Windows the Store build reads the same `%APPDATA%\rism\config.toml`, so a config
written by the CLI works unchanged in the MCP server.

## Where to go next

- [CLI Reference](cli-reference.md)
- [MCP Tools](mcp-tools.md)
