//! Server-info tool: the unversioned root probe, also useful as a smoke test.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::serverinfo as api;

/// Arguments for [`get_server_info`] (empty today; a type keeps the MCP
/// schema stable when options are added).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetServerInfoArgs {}

/// Server info result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ServerInfoOut {
    /// Product version string.
    pub version: String,
    /// Atelier API version in use on the server.
    pub api: u8,
    /// Namespaces on the instance.
    pub namespaces: Vec<String>,
    /// Base URL this client is talking to.
    pub base_url: String,
}

/// Fetch server info — the single implementation behind `rism info` and
/// MCP `get_server_info`.
///
/// # Errors
/// Propagates [`crate::error::Error`] from the Atelier layer.
pub async fn get_server_info(
    client: &IrisClient,
    _args: &GetServerInfoArgs,
) -> Result<ServerInfoOut> {
    let info = api::server_info(client).await?;
    Ok(ServerInfoOut {
        version: info.version,
        api: info.api,
        namespaces: info.namespaces,
        base_url: client.settings().iris_base_url,
    })
}
