//! HTTP plumbing: shared client, `api_url` (%-encoding rule), and the
//! Atelier envelope checker. Everything else in `iris/` goes through here.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::settings::Settings;

/// One configured connection to an IRIS instance. Cheap to clone (Arc).
#[derive(Clone)]
pub struct IrisClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base: String, // no trailing slash
    settings: Settings,
}

/// `status.errors[]` envelope entry.
#[derive(Debug, Deserialize)]
pub struct EnvelopeError {
    /// Error text as reported by the server.
    #[serde(default)]
    pub error: Option<String>,
    /// IRIS error number.
    #[serde(default)]
    pub code: Option<u32>,
}

/// The universal Atelier response envelope.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    /// Envelope-level status.
    #[serde(default)]
    pub status: EnvelopeStatus,
    /// Server console output (compile logs...).
    #[serde(default)]
    pub console: Vec<String>,
    /// Endpoint-specific payload.
    #[serde(default)]
    pub result: Value,
}

/// `status` object of the envelope.
#[derive(Debug, Default, Deserialize)]
pub struct EnvelopeStatus {
    /// Error entries.
    #[serde(default)]
    pub errors: Vec<EnvelopeError>,
    /// Summary text.
    #[serde(default)]
    pub summary: Option<String>,
}

impl IrisClient {
    /// Build a client from settings.
    ///
    /// # Errors
    /// [`Error::Config`] on invalid settings or client-build failure.
    pub fn new(settings: Settings) -> Result<Self> {
        settings.validate()?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(settings.timeout_secs))
            .cookie_store(true)
            .build()
            .map_err(|e| Error::Config(format!("client build: {e}")))?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                base: settings.iris_base_url.trim_end_matches('/').to_string(),
                settings,
            }),
        })
    }

    /// Effective settings.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.inner.settings
    }

    /// Atelier API prefix for a path tail: `/api/atelier/v{N}`.
    #[must_use]
    pub fn api_prefix(&self) -> String {
        let v = if self.inner.settings.iris_api_version == 0 {
            8 // sensible default before negotiation
        } else {
            self.inner.settings.iris_api_version
        };
        format!("/api/atelier/v{v}")
    }

    /// Build the URL for a namespace-scoped Atelier call.
    ///
    /// The `%`-in-namespace rule (documentation/iris-atelier.md §1): `%SYS`
    /// must go on the wire as `%25SYS`. reqwest does not re-encode `%`, so we
    /// pre-encode exactly as Prism's `api_url()` does.
    #[must_use]
    pub fn api_url(&self, namespace: &str) -> String {
        let ns = namespace.replace('%', "%25");
        format!("{}{}/{}", self.inner.base, self.api_prefix(), ns)
    }

    /// GET a URL, returning the parsed envelope.
    ///
    /// # Errors
    /// Transport, status, envelope, or IRIS in-envelope errors.
    pub async fn get(&self, url: &str) -> Result<Envelope> {
        let resp = self
            .inner
            .http
            .get(url)
            .basic_auth(
                &self.inner.settings.iris_username,
                Some(&self.inner.settings.iris_password),
            )
            .send()
            .await?;
        self.absorb(resp, "GET", url).await
    }

    /// POST a JSON body.
    ///
    /// # Errors
    /// Transport, status, envelope, or IRIS in-envelope errors.
    pub async fn post_json(&self, url: &str, body: &Value) -> Result<Envelope> {
        let resp = self
            .inner
            .http
            .post(url)
            .basic_auth(
                &self.inner.settings.iris_username,
                Some(&self.inner.settings.iris_password),
            )
            .json(body)
            .send()
            .await?;
        self.absorb(resp, "POST", url).await
    }

    /// PUT a JSON body with optional query pairs.
    ///
    /// # Errors
    /// Transport, status, envelope, or IRIS in-envelope errors.
    pub async fn put_json(
        &self,
        url: &str,
        query: &[(&str, &str)],
        body: &Value,
    ) -> Result<Envelope> {
        let resp = self
            .inner
            .http
            .put(url)
            .basic_auth(
                &self.inner.settings.iris_username,
                Some(&self.inner.settings.iris_password),
            )
            .query(query)
            .json(body)
            .send()
            .await?;
        self.absorb(resp, "PUT", url).await
    }

    /// DELETE a URL.
    ///
    /// # Errors
    /// Transport, status, envelope, or IRIS in-envelope errors.
    pub async fn delete(&self, url: &str) -> Result<Envelope> {
        let resp = self
            .inner
            .http
            .delete(url)
            .basic_auth(
                &self.inner.settings.iris_username,
                Some(&self.inner.settings.iris_password),
            )
            .send()
            .await?;
        self.absorb(resp, "DELETE", url).await
    }

    /// Status-code + envelope handling shared by all verbs.
    ///
    /// Rules (all verified live, iris-atelier.md §1/§4):
    /// - Non-2xx → [`Error::HttpStatus`] (404 callers translate to
    ///   `DocNotFound` in a match arm).
    /// - 2xx with `status.errors[]` non-empty → [`Error::Iris`].
    async fn absorb(&self, resp: reqwest::Response, method: &str, url: &str) -> Result<Envelope> {
        let status = resp.status();
        let path = url
            .strip_prefix(&self.inner.base)
            .unwrap_or(url)
            .to_string();

        // Try to parse the envelope even on error status (it usually is one).
        let body: Value = resp
            .json()
            .await
            .map_err(|_| Error::InvalidEnvelope(format!("{method} {path}: non-JSON body")))?;
        let env: Envelope = serde_json::from_value(body)
            .map_err(|e| Error::InvalidEnvelope(format!("{method} {path}: {e}")))?;

        if !status.is_success() {
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                method: method.to_string(),
                path,
            });
        }

        check_status_errors(&env)?;
        Ok(env)
    }
}

/// Raise [`Error::Iris`] for the first envelope error entry if any.
pub(crate) fn check_status_errors(env: &Envelope) -> Result<()> {
    if let Some(first) = env.status.errors.first() {
        let msg = first
            .error
            .clone()
            .or_else(|| env.status.summary.clone())
            .unwrap_or_else(|| "unknown error".to_string());
        let code = first.code.unwrap_or(0);
        return Err(Error::Iris { code, message: msg });
    }
    Ok(())
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
    fn api_url_encodes_percent_namespaces() {
        let c = IrisClient::new(Settings::default()).unwrap();
        assert_eq!(
            c.api_url("%SYS"),
            "http://localhost:52773/api/atelier/v8/%25SYS"
        );
        assert_eq!(
            c.api_url("USER"),
            "http://localhost:52773/api/atelier/v8/USER"
        );
    }

    fn envelope_with_error(code: u32, msg: &str) -> Envelope {
        serde_json::from_value(serde_json::json!({
            "status": {"errors": [{"error": msg, "code": code}], "summary": msg},
            "console": [],
            "result": {}
        }))
        .unwrap()
    }

    #[test]
    fn error_envelope_maps_to_iris_error() {
        let env = envelope_with_error(16002, "ERROR #16002: Invalid JSON Content");
        let err = check_status_errors(&env).unwrap_err();
        assert!(matches!(err, Error::Iris { code: 16002, .. }));
        assert!(err.to_string().contains("Invalid JSON Content"));
    }

    #[test]
    fn clean_envelope_passes() {
        let env: Envelope = serde_json::from_value(serde_json::json!({
            "status": {"errors": [], "summary": ""},
            "console": [],
            "result": {"content": []}
        }))
        .unwrap();
        assert!(check_status_errors(&env).is_ok());
    }
}
