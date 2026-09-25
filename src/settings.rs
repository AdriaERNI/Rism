//! Settings with precedence: CLI flags > environment (`RISM_*`) >
//! `<config-dir>/rism/config.toml` > defaults.
//!
//! Mirrors Prism's 28-field loader design, reduced to the fields the first
//! vertical slice needs.

use std::path::PathBuf;

use serde::Deserialize;

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

    /// Config file location: `$XDG_CONFIG_HOME` / platform equivalent.
    #[must_use]
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("eu", "vortexis", "rism")
            .map(|d| d.config_dir().join("config.toml"))
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
}
