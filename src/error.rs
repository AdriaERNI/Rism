//! Domain error types for the Rism library.

/// Result alias used across the library.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by Rism's library layers.
///
/// Atelier envelope error codes (16002, 16005, ...) are mapped to variants
/// here, once, in [`crate::iris::http`]; never match on raw codes downstream.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// HTTP transport failure (DNS, refused, timeout...).
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    /// A configured URL failed to parse.
    #[error("invalid base URL '{url}': {reason}")]
    InvalidUrl {
        /// The URL that failed to parse.
        url: String,
        /// Parser message.
        reason: String,
    },

    /// Non-2xx HTTP status that is not a documented in-envelope error.
    #[error("IRIS returned HTTP {status} for {method} {path}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Method used.
        method: String,
        /// Request path.
        path: String,
    },

    /// The response body was not a valid Atelier envelope.
    #[error("invalid Atelier response: {0}")]
    InvalidEnvelope(String),

    /// A server-side error carried in the Atelier envelope
    /// (`status.errors[]` or `result.status`), e.g. `#16002 Invalid JSON`.
    #[error("IRIS error #{code}: {message}")]
    Iris {
        /// IRIS error number (0 when unparsable).
        code: u32,
        /// Error text as reported by the server.
        message: String,
    },

    /// A document does not exist in the namespace (HTTP 404 + envelope).
    #[error("document '{name}' not found in namespace '{ns}'")]
    DocNotFound {
        /// Document name, e.g. `My.Class.cls`.
        name: String,
        /// Namespace it was looked up in.
        ns: String,
    },

    /// Compilation reported failures (result has per-doc status strings).
    #[error("{count} document(s) failed to compile: {summary}")]
    CompileFailed {
        /// How many documents errored.
        count: usize,
        /// Joined first-line summaries.
        summary: String,
    },

    /// SQL statement failed server-side.
    #[error("SQL error: {0}")]
    Sql(String),

    /// Configuration/settings problem.
    #[error("configuration error: {0}")]
    Config(String),

    /// Terminal WebSocket session failure (timeout, closed, server error frame).
    #[error("terminal error: {0}")]
    Terminal(String),
}

impl Error {
    /// True when this error means "the MCP/CLI call was handled, but IRIS
    /// reported a failure" — surfaces as `CallToolResult` error, not a
    /// protocol-level `ErrorData`.
    #[must_use]
    pub fn is_tool_error(&self) -> bool {
        !matches!(self, Self::Config(_) | Self::InvalidUrl { .. })
    }
}
