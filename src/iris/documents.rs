//! Document endpoints: `/{ns}/doc[/{name}]`, `/{ns}/docnames`
//! (all shapes verified live — iris-atelier.md §4).

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::iris::http::IrisClient;

/// Metadata for one source document.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DocMeta {
    /// Document name (`My.Class.cls`).
    pub name: String,
    /// Category code (`CLS`, `RTN`, ...).
    #[serde(default)]
    pub cat: String,
    /// Last-modified timestamp as reported by the server.
    #[serde(default)]
    pub ts: String,
    /// Database the master lives in.
    #[serde(default)]
    pub db: String,
}

/// A document with its content lines.
#[derive(Debug, Clone)]
pub struct Document {
    /// Name.
    pub name: String,
    /// Category code.
    pub cat: String,
    /// Server timestamp.
    pub ts: String,
    /// Source as line array (Studio convention; may have "" fillers).
    pub content: Vec<String>,
}

/// List document metadata, optionally filtered.
///
/// # Errors
/// Propagates [`Error`] from transport/envelope layers.
pub async fn list_docs(
    client: &IrisClient,
    namespace: &str,
    filter: Option<&str>,
    filetypes: Option<&[&str]>,
    count: Option<u32>,
) -> Result<Vec<DocMeta>> {
    let mut url = format!("{}/docnames", client.api_url(namespace));
    let mut qs: Vec<(String, String)> = Vec::new();
    if let Some(f) = filter {
        qs.push(("filter".to_string(), f.to_string()));
    }
    if let Some(ft) = filetypes {
        qs.push(("filetypes".to_string(), ft.join(",")));
    }
    if let Some(c) = count {
        qs.push(("count".to_string(), c.to_string()));
    }
    if !qs.is_empty() {
        let joined = qs
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&joined);
    }

    let env = client.get(&url).await?;
    let content = env
        .result
        .get("content")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    serde_json::from_value(content)
        .map_err(|e| Error::InvalidEnvelope(format!("docnames content: {e}")))
}

/// Fetch one document. 404 → [`Error::DocNotFound`] (the only legitimate
/// 404 in the Atelier API — verified: missing docs also carry 16005 in-band).
///
/// # Errors
/// [`Error::DocNotFound`] on 404; transport/envelope errors otherwise.
pub async fn get_doc(client: &IrisClient, namespace: &str, name: &str) -> Result<Document> {
    let url = doc_url(client, namespace, name);
    match client.get(&url).await {
        Ok(env) => Ok(Document {
            name: env
                .result
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string(),
            cat: env
                .result
                .get("cat")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ts: env
                .result
                .get("ts")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            content: env
                .result
                .get("content")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        }),
        Err(Error::HttpStatus { status: 404, .. }) => Err(Error::DocNotFound {
            name: name.to_string(),
            ns: namespace.to_string(),
        }),
        Err(other) => Err(other),
    }
}

/// Create/update a document. Body MUST be `{"enc":false,"content":[lines]}`
/// (raw string content 16002s — verified live).
///
/// # Errors
/// Propagates [`Error`]; returns the new server timestamp on success.
pub async fn put_doc(
    client: &IrisClient,
    namespace: &str,
    name: &str,
    content: &[String],
    ignore_conflict: bool,
) -> Result<String> {
    let url = doc_url(client, namespace, name);
    let body = json!({ "enc": false, "content": content });
    let q = if ignore_conflict {
        vec![("ignoreConflict", "1")]
    } else {
        vec![]
    };
    let env = client.put_json(&url, &q, &body).await?;
    Ok(env
        .result
        .get("ts")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// Delete a document. 404 → [`Error::DocNotFound`].
///
/// # Errors
/// [`Error::DocNotFound`] on 404; transport/envelope errors otherwise.
pub async fn delete_doc(client: &IrisClient, namespace: &str, name: &str) -> Result<()> {
    let url = doc_url(client, namespace, name);
    match client.delete(&url).await {
        Ok(_) => Ok(()),
        Err(Error::HttpStatus { status: 404, .. }) => Err(Error::DocNotFound {
            name: name.to_string(),
            ns: namespace.to_string(),
        }),
        Err(other) => Err(other),
    }
}

fn doc_url(client: &IrisClient, namespace: &str, name: &str) -> String {
    format!(
        "{}/doc/{}",
        client.api_url(namespace),
        name.replace('%', "%25")
    )
}
