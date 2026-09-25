//! MCP server adapter. Every `#[tool]` body is ONE call into `crate::tools`
//! plus the result mapping — zero logic here (rust-bestpractices.md §3.2).
//! NOTE: this module is part of the lib crate — use `crate::`, never `rism::`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::iris::IrisClient;
use crate::settings::Settings;
use crate::tools::compile::{CompileDocumentsArgs, compile_documents};
use crate::tools::documents::{
    DeleteDocumentArgs, GetDocumentArgs, ListDocumentsArgs, PutDocumentArgs, delete_document,
    get_document, list_documents, put_and_compile, put_document,
};
use crate::tools::serverinfo::{GetServerInfoArgs, get_server_info};
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
    pub async fn new(settings: Settings) -> crate::Result<Self> {
        let client = IrisClient::new(settings)?;
        client.negotiate_version().await;
        Ok(Self {
            client,
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

    #[tool(
        description = "List documents in a namespace. Optional LIKE filter (e.g. 'My.%'), filetypes (CLS, RTN, ...), and count."
    )]
    async fn list_documents(
        &self,
        Parameters(args): Parameters<ListDocumentsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(list_documents(&self.client, &args).await))
    }

    #[tool(description = "Get a document's source (as text lines) plus metadata.")]
    async fn get_document(
        &self,
        Parameters(args): Parameters<GetDocumentArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(get_document(&self.client, &args).await))
    }

    #[tool(
        description = "Upload/overwrite a document WITHOUT compiling. Content is an array of source lines."
    )]
    async fn put_document(
        &self,
        Parameters(args): Parameters<PutDocumentArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(put_document(&self.client, &args).await))
    }

    #[tool(
        description = "Upload/overwrite a document AND compile it. Returns the compile console log; fails with the IRIS compile errors when compilation fails."
    )]
    async fn put_and_compile(
        &self,
        Parameters(args): Parameters<PutDocumentArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(put_and_compile(&self.client, &args).await))
    }

    #[tool(description = "Delete a document from the server.")]
    async fn delete_document(
        &self,
        Parameters(args): Parameters<DeleteDocumentArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(delete_document(&self.client, &args).await))
    }

    #[tool(
        description = "Compile documents already on the server (no upload). Returns the compile console log; fails with IRIS errors on compile failure."
    )]
    async fn compile_documents(
        &self,
        Parameters(args): Parameters<CompileDocumentsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(compile_documents(&self.client, &args).await))
    }

    #[tool(
        description = "Get IRIS server info: product version, Atelier API version, namespace list. Also a connectivity smoke test."
    )]
    async fn get_server_info(
        &self,
        Parameters(args): Parameters<GetServerInfoArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(get_server_info(&self.client, &args).await))
    }
}

/// Shared result mapping: JSON-pretty on success, tool-level error text.
fn map_json<T: serde::Serialize>(res: crate::Result<T>) -> CallToolResult {
    match res {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "serialization failed: {e}"
            ))]),
        },
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
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

    let server = RismMcp::new(settings).await?;
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!("rism MCP server ready (stdio)");
    service.waiting().await?;
    Ok(())
}
