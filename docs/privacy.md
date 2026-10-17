# Privacy Policy

Rism is a local command-line tool and MCP (Model Context Protocol) server for
InterSystems IRIS. This page states exactly what the software does with data.
It is the privacy-policy URL referenced by the Microsoft Store listing
(see [Windows & Microsoft Store](windows-store.md)).

## Telemetry

**None.** Rism contains no analytics, no usage tracking, no crash reporting,
and no phone-home of any kind. The only network code in the binary issues
requests to the IRIS server **you** configure: the Atelier REST API calls in
`src/iris/http.rs` plus the terminal and debugger WebSocket streams in
`src/iris/terminal.rs` / `src/iris/dbgp.rs`, whose URLs are derived from the
same configured server address. Rism never contacts any other host on its own.

## Credentials

Server credentials are stored **locally only**, in the config file documented
in [Configuration](configuration.md):

- Windows: `%APPDATA%\rism\config.toml`
- Linux/macOS: `~/.config/rism/config.toml` (honors `$XDG_CONFIG_HOME`)

They are never transmitted anywhere except to the IRIS server you point Rism
at, as part of authenticating the connection you initiated. Uninstalling Rism
leaves this file in place so your settings survive re-install; deleting the
folder removes every trace of configuration the app keeps.

## Data access

Everything Rism reads or writes on an IRIS system is a direct, synchronous
result of a command or tool call you issued. There are no background jobs,
no caching to remote services, and no data collection. The privacy stance is
identical for every install channel (GitHub Release, package manager, or the
Microsoft Store package): the same binary, no Store-specific instrumentation.

## Source of truth

Rism is open source (AGPL-3.0): <https://github.com/AdriaERNI/Rism>. The
network-behavior claims above are auditable in the repository — every outbound
connection (HTTP and WebSocket) targets only the configured server URL.
