//! CLI adapter: clap definitions only — command bodies call `rism::tools`
//! directly from main dispatch (rust-bestpractices.md §3.3).

use clap::{Parser, Subcommand};

use crate::tools::documents::{
    DeleteDocumentArgs, GetDocumentArgs, ListDocumentsArgs, PutDocumentArgs,
};
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
    /// Document operations (list/get/put/compile/delete)
    #[command(subcommand)]
    Doc(DocCommands),
    /// Execute an `ObjectScript` command via the terminal WebSocket
    Exec {
        /// `ObjectScript` command
        command: String,
        /// Timeout seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Unit-testing operations (run/list/results)
    #[command(subcommand)]
    Test(TestCommands),
    /// Compile documents already on the server (no upload)
    Compile {
        /// Document names (e.g. My.Class.cls)
        names: Vec<String>,
        /// Compile flags
        #[arg(long, default_value = "cuk")]
        flags: String,
    },
    /// Live metrics + load score snapshot
    Monitor {
        /// Include all raw samples
        #[arg(long)]
        raw: bool,
    },
    /// Show IRIS server info (also a connectivity smoke test)
    Info,
    /// Serve as an MCP server over stdio
    Mcp,
}

/// Test subcommands.
#[derive(Subcommand, Debug)]
pub enum TestCommands {
    /// Run `%UnitTest` tests for a class (or one method)
    Run {
        /// Test class extending %UnitTest.TestCase
        class: String,
        /// Single Test* method
        #[arg(long)]
        method: Option<String>,
        /// Timeout seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// List discovered test classes and methods
    List {
        /// Class-name prefix filter
        #[arg(long)]
        filter: Option<String>,
    },
    /// Show stored test-run history
    Results {
        /// Filter by class
        #[arg(long)]
        class: Option<String>,
        /// Max runs
        #[arg(long, default_value = "10")]
        limit: u32,
    },
}

/// Document subcommands.
#[derive(Subcommand, Debug)]
pub enum DocCommands {
    /// List documents
    List {
        /// SQL LIKE filter on names (e.g. 'My.%')
        #[arg(long)]
        filter: Option<String>,
        /// Filetypes to include (e.g. CLS,RTN)
        #[arg(long, value_delimiter = ',')]
        filetypes: Vec<String>,
        /// Max documents
        #[arg(long)]
        count: Option<u32>,
    },
    /// Get a document's source
    Get {
        /// Document name (e.g. My.Class.cls)
        name: String,
    },
    /// Upload a document WITHOUT compiling
    Put {
        /// Document name
        name: String,
        /// Source file ('-' reads stdin)
        #[arg(long)]
        file: String,
        /// Fail instead of overwriting if the server copy changed since read
        #[arg(long)]
        no_ignore_conflict: bool,
    },
    /// Upload a document AND compile it
    Compile {
        /// Document name
        name: String,
        /// Source file ('-' reads stdin)
        #[arg(long)]
        file: String,
        /// Fail instead of overwriting if the server copy changed since read
        #[arg(long)]
        no_ignore_conflict: bool,
        /// Compile flags
        #[arg(long, default_value = "cuk")]
        flags: String,
    },
    /// Delete a document
    Delete {
        /// Document name
        name: String,
    },
}

impl Commands {
    /// Convert the CLI shape into the canonical shared args (namespace comes
    /// from the global flag; tools layer falls back to settings when None).
    /// One variant per command; dispatch in [`main`].
    #[must_use]
    pub fn to_execute_sql(&self, ns_override: &Option<String>) -> Option<ExecuteSqlArgs> {
        match self {
            Self::Sql { query, max_rows } => Some(ExecuteSqlArgs {
                query: query.clone(),
                namespace: ns_override.clone(),
                max_rows: *max_rows,
            }),
            Self::Mcp
            | Self::Doc(_)
            | Self::Compile { .. }
            | Self::Info
            | Self::Exec { .. }
            | Self::Test(_)
            | Self::Monitor { .. } => None,
        }
    }

    /// Shared args for the document tools, or None when the command isn't `doc`.
    #[must_use]
    pub fn to_doc(&self, ns_override: &Option<String>) -> Option<DocCall> {
        match self {
            Self::Doc(d) => Some(match d {
                DocCommands::List {
                    filter,
                    filetypes,
                    count,
                } => DocCall::List(ListDocumentsArgs {
                    filter: filter.clone(),
                    filetypes: if filetypes.is_empty() {
                        None
                    } else {
                        Some(filetypes.clone())
                    },
                    count: *count,
                    namespace: ns_override.clone(),
                }),
                DocCommands::Get { name } => DocCall::Get(GetDocumentArgs {
                    name: name.clone(),
                    namespace: ns_override.clone(),
                }),
                DocCommands::Put {
                    name,
                    file,
                    no_ignore_conflict,
                } => DocCall::Put {
                    args: PutDocumentArgs {
                        name: name.clone(),
                        content: Vec::new(),
                        ignore_conflict: !no_ignore_conflict,
                        namespace: ns_override.clone(),
                        flags: None,
                    },
                    file: file.clone(),
                    compile: false,
                },
                DocCommands::Compile {
                    name,
                    file,
                    no_ignore_conflict,
                    flags,
                } => DocCall::Put {
                    args: PutDocumentArgs {
                        name: name.clone(),
                        content: Vec::new(),
                        ignore_conflict: !no_ignore_conflict,
                        namespace: ns_override.clone(),
                        flags: Some(flags.clone()),
                    },
                    file: file.clone(),
                    compile: true,
                },
                DocCommands::Delete { name } => DocCall::Delete(DeleteDocumentArgs {
                    name: name.clone(),
                    namespace: ns_override.clone(),
                }),
            }),
            Self::Sql { .. }
            | Self::Mcp
            | Self::Compile { .. }
            | Self::Info
            | Self::Exec { .. }
            | Self::Test(_)
            | Self::Monitor { .. } => None,
        }
    }
}

/// A dispatched document call (content filled from file at exec time).
#[derive(Debug)]
pub enum DocCall {
    /// list
    List(ListDocumentsArgs),
    /// get
    Get(GetDocumentArgs),
    /// put (compile flag says which)
    Put {
        /// Shared put args.
        args: PutDocumentArgs,
        /// Source path or '-' for stdin.
        file: String,
        /// Compile after put?
        compile: bool,
    },
    /// delete
    Delete(DeleteDocumentArgs),
}
