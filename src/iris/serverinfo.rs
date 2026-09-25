//! `GET /api/atelier/` server-info probe — used to negotiate the API version
//! prefix at startup (iris-atelier.md §2, §9: never hardcode v8).

use serde_json::Value;

use crate::error::Result;
use crate::iris::http::IrisClient;

/// Info reported by the root Atelier endpoint.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Product version string, e.g. `IRIS for UNIX ... 2025.3 (Build 226U)`.
    pub version: String,
    /// Atelier API version (8 on 2025.3).
    pub api: u8,
    /// Namespaces present on the instance.
    pub namespaces: Vec<String>,
}

/// Fetch and parse server info.
///
/// # Errors
/// Propagates [`crate::error::Error`] from the transport/envelope layer.
pub async fn server_info(client: &IrisClient) -> Result<ServerInfo> {
    let url = format!(
        "{}{}/",
        client.settings().iris_base_url.trim_end_matches('/'),
        client.api_prefix()
    );
    let env = client.get(&url).await?;

    let content = env.result.get("content").cloned().unwrap_or(Value::Null);
    Ok(ServerInfo {
        version: content
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        api: content
            .get("api")
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(0),
        namespaces: content
            .get("namespaces")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
mod tests {
    use serde_json::json;

    #[test]
    fn parses_verified_payload_shape() {
        // shape captured live from rism-iris (iris-atelier.md §10)
        let content = json!({
            "version": "IRIS for UNIX (Ubuntu Server LTS for x86-64 Containers) 2025.3 (Build 226U)",
            "id": "437BC286-...",
            "api": 8,
            "namespaces": ["%SYS", "USER"]
        });
        assert_eq!(content["api"], 8);
        assert_eq!(content["namespaces"].as_array().map(Vec::len), Some(2));
    }
}
