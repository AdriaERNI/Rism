//! Rism — a Rust client library for `InterSystems` IRIS development via the
//! Atelier REST API, consumed identically by the terminal CLI (`rism ...`) and
//! the MCP server (`rism mcp`).
//!
//! Layering (top → bottom):
//!
//! - [`tools`] — framework-free shared logic. **The only place tool behaviour
//!   lives.** Both front-door adapters call these functions.
//! - [`cli`] / [`mcp`] — thin adapters: parse their input shape, call a
//!   `tools` function, render/return the result. No logic here.
//! - [`iris`] — Atelier REST client layer (HTTP, envelope handling, endpoints).
//! - [`settings`] / [`error`] — configuration and the domain error type.

// Note: `unsafe_code = forbid` and the clippy gates are set crate-wide in
// Cargo.toml under [lints]; see documentation/rust-bestpractices.md §2.

pub mod cli;
pub mod error;
pub mod iris;
pub mod mcp;
pub mod settings;
pub mod tools;

pub use error::{Error, Result};
pub use settings::Settings;
