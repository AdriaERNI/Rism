//! Streamable-HTTP MCP door for `rism mcp --transport http`. Thin adapter:
//! builds the shared handler once, hands rmcp's tower service to an axum
//! router mounted at `/mcp` (Prism parity). No tool logic here.
//!
//! NOTE: this module is part of the lib crate — use `crate::`, never `rism::`.

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::error::Error;
use crate::mcp::RismMcp;
use crate::settings::{McpTransport, Settings};

/// Resolved, validated HTTP-door bind parameters (main dispatch owns the
/// precedence merge; this module owns bind policy + serving).
#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    /// Bind host as given on the CLI/config tier.
    pub host: String,
    /// Bind port; 0 = ephemeral (the ACTUAL port lands in the ready line).
    pub port: u16,
    /// Deliberate opt-out of the loopback-only rule (warns on stderr).
    pub allow_all_interfaces: bool,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            allow_all_interfaces: false,
        }
    }
}

/// True when `host` resolves only to loopback addresses (bind-policy check;
/// an unresolvable host is NOT loopback — the bind error will name it).
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        // Unresolvable host is NOT loopback (refused; the bind error names it).
        Err(_) => host
            .to_socket_addrs()
            .is_ok_and(|mut addrs| !addrs.any(|a| !a.ip().is_loopback())),
    }
}

/// Bind-policy gate: refuse any non-loopback bind without the explicit
/// `--allow-all-interfaces` opt-out. Called before any socket is opened.
///
/// # Errors
/// [`Error::Config`] with the reason when the bind would expose an
/// auth-less door to the network.
fn check_bind_policy(cfg: &HttpServerConfig) -> crate::Result<()> {
    if cfg.allow_all_interfaces || is_loopback_host(&cfg.host) {
        return Ok(());
    }
    Err(Error::Config(format!(
        "refusing to bind the HTTP MCP door to '{host}' without --allow-all-interfaces: \
         this door has NO authentication - every host on the network could drive all \
         25 Rism tools. Use --host 127.0.0.1, or pass --allow-all-interfaces to accept \
         that risk explicitly.",
        host = cfg.host
    )))
}

/// Resolve the transport for the `rism mcp` dispatch: a parsed CLI/env tier
/// (clap already enforced the alias map) merged over the config.toml tier.
///
/// # Errors
/// [`Error::Config`] naming the config value when it selects no known
/// transport (never a silent fallback — a typo must not open stdio).
pub fn resolve_transport(
    cli: Option<McpTransport>,
    config: &str,
    path: Option<&std::path::Path>,
) -> crate::Result<McpTransport> {
    McpTransport::resolve(cli, config).map_err(|e| match path {
        Some(p) => Error::Config(format!("{}: {e}", p.display())),
        None => Error::Config(e),
    })
}

/// Serve the MCP over streamable-HTTP at `/mcp` until cancelled.
///
/// # Errors
/// Bind refusal (non-loopback without the opt-out), address-in-use, or
/// server errors. Always a clean message on stderr — never a panic, never
/// a hang.
pub async fn serve_http(settings: Settings, cfg: HttpServerConfig) -> anyhow::Result<()> {
    // Same stderr-only logging contract as the stdio door: stdout stays
    // empty on this door (logs NEVER on stdout, mcp.md §8.1).
    crate::mcp::init_logging_stderr();

    check_bind_policy(&cfg)?;
    // Bind BEFORE building the handler: a refused/in-use port fails fast
    // without ever contacting IRIS.
    let bind = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| anyhow::anyhow!("rism mcp: cannot bind {bind}: {e}"))?;
    if cfg.allow_all_interfaces {
        // Loud opt-out banner (stderr; the actual port when ephemeral).
        eprintln!(
            "warning: rism MCP HTTP door bound to {} with NO authentication - every \
             process that can reach this address can drive all 25 tools (incl. shell \
             and debugger). Do not expose it beyond a trusted network.",
            match listener.local_addr() {
                Ok(a) => display_addr(a),
                Err(_) => bind.clone(),
            }
        );
    }
    serve_http_on(listener, settings, cfg.allow_all_interfaces).await
}

/// Serve the door on an ALREADY-bound listener (tests bind `:0` themselves
/// and learn the port from `local_addr`). Everything else — factory,
/// sessions, signals, ready line — is the shipped path.
///
/// # Errors
/// As [`serve_http`], minus the bind itself.
pub async fn serve_http_on(
    listener: tokio::net::TcpListener,
    settings: Settings,
    all_interfaces: bool,
) -> anyhow::Result<()> {
    let addr = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("rism mcp: local_addr: {e}"))?;

    // Handler built ONCE: RismMcp is Clone and cheap (IrisClient =
    // Arc<Inner>; ToolRouter clones per-route Arcs + small Arc'd metadata),
    // so the per-request factory is a shallow field clone — no IRIS
    // (re)connect or version negotiation per request.
    // (RismMcp::new's negotiate_version is a best-effort GET: a dead or
    // absent IRIS never fails startup; every tool call must ANSWER.)
    let handle = RismMcp::new(settings).await?;
    let factory = move || Ok(handle.clone());

    let mut config = StreamableHttpServerConfig::default()
        // Stateful `Mcp-Session-Id` sessions — the row-D contract (rmcp's
        // own default; pinned so an upstream default flip can't sneak in).
        .with_legacy_session_mode(true)
        .with_cancellation_token(CancellationToken::new());
    if all_interfaces {
        // A loopback bind keeps rmcp's Host-header guard (DNS-rebinding);
        // the explicit opt-out disables it — the bind it guards is already
        // all-interfaces.
        config = config.disable_allowed_hosts();
    }
    let ct = config.cancellation_token.clone();

    let service: StreamableHttpService<RismMcp, LocalSessionManager> =
        StreamableHttpService::new(factory, Arc::new(LocalSessionManager::default()), config);
    let router = axum::Router::new().nest_service("/mcp", service);

    // Probes/tests parse this exact line for the ACTUAL port (row D).
    eprintln!(
        "rism MCP server ready (http) - listening on http://{}/mcp",
        display_addr(addr)
    );

    let shutdown = signal_shutdown();
    let keepalive = ct.clone();
    tokio::select! {
        r = axum::serve(listener, router).with_graceful_shutdown(async move {
            shutdown.await;
            ct.cancel();
        }) => r?,
        () = keepalive.cancelled() => {}
    }
    tracing::info!("rism MCP server stopped (http)");
    Ok(())
}

/// Ctrl+C works on both platforms (VM row D kills via Ctrl+C); SIGTERM is
/// the Unix service/shell-out kill. Either resolves this future.
async fn signal_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            // Signal handlers cannot install (e.g. sandboxed CI): degrade
            // to Ctrl+C only rather than fail an otherwise fine door.
            Err(_) => {
                tokio::signal::ctrl_c().await.ok();
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
    }
}

/// Ready-line address form: `SocketAddr` Display is already URL-correct
/// (IPv4 `1.2.3.4:port`, IPv6 bracketed `[::1]:port`).
fn display_addr(addr: std::net::SocketAddr) -> String {
    addr.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_policy() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.121.127"));
        assert!(!is_loopback_host("::"));
        // unresolvable is NOT loopback (refused; the bind error will name it)
        assert!(!is_loopback_host("no.such.host.invalid"));
    }

    #[test]
    fn refuse_non_loopback_without_optout() {
        let cfg = HttpServerConfig {
            host: "0.0.0.0".to_string(),
            ..Default::default()
        };
        let err = check_bind_policy(&cfg).err();
        assert!(err.is_some_and(|e| {
            let t = e.to_string();
            t.contains("--allow-all-interfaces") && t.contains("NO authentication")
        }));
        // the opt-out is heard
        let cfg = HttpServerConfig {
            allow_all_interfaces: true,
            ..cfg
        };
        assert!(check_bind_policy(&cfg).is_ok());
    }

    #[test]
    fn transport_resolution_is_the_precedence_contract() {
        // CLI/env tier wins; the config tier error names the file path
        // (the raw resolve() precedence lives in settings.rs tests).
        assert_eq!(
            resolve_transport(Some(McpTransport::Stdio), "http", None).ok(),
            Some(McpTransport::Stdio)
        );
        assert_eq!(
            resolve_transport(None, "streamable-http", None).ok(),
            Some(McpTransport::Http)
        );
        let err = resolve_transport(None, "sse", Some(std::path::Path::new("/x/config.toml")))
            .err()
            .map(|e| e.to_string());
        assert!(err.is_some_and(|e| e.contains("/x/config.toml") && e.contains("sse")));
    }

    #[test]
    fn display_addr_keeps_urls_usable() {
        let v4 = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));
        assert_eq!(display_addr(v4), "127.0.0.1:8080");
        let v6 =
            std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 8080);
        assert_eq!(display_addr(v6), "[::1]:8080");
    }
}
