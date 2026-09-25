//! Compile endpoint: `POST /{ns}/action/compile` — verified shape (live):
//! body = plain JSON **array** of doc names, flags in the query string,
//! full compilation log arrives in `console[]` (iris-atelier.md §5/§10).
//! NOTE: there is no `action/putandcompile` route (404, verified) — the
//! put-and-compile behaviour is composed in `tools`.

use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::iris::http::IrisClient;

/// Outcome of a compile call.
#[derive(Debug, Clone, Serialize)]
pub struct CompileOutcome {
    /// Server-side compilation log lines.
    pub console: Vec<String>,
    /// Per-document status entries reported in `result.content[]`.
    pub statuses: Vec<DocStatus>,
}

/// One document's compile status.
#[derive(Debug, Clone, Serialize)]
pub struct DocStatus {
    /// Document name.
    pub name: String,
    /// Status text (empty when clean).
    pub status: String,
}

/// Compile `docs` in `namespace` with `flags` (e.g. `cuk`).
///
/// # Errors
/// Transport/envelope errors, or [`Error::CompileFailed`] when any document
/// reports a status containing `ERROR`.
pub async fn compile(
    client: &IrisClient,
    namespace: &str,
    docs: &[String],
    flags: &str,
) -> Result<CompileOutcome> {
    let url = format!(
        "{}/action/compile?flags={}",
        client.api_url(namespace),
        urlencoding::encode(flags)
    );
    // Body is a bare array — an object 16002s (verified live).
    let env = client.post_json(&url, &serde_json::json!(docs)).await?;

    let statuses: Vec<DocStatus> = env
        .result
        .get("content")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(DocStatus {
                        name: e.get("name")?.as_str()?.to_string(),
                        status: e
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let failed: Vec<String> = statuses
        .iter()
        .filter(|s| s.status.contains("ERROR"))
        .map(|s| format!("{}: {}", s.name, s.status))
        .collect();
    if !failed.is_empty() {
        return Err(Error::CompileFailed {
            count: failed.len(),
            summary: failed.join(" | "),
        });
    }

    Ok(CompileOutcome {
        console: env.console,
        statuses,
    })
}
