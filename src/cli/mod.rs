//! CLI adapter: clap definitions only — command bodies call `rism::tools`
//! directly from main dispatch (rust-bestpractices.md §3.3).

use clap::{Parser, Subcommand};

use crate::settings::McpTransport;
use crate::tools::documents::{
    DeleteDocumentArgs, GetDocumentArgs, ListDocumentsArgs, PutDocumentArgs,
};
use crate::tools::graph::{
    ClassDependenciesArgs, ClassMembersArgs, DependencyDirection, FindSubclassesArgs, MembersKind,
};
use crate::tools::search::SearchCodeArgs;
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
    /// Text/regex search over server source documents
    Search {
        /// Pattern (regex unless --no-regex)
        pattern: String,
        /// Document masks (repeatable or comma list; default *.cls,*.mac,*.int)
        #[arg(long, value_delimiter = ',')]
        documents: Vec<String>,
        /// Disable regex (literal text search)
        #[arg(long)]
        no_regex: bool,
        /// Include system documents
        #[arg(long)]
        system: bool,
        /// Include generated documents
        #[arg(long)]
        generated: bool,
        /// Case-sensitive match
        #[arg(long)]
        case_sensitive: bool,
        /// Whole-word match
        #[arg(long)]
        whole_word: bool,
        /// Max matches (default: settings `search_max_results` = 200)
        #[arg(long)]
        max: Option<u32>,
    },
    /// Class reference graph (subclasses / members / deps)
    #[command(subcommand)]
    Graph(GraphCommands),
    /// Document operations (list/get/put/compile/delete)
    #[command(subcommand)]
    Doc(DocCommands),
    /// Execute an `ObjectScript` command via the terminal WebSocket
    /// (no argument: interactive REPL with history)
    Exec {
        /// `ObjectScript` command (omit for interactive mode)
        command: Option<String>,
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
    /// Run a shell command on the local host
    Shell {
        /// Command (bash / `PowerShell` on Windows)
        command: String,
        /// Timeout seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Read a workspace file
    Cat {
        /// Path relative to `RISM_WORKSPACE`
        path: String,
    },
    /// List workspace files
    Ls {
        /// Directory (default workspace root)
        path: Option<String>,
        /// Glob pattern
        #[arg(long)]
        pattern: Option<String>,
    },
    /// Interactive debugging (DBGP over the Atelier API — no uploads)
    #[command(subcommand)]
    Debug(DebugCommands),
    /// Live metrics + load score snapshot
    Monitor {
        /// Include all raw samples
        #[arg(long)]
        raw: bool,
    },
    /// Show IRIS server info (also a connectivity smoke test)
    Info,
    /// Serve as an MCP server. Default door: stdio (byte-identical to every
    /// released version). `--transport http` opens a streamable-HTTP door at
    /// /mcp - NO authentication on that door: every local process can drive
    /// all 29 tools. Binds 127.0.0.1 unless --host/--allow-all-interfaces.
    Mcp {
        /// Transport door: `stdio` (default) or `http`; aliases
        /// `streamable-http` / `streamable_http` (env: `RISM_MCP_TRANSPORT`)
        #[arg(long, env = "RISM_MCP_TRANSPORT", value_parser = parse_mcp_transport)]
        transport: Option<McpTransport>,
        /// TCP port for the `http` door; 0 = ephemeral, actual port printed
        /// on the stderr ready line (env: `RISM_MCP_PORT`)
        #[arg(long, env = "RISM_MCP_PORT")]
        port: Option<u16>,
        /// Bind host for the http door (default 127.0.0.1; loopback only
        /// without --allow-all-interfaces)
        #[arg(long)]
        host: Option<String>,
        /// Bind a non-loopback host with no auth on the wire - loud warning,
        /// deliberate opt-out
        #[arg(long)]
        allow_all_interfaces: bool,
    },
}

/// clap `--transport` parser: one alias map for CLI and env, so an unknown
/// value is clap's standard invalid-value error (exit 2), never a panic.
fn parse_mcp_transport(s: &str) -> std::result::Result<McpTransport, String> {
    McpTransport::parse_alias(s).ok_or_else(|| {
        format!("'{s}' is not a transport (stdio, http, streamable-http, streamable_http)")
    })
}

/// Resolved `rism mcp` transport config (precedence: CLI flag >
/// `RISM_MCP_*` env > config.toml > defaults; merged by the main dispatch).
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Which door to serve.
    pub transport: Option<McpTransport>,
    /// http door port (None = settings/default 3000; Some(0) = ephemeral).
    pub port: Option<u16>,
    /// http door bind host (None = settings/default 127.0.0.1).
    pub host: Option<String>,
    /// Opt-out of the loopback-only bind rule (loud warning on stderr).
    pub allow_all_interfaces: bool,
}

impl McpConfig {
    /// True when `--port`/`--host` were given but the door is stdio (warn,
    /// then serve stdio exactly as shipped).
    #[must_use]
    pub fn has_unused_net_flags_stdio(&self) -> bool {
        !matches!(self.transport, Some(McpTransport::Http))
            && (self.port.is_some() || self.host.is_some())
    }
}

/// Graph subcommands (issue #17 Tier 2).
#[derive(Subcommand, Debug)]
pub enum GraphCommands {
    /// List subclasses of a class (bounded BFS)
    Subclasses {
        /// Class name
        class: String,
        /// Max BFS levels (default: settings `graph_depth`)
        #[arg(long)]
        depth: Option<u32>,
    },
    /// List a class's members
    Members {
        /// Class name
        class: String,
        /// Which members to list
        #[arg(long, value_parser = parse_kind_arg, default_value = "all")]
        kind: KindArg,
        /// Include inherited members
        #[arg(long)]
        include_inherited: bool,
    },
    /// Class dependency edges
    Deps {
        /// Class name
        class: String,
        /// Which direction(s)
        #[arg(long, value_parser = parse_direction_arg, default_value = "both")]
        direction: DirectionArg,
        /// Max BFS levels for `used_by` (default 1)
        #[arg(long)]
        depth: Option<u32>,
    },
}

/// `--kind` parser: accepts the frozen `snake_case` strings (§4) plus
/// clap's kebab derivations, so `depends_on`/`depends-on` both work.
fn parse_kind_arg(s: &str) -> std::result::Result<KindArg, String> {
    match s.replace('-', "_").as_str() {
        "all" => Ok(KindArg::All),
        "methods" => Ok(KindArg::Methods),
        "properties" => Ok(KindArg::Properties),
        "parameters" => Ok(KindArg::Parameters),
        "xdata" => Ok(KindArg::Xdata),
        other => Err(format!(
            "'{other}' is not a kind (all, methods, properties, parameters, xdata)"
        )),
    }
}

/// `--direction` parser (same dual-spelling rule as `--kind`).
fn parse_direction_arg(s: &str) -> std::result::Result<DirectionArg, String> {
    match s.replace('-', "_").as_str() {
        "depends_on" => Ok(DirectionArg::DependsOn),
        "used_by" => Ok(DirectionArg::UsedBy),
        "both" => Ok(DirectionArg::Both),
        other => Err(format!(
            "'{other}' is not a direction (depends_on, used_by, both)"
        )),
    }
}

/// clap mirror of [`MembersKind`] (cli stays the only clap-aware layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum KindArg {
    /// All member tables
    All,
    /// Methods only
    Methods,
    /// Properties only
    Properties,
    /// Parameters only
    Parameters,
    /// `XData` block names only
    Xdata,
}

/// clap mirror of [`DependencyDirection`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum DirectionArg {
    /// What the class uses
    DependsOn,
    /// Who uses the class
    UsedBy,
    /// Both directions
    Both,
}

/// Debug subcommands. CLI sessions are self-contained: each command opens,
/// drives, and closes the debug session (target process resumes on exit).
#[derive(Subcommand, Debug, Clone)]
pub enum DebugCommands {
    /// List debuggable IRIS processes (jobs)
    Ps {
        /// Include system processes
        #[arg(long)]
        system: bool,
        /// Filter by namespace
        #[arg(long)]
        ns: Option<String>,
    },
    /// Run a target under the debugger to completion (print stops)
    Run {
        /// `ObjectScript` target, e.g. '##class(Pkg.Cls).Method(1,2)'
        target: String,
        /// Break at the entry line, then continue
        #[arg(long)]
        stop_on_entry: bool,
        /// Breakpoint class (with --method)
        #[arg(long)]
        class: Option<String>,
        /// Breakpoint method
        #[arg(long)]
        method: Option<String>,
        /// Breakpoint line offset in the method
        #[arg(long, default_value_t = 1)]
        offset: u32,
        /// Max stops before forcing completion
        #[arg(long, default_value_t = 50)]
        max_stops: u32,
    },
    /// Attach to a running process, capture its location, resume it
    Attach {
        /// Process id (from `rism debug ps`)
        pid: u32,
    },
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
        /// Also save to this local path (explicit local file, like --file)
        #[arg(long)]
        save: Option<String>,
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
            Self::Mcp { .. }
            | Self::Doc(_)
            | Self::Search { .. }
            | Self::Graph(_)
            | Self::Compile { .. }
            | Self::Info
            | Self::Exec { .. }
            | Self::Test(_)
            | Self::Debug(_)
            | Self::Monitor { .. }
            | Self::Shell { .. }
            | Self::Cat { .. }
            | Self::Ls { .. } => None,
        }
    }

    /// Shared args for [`crate::tools::search::search_code`], or None when
    /// the command isn't `search`. Flag presence maps 1:1 to §1 defaults:
    /// unset flags stay `None` so the tools layer resolves via settings.
    #[must_use]
    pub fn to_search(&self, ns_override: &Option<String>) -> Option<SearchCodeArgs> {
        match self {
            Self::Search {
                pattern,
                documents,
                no_regex,
                system,
                generated,
                case_sensitive,
                whole_word,
                max,
            } => Some(SearchCodeArgs {
                query: pattern.clone(),
                namespace: ns_override.clone(),
                documents: if documents.is_empty() {
                    None
                } else {
                    Some(documents.clone())
                },
                regex: if *no_regex { Some(false) } else { None },
                include_system: if *system { Some(true) } else { None },
                include_generated: if *generated { Some(true) } else { None },
                case_sensitive: if *case_sensitive { Some(true) } else { None },
                whole_word: if *whole_word { Some(true) } else { None },
                max_results: *max,
            }),
            _ => None,
        }
    }

    /// Search/graph commands as one [`CodeCall`] for main dispatch.
    #[must_use]
    pub fn to_code(&self, ns_override: &Option<String>) -> Option<CodeCall> {
        match self {
            Self::Search { .. } => self.to_search(ns_override).map(CodeCall::Search),
            Self::Graph(_) => self.to_graph(ns_override).map(CodeCall::Graph),
            _ => None,
        }
    }

    /// Shared args for the graph tools, or None when the command isn't
    /// `graph`. The three graph entry points share no args struct, so this
    /// hands back the enum of conversions for main dispatch.
    #[must_use]
    pub fn to_graph(&self, ns_override: &Option<String>) -> Option<GraphCall> {
        match self {
            Self::Graph(g) => Some(match g {
                GraphCommands::Subclasses { class, depth } => GraphCall::Subclasses {
                    args: FindSubclassesArgs {
                        class: class.clone(),
                        namespace: ns_override.clone(),
                        depth: *depth,
                    },
                },
                GraphCommands::Members {
                    class,
                    kind,
                    include_inherited,
                } => GraphCall::Members {
                    args: ClassMembersArgs {
                        class: class.clone(),
                        namespace: ns_override.clone(),
                        kind: Some(match kind {
                            KindArg::All => MembersKind::All,
                            KindArg::Methods => MembersKind::Methods,
                            KindArg::Properties => MembersKind::Properties,
                            KindArg::Parameters => MembersKind::Parameters,
                            KindArg::Xdata => MembersKind::XData,
                        }),
                        include_inherited: if *include_inherited { Some(true) } else { None },
                    },
                },
                GraphCommands::Deps {
                    class,
                    direction,
                    depth,
                } => GraphCall::Deps {
                    args: ClassDependenciesArgs {
                        class: class.clone(),
                        namespace: ns_override.clone(),
                        direction: Some(match direction {
                            DirectionArg::DependsOn => DependencyDirection::DependsOn,
                            DirectionArg::UsedBy => DependencyDirection::UsedBy,
                            DirectionArg::Both => DependencyDirection::Both,
                        }),
                        depth: *depth,
                    },
                },
            }),
            _ => None,
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
                DocCommands::Get { name, save } => DocCall::Get {
                    args: GetDocumentArgs {
                        name: name.clone(),
                        namespace: ns_override.clone(),
                    },
                    save: save.clone(),
                },
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
            | Self::Mcp { .. }
            | Self::Search { .. }
            | Self::Graph(_)
            | Self::Compile { .. }
            | Self::Info
            | Self::Exec { .. }
            | Self::Test(_)
            | Self::Debug(_)
            | Self::Monitor { .. }
            | Self::Shell { .. }
            | Self::Cat { .. }
            | Self::Ls { .. } => None,
        }
    }
}

/// A dispatched search/graph call.
#[derive(Debug)]
pub enum CodeCall {
    /// search
    Search(SearchCodeArgs),
    /// graph family
    Graph(GraphCall),
}

/// A dispatched graph call.
#[derive(Debug)]
pub enum GraphCall {
    /// subclasses
    Subclasses {
        /// Shared args.
        args: FindSubclassesArgs,
    },
    /// members
    Members {
        /// Shared args.
        args: ClassMembersArgs,
    },
    /// deps
    Deps {
        /// Shared args.
        args: ClassDependenciesArgs,
    },
}

/// A dispatched document call (content filled from file at exec time).
#[derive(Debug)]
pub enum DocCall {
    /// list
    List(ListDocumentsArgs),
    /// get (with optional local save)
    Get {
        /// Shared args.
        args: GetDocumentArgs,
        /// Local path to save content to.
        save: Option<String>,
    },
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
