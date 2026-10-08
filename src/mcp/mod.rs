//! MCP server adapter. Every `#[tool]` body is ONE call into `crate::tools`
//! plus the result mapping — zero logic here (rust-bestpractices.md §3.2).
//! NOTE: this module is part of the lib crate — use `crate::`, never `rism::`.

pub mod tasks;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, CancelTaskParams, ContentBlock, GetTaskParams, GetTaskResult,
    ServerCapabilities, ServerConfig, UpdateTaskParams,
};
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
use crate::tools::jobs;
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
        description = "Execute an ObjectScript command in the IRIS terminal (WebSocket). For method calls, globals, system utilities — anything ObjectScript. For long-running work (loops, imports, batch methods) pass background=true: the call answers with a task handle (taskId) instead of blocking; poll it with tasks/get and stop it with tasks/cancel (real server-side interrupt).",
        annotations(
            title = "Run ObjectScript",
            destructive_hint = true,
            idempotent_hint = false
        )
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

/// Prism-parity REQUEST banner (log.py `logged_tool`): logged at the TOP of
/// `call_tool` for EVERY path, before capability/task-mode branches return.
fn log_request(name: &str, arguments: Option<&rmcp::model::JsonObject>) {
    if tracing::enabled!(tracing::Level::DEBUG) {
        let params = arguments.map_or_else(
            || serde_json::json!({}),
            |obj| serde_json::Value::Object(obj.clone().into_iter().collect()),
        );
        tracing::debug!(
            "\n{}\n{}",
            crate::logfmt::request_banner(name),
            crate::logfmt::pretty(&crate::logfmt::truncate_params(&params))
        );
    }
}

/// Prism-parity RESPONSE banner for a plain-text result (refusals returning
/// early — the router tail never runs for them).
fn log_response_text(name: &str, text: &str) {
    log_response_json(name, &crate::logfmt::response_value(text));
}

/// Prism-parity RESPONSE banner for a structured result (task handles).
fn log_response_json<T: serde::Serialize>(name: &str, value: &T) {
    if tracing::enabled!(tracing::Level::DEBUG) {
        let v = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
        tracing::debug!(
            "\n{}\n{}",
            crate::logfmt::response_banner(name),
            crate::logfmt::pretty(&crate::logfmt::truncate_result(&v))
        );
    }
}

/// -32021 for tasks/* calls whose session predates the extension
/// mechanism — same shape rmcp itself returns for missing caps.
fn missing_tasks_capability() -> rmcp::ErrorData {
    rmcp::ErrorData::missing_required_client_capability(
        rmcp::model::ClientCapabilities::builder()
            .enable_tasks()
            .build(),
    )
}

/// First protocol version under which the Tasks extension (SEP-2663) is
/// defined: 2026-06-30, the first release with the extension mechanism
/// (SEP-2133). Under 2025-11-25 the extension table is explicit — a
/// declared `io.modelcontextprotocol/tasks` key MUST be ignored (that
/// version has a DIFFERENT, not-wire-compatible tasks spec). rmcp 3.4.1
/// knows no `V_2026_06_30` constant, so compare the ISO-date strings
/// (every protocolVersion is `YYYY-MM-DD`, lexicographic == chronological).
pub(crate) const TASKS_MIN_PROTOCOL: &str = "2026-06-30";

/// SEP-2663 task gate: client declared the extension AND the negotiated
/// protocol version defines extensions. Use in `call_tool` (task mode) and
/// in the `tasks/*` handlers — rmcp's own dispatch validates only the
/// capability, so the version clause is ours to enforce.
pub(crate) fn tasks_supported(context: &rmcp::service::RequestContext<rmcp::RoleServer>) -> bool {
    context
        .client_capabilities()
        .is_some_and(|caps| caps.supports_tasks())
        && context
            .protocol_version()
            .is_some_and(|v| v.as_str() >= TASKS_MIN_PROTOCOL)
}

#[tool_handler(router = self.tool_router)]
#[allow(
    // three instant-resolving trait impls (registry reads + pure mapping);
    // yield_now boilerplate is noise and method-level allow is ignored by
    // CI's 1.99 (module/impl-level is what works). The lint is `unused_async`
    // on stable; 1.99 nightlies renamed it to `unused_async_trait_impl` —
    // silence both names, `unknown_lints` keeps the inactive one quiet.
    unknown_lints,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
impl ServerHandler for RismMcp {
    /// Prism-parity request/response logging (log.py): every tool call is
    /// logged to stderr at DEBUG with truncation — REQUEST at entry (so
    /// refusals and early returns are never silent) and RESPONSE wherever a
    /// result exists to log. Debug tools are gated off when
    /// `debug_tools_enabled` is false (attach pauses live jobs — Prism hides
    /// the whole module in that case).
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let name = request.name.to_string();
        // Hoisted: log the REQUEST before ANY branch (capability refusals,
        // task-mode early returns included) so "every call is logged" holds
        // for every path through this function.
        log_request(&name, request.arguments.as_ref());
        if name.starts_with("debug_") && !self.client.settings().debug_tools_enabled {
            let text = "debugger tools are disabled (set RISM_DEBUG_TOOLS=1 to enable)";
            log_response_text(&name, text);
            return Ok(rmcp::model::CallToolResponse::Complete(
                CallToolResult::error(vec![ContentBlock::text(text)]),
            ));
        }
        // SEP-2663 task mode: a Tasks-capable client asking for background
        // execution gets a task handle instead of a blocking call. The gate
        // lives here (not in the tool body) because tool fns cannot see the
        // request context; rmcp's dispatch additionally rejects Task
        // responses to non-declaring clients with -32021, so this is exact.
        if name == "execute_command"
            && request.arguments.as_ref().is_some_and(|a| {
                a.get("background")
                    .is_some_and(|b| b == &serde_json::Value::Bool(true))
            })
        {
            let declared = tasks_supported(&context);
            if !declared {
                // Rail A first, before ANY parsing: the honest guidance
                // must reach a non-declaring client even if the payload
                // would also fail schema validation. "Declaring" includes
                // the protocol version: under < 2026-06-30 the extension
                // key MUST be treated as if absent (SEP-2663 compat table).
                let text = "background=true requires the Tasks extension (the client must \
                            declare io.modelcontextprotocol/tasks in its capabilities — \
                            initialize or per-request _meta — under protocol version \
                            2026-06-30 or later); this client did not, so run the command \
                            synchronously with a safe timeout_secs instead";
                log_response_text(&name, text);
                return Ok(rmcp::model::CallToolResponse::Complete(
                    CallToolResult::error(vec![ContentBlock::text(text)]),
                ));
            }
            let args: ExecuteCommandArgs = serde_json::from_value(serde_json::Value::Object(
                request
                    .arguments
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            ))
            .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
            {
                // Wall-clock cap floor for background jobs: a task is
                // detached precisely because it outlives sync timeouts, so
                // the short sync default would be wrong here. Documented
                // in docs/mcp-tools.md (Background execution section).
                let timeout = std::time::Duration::from_secs(
                    args.timeout_secs
                        .unwrap_or(self.client.settings().timeout_secs.max(3600)),
                );
                let info = jobs::start(
                    &self.client,
                    args.namespace.clone(),
                    args.command.clone(),
                    timeout,
                )
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("failed to create task: {e}"), None)
                })?;
                let seed = tasks::create_task(&info);
                log_response_json(&name, &seed);
                return Ok(rmcp::model::CallToolResponse::Task(seed));
            }
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
            log_response_text(&name, &text);
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
        // Trait-mandated async with no real I/O: one yield keeps newer
        // toolchains' `unused_async`-on-trait-impl satisfied (method-level
        // allow is ignored by that check — learned from CI run 36171856244).
        tokio::task::yield_now().await;
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

    /// Advertise tools + the SEP-2663 Tasks extension (this replaces the
    /// macro-generated version — keep `enable_tasks()` in sync with the
    /// `background=true` gate in `call_tool`).
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "rism",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "IRIS development tools via the Atelier API: SQL, documents, compilation. \
             Long-running terminal work: execute_command with background=true returns a \
             task handle; poll tasks/get, stop tasks/cancel.",
        )
    }

    /// SEP-2663 `tasks/get`: the registry snapshot projected to a task.
    /// Dispatch-level capability validation already rejected non-declaring
    /// clients; unknown/GC'd ids are a clean JSON-RPC error. The version
    /// clause of the declaration gate is ours — rmcp validates the
    /// capability only, so under < 2026-06-30 we must answer here (SEP-2663
    /// compat table: the extension key is inert on older versions).
    async fn get_task(
        &self,
        request: GetTaskParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<GetTaskResult, rmcp::ErrorData> {
        if !tasks_supported(&context) {
            return Err(missing_tasks_capability());
        }
        let info =
            jobs::status(&request.task_id).map_err(|e| tasks::task_error(&e, &request.task_id))?;
        Ok(GetTaskResult::new(tasks::detailed_task(&info)))
    }

    /// SEP-2663 `tasks/cancel`: cooperative — trip the flag whose next
    /// frame poll sends the protocol interrupt. The observable status
    /// flips to `cancelled` on the next `tasks/get` (spec allows lag).
    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<(), rmcp::ErrorData> {
        if !tasks_supported(&context) {
            return Err(missing_tasks_capability());
        }
        jobs::cancel(&request.task_id).map_err(|e| tasks::task_error(&e, &request.task_id))?;
        Ok(())
    }

    /// SEP-2663 `tasks/update`: rism tasks never request input (a terminal
    /// `read` during a background job is auto-answered), so there are no
    /// outstanding input requests to deliver.
    async fn update_task(
        &self,
        request: UpdateTaskParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<(), rmcp::ErrorData> {
        let _ = request;
        if !tasks_supported(&context) {
            return Err(missing_tasks_capability());
        }
        Err(rmcp::ErrorData::invalid_request(
            "rism tasks never request input; tasks/update is not used",
            None,
        ))
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
