//! MCP server adapter. Every `#[tool]` body is ONE call into `crate::tools`
//! plus the result mapping — zero logic here (rust-bestpractices.md §3.2).
//! NOTE: this module is part of the lib crate — use `crate::`, never `rism::`.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};

use crate::iris::IrisClient;
use crate::settings::Settings;
use crate::tools::command::{ExecuteCommandArgs, execute_command};
use crate::tools::compile::{CompileDocumentsArgs, compile_documents};
use crate::tools::debugger::{
    DebugAttachArgs, DebugBreakpointsArgs, DebugInspectArgs, DebugListProcessesArgs,
    DebugStackArgs, DebugStartArgs, DebugStepArgs, DebugStopArgs, DebugVariablesArgs, debug_attach,
    debug_breakpoints, debug_inspect, debug_list_processes, debug_stack, debug_start, debug_step,
    debug_stop, debug_variables,
};
use crate::tools::documents::{
    DeleteDocumentArgs, GetDocumentArgs, ListDocumentsArgs, PutDocumentArgs, delete_document,
    get_document, list_documents, put_and_compile, put_document,
};
use crate::tools::host::{
    ListFilesArgs, ReadFileArgs, RunShellArgs, list_files, read_file, run_shell,
};
use crate::tools::monitor::{MonitorArgs, monitor_system};
use crate::tools::serverinfo::{GetServerInfoArgs, get_server_info};
use crate::tools::sql::{ExecuteSqlArgs, execute_sql};
use crate::tools::testing::{
    GetTestResultsArgs, ListTestsArgs, RunTestsArgs, get_test_results, list_tests, run_tests,
};

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
        description = "Execute an ObjectScript command in the IRIS terminal (WebSocket). For method calls, globals, system utilities — anything ObjectScript."
    )]
    async fn execute_command(
        &self,
        Parameters(args): Parameters<ExecuteCommandArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(execute_command(&self.client, &args).await))
    }

    #[tool(
        description = "Run a shell command on the LOCAL host running Rism (bash; PowerShell on Windows). Returns stdout/stderr/exit_code; the command is killed after the timeout. NOT for IRIS — for ObjectScript use execute_command."
    )]
    async fn run_shell(
        &self,
        Parameters(args): Parameters<RunShellArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(run_shell(&self.client, &args).await))
    }

    #[tool(
        description = "Read a text file from the local workspace (RISM_WORKSPACE root; traversal blocked, binary rejected, 100k-char cap). For IRIS source use get_document."
    )]
    async fn read_file(
        &self,
        Parameters(args): Parameters<ReadFileArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(read_file(&self.client, &args).await))
    }

    #[tool(
        description = "List files in the local workspace (optional glob pattern, max_results cap). For IRIS source use list_documents."
    )]
    async fn list_files(
        &self,
        Parameters(args): Parameters<ListFilesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(list_files(&self.client, &args).await))
    }

    #[tool(
        description = "Fetch live IRIS metrics and return a scored load snapshot: overall + cpu/memory/disk/process sub-scores (0-100), health grade, key metrics, aggregates (DB sizes, top processes, CSP connections). Compare two snapshots: lower score wins."
    )]
    async fn monitor_system(
        &self,
        Parameters(args): Parameters<MonitorArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(monitor_system(&self.client, &args).await))
    }

    #[tool(
        description = "Run %UnitTest tests on the IRIS server (via the manager; no code is uploaded). Returns per-method results with assertion details for failures."
    )]
    async fn run_tests(
        &self,
        Parameters(args): Parameters<RunTestsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(run_tests(&self.client, &args).await))
    }

    #[tool(
        description = "Discover %UnitTest.TestCase classes and their Test* methods on the server (pure SQL over %Dictionary)."
    )]
    async fn list_tests(
        &self,
        Parameters(args): Parameters<ListTestsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(list_tests(&self.client, &args).await))
    }

    #[tool(
        description = "Read stored %UnitTest results history (runs newest first), optionally filtered by class."
    )]
    async fn get_test_results(
        &self,
        Parameters(args): Parameters<GetTestResultsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(get_test_results(&self.client, &args).await))
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

    #[tool(description = "List running IRIS processes (jobs) for debugger attach.")]
    async fn debug_list_processes(
        &self,
        Parameters(args): Parameters<DebugListProcessesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_list_processes(&self.client, &args).await))
    }

    #[tool(
        description = "Start a DBGP debug session on an ObjectScript target (##class(Pkg.Cls).Method(args) or Do^Routine). Returns session_id; one session at a time."
    )]
    async fn debug_start(
        &self,
        Parameters(args): Parameters<DebugStartArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_start(&self.client, &args).await))
    }

    #[tool(
        description = "Attach the debugger to a running IRIS process by pid (from debug_list_processes). Pauses it; call debug_stop to resume."
    )]
    async fn debug_attach(
        &self,
        Parameters(args): Parameters<DebugAttachArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_attach(&self.client, &args).await))
    }

    #[tool(
        description = "Debug step action: step_into|step_over|step_out|run|break|stop. Returns new state, location, variables at breaks."
    )]
    async fn debug_step(
        &self,
        Parameters(args): Parameters<DebugStepArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_step(&self.client, &args).await))
    }

    #[tool(
        description = "Variables at the current break (context: private|public|class). Always run after stack_get internally — required by the IRIS agent."
    )]
    async fn debug_variables(
        &self,
        Parameters(args): Parameters<DebugVariablesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_variables(&self.client, &args).await))
    }

    #[tool(description = "Evaluate an ObjectScript expression/variable in the break context.")]
    async fn debug_inspect(
        &self,
        Parameters(args): Parameters<DebugInspectArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_inspect(&self.client, &args).await))
    }

    #[tool(description = "Call stack frames of the current break (levels start at 1 on IRIS).")]
    async fn debug_stack(
        &self,
        Parameters(args): Parameters<DebugStackArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_stack(&self.client, &args).await))
    }

    #[tool(
        description = "Breakpoints: action list|set|remove|enable|disable. Entry breakpoints need offset 1 on current IRIS (offset 0 fails)."
    )]
    async fn debug_breakpoints(
        &self,
        Parameters(args): Parameters<DebugBreakpointsArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_breakpoints(&self.client, &args).await))
    }

    #[tool(description = "Stop a debug session and resume the target process.")]
    async fn debug_stop(
        &self,
        Parameters(args): Parameters<DebugStopArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(map_json(debug_stop(&self.client, &args).await))
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
impl ServerHandler for RismMcp {
    /// Prism-parity request/response logging (log.py): every tool call is
    /// logged to stderr at DEBUG with truncation. Debug tools are gated off
    /// when `debug_tools_enabled` is false (attach pauses live jobs — Prism
    /// hides the whole module in that case).
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let name = request.name.to_string();
        if name.starts_with("debug_") && !self.client.settings().debug_tools_enabled {
            return Ok(rmcp::model::CallToolResponse::Complete(
                CallToolResult::error(vec![ContentBlock::text(
                    "debugger tools are disabled (set RISM_DEBUG_TOOLS=1 to enable)",
                )]),
            ));
        }
        if tracing::enabled!(tracing::Level::DEBUG) {
            let params = request.arguments.as_ref().map_or_else(
                || serde_json::json!({}),
                |obj| serde_json::Value::Object(obj.clone().into_iter().collect()),
            );
            tracing::debug!(
                "\n{}\n{}",
                crate::logfmt::request_banner(&name),
                crate::logfmt::pretty(&crate::logfmt::truncate_params(&params))
            );
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let res = self.tool_router.call(tcc).await;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let text = match res.as_ref().ok() {
                Some(rmcp::model::CallToolResponse::Complete(r)) => r
                    .content
                    .first()
                    .and_then(|c| match c {
                        rmcp::model::ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let value = serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or_else(|_| serde_json::json!(&text));
            tracing::debug!(
                "\n{}\n{}",
                crate::logfmt::response_banner(&name),
                crate::logfmt::pretty(&crate::logfmt::truncate_result(&value))
            );
        }
        res
    }

    /// The 9 `debug_*` tools vanish from tools/list when disabled —
    /// discovery-level parity with Prism's `_SKIP_MODULES` gating.
    async fn list_tools(
        &self,
        request: Option<rmcp::model::PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= rmcp::model::ProtocolVersion::V_2026_07_28);
        let _ = request;
        let mut res = rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(rmcp::model::CacheScope::Public),
        };
        if !self.client.settings().debug_tools_enabled {
            res.tools.retain(|t| !t.name.as_ref().starts_with("debug_"));
        }
        Ok(res)
    }
}

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
