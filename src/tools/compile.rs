//! Standalone compile tool (compiles documents already on the server —
//! no upload involved).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::compile as api;

/// Arguments for [`compile_documents`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompileDocumentsArgs {
    /// Document names to compile (e.g. `["My.Class.cls"]`)
    pub names: Vec<String>,
    /// Compile flags (default cuk)
    pub flags: Option<String>,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// Compile result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompileResult {
    /// Compile console log.
    pub console: Vec<String>,
    /// Per-document status (empty status = clean).
    pub statuses: Vec<DocStatusOut>,
}

/// One document's compile status.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocStatusOut {
    /// Name.
    pub name: String,
    /// Status text.
    pub status: String,
}

/// Compile existing server documents.
///
/// # Errors
/// [`crate::error::Error::Iris`] when compilation fails; transport/envelope
/// otherwise.
pub async fn compile_documents(
    client: &IrisClient,
    args: &CompileDocumentsArgs,
) -> Result<CompileResult> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace.clone());
    let flags = args.flags.clone().unwrap_or_else(|| "cuk".to_string());
    let outcome = api::compile(client, &ns, &args.names, &flags).await?;
    Ok(CompileResult {
        console: outcome.console,
        statuses: outcome
            .statuses
            .into_iter()
            .map(|s| DocStatusOut {
                name: s.name,
                status: s.status,
            })
            .collect(),
    })
}
