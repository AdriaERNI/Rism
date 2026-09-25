//! Document tools: list / get / put / delete + put-and-compile.
//! Canonical args types feed MCP JSON schema and `clap --help` alike.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::documents as api;

/// Arguments for [`list_documents`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDocumentsArgs {
    /// SQL LIKE filter on document names (e.g. `RismTest.%`)
    pub filter: Option<String>,
    /// File types to include (e.g. `["CLS", "RTN"]`)
    pub filetypes: Option<Vec<String>>,
    /// Max documents to return
    pub count: Option<u32>,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// One document's metadata.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocMetaOut {
    /// Document name.
    pub name: String,
    /// Category code (CLS, RTN, ...).
    pub cat: String,
    /// Last-modified timestamp.
    pub ts: String,
    /// Database.
    pub db: String,
}

/// Documents list result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListDocumentsResult {
    /// Matching documents.
    pub documents: Vec<DocMetaOut>,
    /// How many were returned.
    pub count: usize,
}

/// List documents in a namespace.
///
/// # Errors
/// Propagates [`crate::error::Error`] from the Atelier layer.
pub async fn list_documents(
    client: &IrisClient,
    args: &ListDocumentsArgs,
) -> Result<ListDocumentsResult> {
    let ns = ns(client, args.namespace.as_ref());
    let fts: Vec<&str> = args
        .filetypes
        .as_deref()
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let metas = api::list_docs(
        client,
        &ns,
        args.filter.as_deref(),
        if fts.is_empty() { None } else { Some(&fts) },
        args.count,
    )
    .await?;
    let documents = metas
        .into_iter()
        .map(|m| DocMetaOut {
            name: m.name,
            cat: m.cat,
            ts: m.ts,
            db: m.db,
        })
        .collect::<Vec<_>>();
    let count = documents.len();
    Ok(ListDocumentsResult { documents, count })
}

/// Arguments for [`get_document`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDocumentArgs {
    /// Document name (e.g. `My.Class.cls`)
    pub name: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// Document content result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocumentOut {
    /// Name.
    pub name: String,
    /// Category code.
    pub cat: String,
    /// Server timestamp.
    pub ts: String,
    /// Source lines.
    pub content: Vec<String>,
}

/// Fetch a document with its content.
///
/// # Errors
/// [`crate::error::Error::DocNotFound`] on 404; transport/envelope otherwise.
pub async fn get_document(client: &IrisClient, args: &GetDocumentArgs) -> Result<DocumentOut> {
    let ns = ns(client, args.namespace.as_ref());
    let doc = api::get_doc(client, &ns, &args.name).await?;
    Ok(DocumentOut {
        name: doc.name,
        cat: doc.cat,
        ts: doc.ts,
        content: doc.content,
    })
}

/// Arguments for write tools (put and put-and-compile share them).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PutDocumentArgs {
    /// Document name (e.g. `My.Class.cls`)
    pub name: String,
    /// Source lines
    pub content: Vec<String>,
    /// Overwrite even if the server copy changed since read
    #[serde(default)]
    pub ignore_conflict: bool,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Compile flags for the compile step (default `cuk`)
    pub flags: Option<String>,
}

/// Result of a put (and optional compile).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PutDocumentResult {
    /// Document name.
    pub name: String,
    /// New server timestamp.
    pub ts: String,
    /// Compile console lines (empty when no compile ran).
    pub console: Vec<String>,
}

/// Upload/overwrite a document (no compile).
///
/// # Errors
/// Transport/envelope errors; conflict errors from the server.
pub async fn put_document(
    client: &IrisClient,
    args: &PutDocumentArgs,
) -> Result<PutDocumentResult> {
    let ns = ns(client, args.namespace.as_ref());
    let ts = api::put_doc(client, &ns, &args.name, &args.content, args.ignore_conflict).await?;
    Ok(PutDocumentResult {
        name: args.name.clone(),
        ts,
        console: Vec::new(),
    })
}

/// Upload/overwrite a document, then compile it.
///
/// # Errors
/// Put errors, or [`crate::error::Error::Iris`] when compilation fails (the
/// server reports compile errors in the envelope `status.errors[]`).
pub async fn put_and_compile(
    client: &IrisClient,
    args: &PutDocumentArgs,
) -> Result<PutDocumentResult> {
    let ns = ns(client, args.namespace.as_ref());
    let ts = api::put_doc(client, &ns, &args.name, &args.content, args.ignore_conflict).await?;
    let flags = args.flags.clone().unwrap_or_else(|| "cuk".to_string());
    let outcome =
        crate::iris::compile::compile(client, &ns, std::slice::from_ref(&args.name), &flags)
            .await?;
    Ok(PutDocumentResult {
        name: args.name.clone(),
        ts,
        console: outcome.console,
    })
}

/// Arguments for [`delete_document`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteDocumentArgs {
    /// Document name
    pub name: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// Delete a document.
///
/// # Errors
/// [`crate::error::Error::DocNotFound`] on 404.
pub async fn delete_document(
    client: &IrisClient,
    args: &DeleteDocumentArgs,
) -> Result<serde_json::Value> {
    let ns = ns(client, args.namespace.as_ref());
    api::delete_doc(client, &ns, &args.name).await?;
    Ok(serde_json::json!({"deleted": args.name}))
}

fn ns(client: &IrisClient, override_ns: Option<&String>) -> String {
    override_ns
        .cloned()
        .unwrap_or_else(|| client.settings().iris_namespace.clone())
}
