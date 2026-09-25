//! Shared tool logic — the single source of behaviour for both the CLI and
//! the MCP server. Framework-free: no rmcp or clap types cross this boundary
//! (documentation/rust-bestpractices.md §3).

pub mod command;
pub mod compile;
pub mod documents;
pub mod serverinfo;
pub mod sql;
