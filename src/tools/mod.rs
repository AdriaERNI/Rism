//! Shared tool logic — the single source of behaviour for both the CLI and
//! the MCP server. Framework-free: no rmcp or clap types cross this boundary
//! (documentation/rust-bestpractices.md §3).

pub mod command;
pub mod compile;
pub mod debugger;
pub mod documents;
pub mod host;
pub mod jobs;
pub mod monitor;
pub mod repl;
pub mod scoring;
pub mod serverinfo;
pub mod sql;
pub mod testing;

/// Wall-clock ceiling for any model-supplied timeout (7 days).
pub const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;

/// Seconds -> capped seconds, so `Instant + Duration` cannot overflow:
/// an absurd `timeout_secs` (issue #16: `u64::MAX`) panicked the arithmetic
/// silently — the request never answered. Clamps, never rejects: every
/// request must ANSWER. Apply at every ingress AFTER the `unwrap_or`, so
/// oversized config defaults are covered too.
#[must_use]
pub fn clamp_timeout_secs(secs: u64) -> u64 {
    secs.min(MAX_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_timeout_secs_bounds_absurd_values() {
        assert_eq!(clamp_timeout_secs(u64::MAX), MAX_TIMEOUT_SECS);
        assert_eq!(clamp_timeout_secs(0), 0);
        assert_eq!(clamp_timeout_secs(MAX_TIMEOUT_SECS + 1), MAX_TIMEOUT_SECS);
        let sane = 3600;
        assert_eq!(clamp_timeout_secs(sane), sane);
    }
}
