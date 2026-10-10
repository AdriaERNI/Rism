//! Settings with precedence: CLI flags > environment (`RISM_*`) >
//! `<config-dir>/rism/config.toml` > defaults.
//!
//! Mirrors Prism's 28-field loader design, reduced to the fields the first
//! vertical slice needs.

use std::path::PathBuf;

use serde::Deserialize;

/// Which MCP door `rism mcp` serves. The CLI flag, `RISM_MCP_TRANSPORT`, and
/// the config value all resolve through one alias map (Prism parity:
/// `http` = streamable-HTTP; both `streamable-http`/`streamable_http`
/// spellings are accepted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    /// Line-delimited JSON-RPC over stdin/stdout (the shipped default).
    Stdio,
    /// Streamable-HTTP server at `/mcp` (opt-in; no auth on the wire).
    Http,
}

impl McpTransport {
    /// Alias map shared by every input tier. Unknown values yield `None` so
    /// the caller decides between a clap error (CLI/env) and a config error.
    #[must_use]
    pub fn parse_alias(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stdio" => Some(Self::Stdio),
            "http" | "streamable-http" | "streamable_http" => Some(Self::Http),
            _ => None,
        }
    }

    /// Precedence merge for the config tier: a CLI/env value (already parsed)
    /// wins; otherwise the config string is resolved through the alias map.
    ///
    /// # Errors
    /// A message naming the offending value when config selects no known
    /// transport (the caller adds the file path).
    pub fn resolve(cli: Option<Self>, config: &str) -> std::result::Result<Self, String> {
        match cli {
            Some(t) => Ok(t),
            None => Self::parse_alias(config).ok_or_else(|| {
                format!("mcp_transport = \"{config}\" is not a known transport (stdio, http)")
            }),
        }
    }
}

/// Rism runtime settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Atelier API base, e.g. `http://localhost:52773`.
    pub iris_base_url: String,
    /// IRIS username.
    pub iris_username: String,
    /// IRIS password (never displayed by Debug).
    #[serde(skip)]
    pub iris_password: String,
    /// Default namespace for tools that accept `namespace: Option<String>`.
    pub iris_namespace: String,
    /// Atelier API version prefix; 0 = negotiate from `GET /api/atelier/`.
    pub iris_api_version: u8,
    /// HTTP timeout for every request.
    pub timeout_secs: u64,
    /// Default SQL max rows.
    pub sql_max_rows: u32,
    /// Terminal output bound (chars); 0 = unlimited.
    pub terminal_max_output_chars: usize,
    /// Local workspace root for host-side file tools (empty = disabled).
    pub workspace_root: String,
    /// Expose the `debug_*` tools (default on; `RISM_DEBUG_TOOLS=0` hides them —
    /// attaching pauses live IRIS jobs, so some deployments disable them).
    pub debug_tools_enabled: bool,
    /// MCP door for `rism mcp`: "stdio" (default) or "http" (streamable-HTTP
    /// aliases accepted too). Resolved via [`McpTransport::resolve`], which
    /// keeps bad values a startup ERROR instead of silently falling back.
    pub mcp_transport: String,
    /// TCP port for the http door (0 = ephemeral; the actual port is printed
    /// on the stderr ready line). Ignored by stdio.
    pub mcp_port: u16,
    /// Bind host for the http door; loopback only unless
    /// `--allow-all-interfaces` is given. Ignored by stdio.
    pub mcp_host: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            iris_base_url: "http://localhost:52773".to_string(),
            iris_username: "_SYSTEM".to_string(),
            iris_password: "SYS".to_string(),
            iris_namespace: "USER".to_string(),
            iris_api_version: 0,
            timeout_secs: 30,
            sql_max_rows: 1_000,
            terminal_max_output_chars: 100_000,
            workspace_root: String::new(),
            debug_tools_enabled: true,
            mcp_transport: "stdio".to_string(),
            mcp_port: 3000,
            mcp_host: "127.0.0.1".to_string(),
        }
    }
}

impl std::fmt::Display for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Debug would also print the password; keep this redacting variant for
        // `{:?}`-free display paths and tests.
        write!(
            f,
            "Settings {{ base_url: {}, user: {}, ns: {}, api: {}, timeout: {}s, password: <redacted> }}",
            self.iris_base_url,
            self.iris_username,
            self.iris_namespace,
            self.iris_api_version,
            self.timeout_secs,
        )
    }
}

impl Settings {
    /// Load with precedence: env (`RISM_IRIS_*`) > config.toml > defaults.
    /// (CLI flag overrides are applied by the callers on top of the result.)
    ///
    /// # Errors
    /// [`Error::Config`](crate::error::Error::Config) when the config file is
    /// present but unparsable.
    pub fn load() -> crate::Result<Self> {
        let mut s = Self::default();

        if let Some(path) = Self::config_path() {
            if path.is_file() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| crate::error::Error::Config(format!("{}: {e}", path.display())))?;
                s = toml::from_str(&text)
                    .map_err(|e| crate::error::Error::Config(format!("{}: {e}", path.display())))?;
            }
        }

        s.apply_env();
        Ok(s)
    }

    /// Config file location — one convention on every OS:
    /// `<user config dir>/rism/config.toml` (`BaseDirs::config_dir`:
    /// `$XDG_CONFIG_HOME` on Linux, `%APPDATA%` on Windows).
    /// `ProjectDirs` is deliberately NOT used: it nests the organization
    /// name and a `config` subfolder, which would make the Windows path
    /// `%APPDATA%\github\rism\config\config.toml`.
    #[must_use]
    pub fn config_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().join("rism").join("config.toml"))
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("RISM_IRIS_BASE_URL") {
            self.iris_base_url = v;
        }
        if let Ok(v) = std::env::var("RISM_IRIS_USERNAME") {
            self.iris_username = v;
        }
        if let Ok(v) = std::env::var("RISM_IRIS_PASSWORD") {
            self.iris_password = v;
        }
        if let Ok(v) = std::env::var("RISM_IRIS_NAMESPACE") {
            self.iris_namespace = v;
        }
        if let Ok(v) = std::env::var("RISM_IRIS_API_VERSION") {
            if let Ok(n) = v.trim().parse::<u8>() {
                self.iris_api_version = n;
            }
        }
        if let Ok(v) = std::env::var("RISM_WORKSPACE") {
            self.workspace_root = v;
        }
        if let Ok(v) = std::env::var("RISM_DEBUG_TOOLS") {
            self.debug_tools_enabled = !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            );
        }
        if let Ok(v) = std::env::var("RISM_TERMINAL_MAX_OUTPUT_CHARS") {
            // unparseable values keep the config/default (same policy as
            // RISM_IRIS_API_VERSION); lets contract tests pin truncation
            // at a tiny bound instead of the 100k default
            if let Some(n) = Self::parse_env_usize(&v) {
                self.terminal_max_output_chars = n;
            }
        }
    }

    /// Trimmed positive-integer env value; `None` on garbage (keeps
    /// config/default — `set_var` is unsafe under edition 2024, so tests
    /// pin this pure form instead of the env itself).
    fn parse_env_usize(v: &str) -> Option<usize> {
        v.trim().parse::<usize>().ok()
    }

    /// Override the base URL (used by tests and by `--url`).
    #[must_use]
    pub fn with_base_url(mut self, url: impl AsRef<str>) -> Self {
        self.iris_base_url = url.as_ref().to_string();
        self
    }

    /// True when an env/config path is under our control in tests.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        self.iris_base_url.contains("localhost") || self.iris_base_url.contains("127.0.0.1")
    }

    /// Sanity fields every request needs.
    ///
    /// # Errors
    /// [`Error::Config`](crate::error::Error::Config) on obviously bad values.
    pub fn validate(&self) -> crate::Result<()> {
        if self.iris_base_url.is_empty()
            || !(self.iris_base_url.starts_with("http://")
                || self.iris_base_url.starts_with("https://"))
        {
            return Err(crate::error::Error::Config(format!(
                "iris_base_url must be http(s), got '{}'",
                self.iris_base_url
            )));
        }
        if self.iris_username.is_empty() {
            return Err(crate::error::Error::Config(
                "iris_username is empty".to_string(),
            ));
        }
        if self.iris_namespace.is_empty() {
            return Err(crate::error::Error::Config(
                "iris_namespace is empty".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_dev_container() {
        let s = Settings::default();
        assert_eq!(s.iris_base_url, "http://localhost:52773");
        assert_eq!(s.iris_namespace, "USER");
        assert_eq!(s.iris_api_version, 0);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn password_never_in_display() {
        let s = Settings {
            iris_password: "SECRET".to_string(),
            ..Default::default()
        };
        assert!(!s.to_string().contains("SECRET"));
        assert!(s.to_string().contains("<redacted>"));
    }

    #[test]
    fn base_url_validation() {
        let s = Settings {
            iris_base_url: "ftp://nope".to_string(),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn transport_alias_map_is_the_parity_contract() {
        // Prism parity: http = streamable-http; both spellings; any case.
        assert_eq!(
            McpTransport::parse_alias("stdio"),
            Some(McpTransport::Stdio)
        );
        assert_eq!(McpTransport::parse_alias("http"), Some(McpTransport::Http));
        assert_eq!(
            McpTransport::parse_alias("streamable-http"),
            Some(McpTransport::Http)
        );
        assert_eq!(
            McpTransport::parse_alias("streamable_http"),
            Some(McpTransport::Http)
        );
        assert_eq!(McpTransport::parse_alias("HTTP"), Some(McpTransport::Http));
        assert_eq!(
            McpTransport::parse_alias(" Stdio "),
            Some(McpTransport::Stdio)
        );
        // SSE was never a transport: reject, never silently map.
        assert_eq!(McpTransport::parse_alias("sse"), None);
        assert_eq!(McpTransport::parse_alias(""), None);
    }

    #[test]
    fn transport_resolve_precedence_and_error() {
        // CLI/env tier (Some) always wins, even over a bad config value.
        assert_eq!(
            McpTransport::resolve(Some(McpTransport::Stdio), "http").ok(),
            Some(McpTransport::Stdio)
        );
        assert_eq!(
            McpTransport::resolve(None, "streamable_http").ok(),
            Some(McpTransport::Http)
        );
        let err = McpTransport::resolve(None, "sse").err();
        assert!(err.is_some_and(|e| e.contains("sse") && e.contains("mcp_transport")));
    }

    #[test]
    fn mcp_transport_settings_defaults() {
        let s = Settings::default();
        assert_eq!(s.mcp_transport, "stdio");
        assert_eq!(s.mcp_port, 3000);
        assert_eq!(s.mcp_host, "127.0.0.1");
        // an old config.toml (no new keys) deserializes with these defaults
        let old: Settings = toml::from_str("iris_namespace = \"USER\"").unwrap_or_default();
        assert_eq!(old.mcp_transport, "stdio");
        assert_eq!(old.mcp_port, 3000);
    }

    #[test]
    fn env_parse_terminal_max_output_chars() {
        assert_eq!(Settings::parse_env_usize("64"), Some(64));
        assert_eq!(Settings::parse_env_usize(" 64 "), Some(64));
        assert_eq!(Settings::parse_env_usize("not-a-number"), None);
        assert_eq!(Settings::parse_env_usize("-5"), None);
        assert_eq!(Settings::parse_env_usize(""), None);
        // round-5 edges: absurd overflow, exponent syntax, and the
        // documented "0 = unlimited" all stay panic-free (None/Some, never
        // a loader abort); the live door with these values was probed in
        // round 5 — the server answers tools/list on every one of them.
        assert_eq!(Settings::parse_env_usize("99999999999999999999999"), None);
        assert_eq!(Settings::parse_env_usize("1e3"), None);
        assert_eq!(Settings::parse_env_usize(" 100000 "), Some(100_000));
        assert_eq!(Settings::parse_env_usize("0"), Some(0));
        assert_eq!(Settings::parse_env_usize(" \t7\n"), Some(7));
    }
}
