//! MCP server adapter. Every `#[tool]` body is ONE call into `crate::tools`
//! plus the result mapping — zero logic here (rust-bestpractices.md §3.2).
//! NOTE: this module is part of the lib crate — use `crate::`, never `rism::`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::iris::IrisClient;
use crate::settings::Settings;
use crate::tools::sql::{ExecuteSqlArgs, execute_sql};

/// The Rism MCP server (stdio transport).
#[derive(Clone)]
pub struct RismMcp {
    client: IrisClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl RismMcp {
    /// Build from settings.
    ///
    /// # Errors
    /// Config errors from [`IrisClient::new`].
    pub fn new(settings: Settings) -> crate::Result<Self> {
        Ok(Self {
            client: IrisClient::new(settings)?,
            tool_router: Self::tool_router(),
        })
    }

    #[tool(
        description = "Execute SQL against the IRIS instance. Returns column names, rows, row count, and a truncated flag."
    )]
    async fn execute_sql(
        &self,
        Parameters(args): Parameters<ExecuteSqlArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match execute_sql(&self.client, &args).await {
            Ok(res) => {
                let text = serde_json::to_string_pretty(&res)
                    .unwrap_or_else(|_| format!("{} rows", res.row_count));
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            // Tool-level failure the model should see and can react to.
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
                e.to_string(),
            )])),
        }
    }
}

#[tool_handler(router = self.tool_router, name = "rism", version = "0.1.0",
    instructions = "IRIS development tools via the Atelier API: SQL, documents, compilation.")]
impl ServerHandler for RismMcp {}

/// Serve the MCP over stdio until the client disconnects.
///
/// # Errors
/// Transport/handshake errors.
pub async fn serve(settings: Settings) -> anyhow::Result<()> {
    // stdout is the JSON-RPC channel: logs go to stderr, always (mcp.md §8.1).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let server = RismMcp::new(settings)?;
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!("rism MCP server ready (stdio)");
    service.waiting().await?;
    Ok(())
}
