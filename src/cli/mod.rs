//! CLI adapter: clap definitions only — command bodies call `rism::tools`
//! directly from main dispatch (rust-bestpractices.md §3.3).

use clap::{Parser, Subcommand};

use crate::tools::sql::ExecuteSqlArgs;

pub mod render;

/// Rism — `InterSystems` IRIS development, from your terminal or as an MCP server.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Output format
    #[arg(long, global = true, default_value = "table", value_enum)]
    pub format: OutputFormat,

    /// IRIS base URL override (default: settings precedence)
    #[arg(long, global = true, env = "RISM_IRIS_BASE_URL")]
    pub url: Option<String>,

    /// Namespace override
    #[arg(long, global = true, env = "RISM_IRIS_NAMESPACE")]
    pub namespace: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Machine- vs human-readable rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Aligned text table (default)
    Table,
    /// JSON on stdout
    Json,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run an SQL statement
    Sql {
        /// SQL statement to execute
        query: String,
        /// Max rows to return
        #[arg(long)]
        max_rows: Option<u32>,
    },
    /// Serve as an MCP server over stdio
    Mcp,
}

impl Commands {
    /// Convert the CLI shape into the canonical shared args (namespace comes
    /// from the global flag; tools layer falls back to settings when None).
    #[must_use]
    pub fn to_execute_sql(&self, ns_override: &Option<String>) -> Option<ExecuteSqlArgs> {
        match self {
            Self::Sql { query, max_rows } => Some(ExecuteSqlArgs {
                query: query.clone(),
                namespace: ns_override.clone(),
                max_rows: *max_rows,
            }),
            Self::Mcp => None,
        }
    }
}
