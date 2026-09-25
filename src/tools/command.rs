//! [`execute_command`] tool: `ObjectScript` via the terminal WebSocket.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::terminal as api;

/// Arguments for [`execute_command`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteCommandArgs {
    /// `ObjectScript` command to execute (e.g. 'write $ZVERSION,!')
    pub command: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Timeout in seconds (default: `timeout_secs` from settings)
    pub timeout_secs: Option<u64>,
}

/// Command result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommandResult {
    /// The namespace the command ran in.
    pub namespace: String,
    /// The command as sent.
    pub command: String,
    /// Cleaned terminal output.
    pub output: String,
    /// Prompt seen after completion.
    pub prompt: String,
    /// True when output exceeded the bound.
    pub output_truncated: bool,
    /// Chars omitted when truncated.
    pub output_omitted_chars: usize,
}

/// Execute an `ObjectScript` command — the single implementation behind
/// `rism exec` and MCP `execute_command`.
///
/// # Errors
/// [`crate::error::Error::Terminal`] on protocol/timeout failures.
pub async fn execute_command(
    client: &IrisClient,
    args: &ExecuteCommandArgs,
) -> Result<CommandResult> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace.clone());
    let timeout = Duration::from_secs(args.timeout_secs.unwrap_or(client.settings().timeout_secs));
    let out = api::execute(client, &ns, &args.command, timeout).await?;
    Ok(CommandResult {
        namespace: ns,
        command: args.command.clone(),
        output: out.output,
        prompt: out.prompt,
        output_truncated: out.truncated,
        output_omitted_chars: out.omitted_chars,
    })
}
