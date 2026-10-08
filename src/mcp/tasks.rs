//! SEP-2663 Tasks projection over the background-job registry.
//!
//! The registry in [`crate::tools::jobs`] already IS a task store: durable ids,
//! 30-min retention (→ `ttlMs`), streamed output tail, cooperative cancel.
//! This module is the pure mapping from a [`JobInfo`] snapshot to the wire
//! types rmcp serves (`CreateTaskResult` for tool calls, `DetailedTask` for
//! `tasks/get`) — zero I/O, unit-testable without a server.
//!
//! Status mapping (spec §Task lifecycle):
//! - running                → `working`   (statusMessage = bytes streamed)
//! - finished, no error     → `completed` (result = the sync `CallToolResult`)
//! - finished, transport/
//!   timeout error          → `failed`    (JSON-RPC error object)
//! - finished by interrupt  → `cancelled` (wire shape carries no result;
//!   the byte count rides in statusMessage)
//!
//! Program-level failures (`<SYNTAX>`, `<ERROR>`) are command OUTPUT, not
//! request failure — they complete the task exactly like the synchronous
//! `execute_command` would.

use rmcp::model::{
    CallToolResult, ContentBlock, CreateTaskResult, DetailedTask, Task, TaskPayload, TaskStatus,
};

use crate::tools::command::CommandResult;
use crate::tools::jobs::{JobInfo, RETENTION};

/// Suggested client polling cadence advertised in every task (ms). The
/// registry refreshes continuously from the terminal stream, so 1 s is a
/// courtesy bound, not a data-availability interval.
pub const POLL_INTERVAL_MS: u64 = 1_000;

/// Retention advertised as `ttlMs` (ms). Exact: RETENTION is whole seconds.
fn ttl_ms() -> u64 {
    RETENTION.as_secs() * 1000
}

/// ISO 8601 UTC from unix seconds (civil-from-days algorithm, Howard
/// Hinnant; no chrono dependency). Inputs are unix seconds from the job
/// registry ([0, ~`u32::MAX`] in practice), so the signed casts cannot wrap.
#[allow(
    clippy::many_single_char_names,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "faithful civil_from_days algorithm; the short names are the published derivation"
)]
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil_from_days: days since 1970-01-01 -> (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{year:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Seed task returned from a task-mode `tools/call` (spec: `resultType:
/// "task"`, status `working`).
#[must_use]
pub fn create_task(info: &JobInfo) -> CreateTaskResult {
    CreateTaskResult::new(base_task(info, TaskStatus::Working))
}

/// Result of `tasks/get`: the same projection plus the status payload.
#[must_use]
pub fn detailed_task(info: &JobInfo) -> DetailedTask {
    let (status, payload) = task_state(info);
    let mut task = base_task(info, status);
    // The registry has no per-frame update stamp; the finish time is the
    // last observable mutation (started otherwise).
    task.last_updated_at = iso8601(info.finished_unix.unwrap_or(info.started_unix));
    DetailedTask::new(task, payload)
}

/// Status + payload for a snapshot (table-driven unit tests target this).
#[must_use]
pub fn task_state(info: &JobInfo) -> (TaskStatus, TaskPayload) {
    if info.running {
        return (TaskStatus::Working, TaskPayload::Working);
    }
    if let Some(err) = &info.error {
        // Transport/timeout failure: a JSON-RPC-style error object inside
        // the failed task (spec §Failed tasks).
        let error = serde_json::json!({ "code": -32000, "message": err });
        let map = match error {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        return (TaskStatus::Failed, TaskPayload::Failed { error: map });
    }
    if info.interrupted {
        // Cancel is terminal. The wire TaskPayload::Cancelled carries no
        // result field (rmcp/spec shape), so the partial output is not
        // re-served here — the command stopped with what it executed.
        return (TaskStatus::Cancelled, TaskPayload::Cancelled);
    }
    (TaskStatus::Completed, payload_from(&sync_result(info)))
}

fn base_task(info: &JobInfo, status: TaskStatus) -> Task {
    Task::new(
        info.id.clone(),
        status,
        iso8601(info.started_unix),
        iso8601(info.started_unix),
    )
    .with_status_message(status_message(info))
    .with_ttl_ms(ttl_ms())
    .with_poll_interval_ms(POLL_INTERVAL_MS)
}

/// The exact synchronous `CommandResult` the blocking call would have
/// returned (spec: completed tasks' result MUST match the sync result).
/// `jobs::finish` stores the joined terminal outcome in `final_result` at
/// completion — byte-equal by construction, since it IS the sync path's
/// `TerminalOutcome`. The streaming `output` tail concatenates raw frames
/// WITHOUT the sync frame join, so reconstructing from it would silently
/// diverge for multi-frame commands (proven live; the equivalence test
/// runs a 4-frame command on both paths). A finished job always has it;
/// the reconstruct branch is a defensive fallback, not the normal path.
fn sync_result(info: &JobInfo) -> CommandResult {
    if let Some(final_result) = &info.final_result {
        return final_result.clone();
    }
    CommandResult {
        namespace: info.namespace.clone(),
        command: info.command.clone(),
        output: info.output.clone(),
        prompt: info.prompt.clone().unwrap_or_default(),
        // byte-vs-byte: `output_bytes` is the total streamed; `output` is
        // the retained tail (bytes). The wire key stays `output_omitted_chars`
        // (frozen name); on this defensive reconstruct branch the count is
        // bytes-accurate (issue #16 F2: was chars-vs-bytes nonsense).
        output_truncated: info.output_bytes > info.output.len(),
        output_omitted_chars: info.output_bytes.saturating_sub(info.output.len()),
    }
}

fn payload_from(result: &CommandResult) -> TaskPayload {
    let text = serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string());
    let cr = CallToolResult::success(vec![ContentBlock::text(text)]);
    match serde_json::to_value(&cr) {
        Ok(serde_json::Value::Object(obj)) => TaskPayload::Completed { result: obj },
        _ => TaskPayload::Completed {
            result: serde_json::Map::new(),
        },
    }
}

fn status_message(info: &JobInfo) -> String {
    if info.running {
        format!("streamed {} bytes", info.output_bytes)
    } else if info.interrupted {
        format!(
            "cancelled (interrupted), streamed {} bytes",
            info.output_bytes
        )
    } else if info.error.is_some() {
        "failed".to_string()
    } else {
        "completed".to_string()
    }
}

/// Message for an unknown / GC'd task id (spec: unknown task = error).
#[must_use]
pub fn unknown_task_msg(id: &str) -> String {
    format!("unknown or expired task id: {id}")
}

/// Registry error -> JSON-RPC error for `tasks/*` handlers. Unknown ids get
/// the task-facing phrasing (retention is 30 min); anything else (e.g. the
/// concurrency cap) propagates verbatim.
#[must_use]
pub fn task_error(err: &crate::error::Error, id: &str) -> rmcp::ErrorData {
    let msg = err.to_string();
    let friendly = if msg.contains("unknown job id") {
        unknown_task_msg(id)
    } else {
        msg
    };
    rmcp::ErrorData::invalid_params(friendly, None)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    fn job(running: bool, interrupted: bool, error: Option<&str>) -> JobInfo {
        JobInfo {
            id: "rism-test-0".into(),
            command: "h 600".into(),
            namespace: "USER".into(),
            started_unix: 1_759_480_000, // 2025-10-03T08:26:40Z
            finished_unix: if running { None } else { Some(1_759_480_060) },
            running,
            interrupted,
            error: error.map(str::to_string),
            prompt: if running { None } else { Some("USER>".into()) },
            output_bytes: 42,
            output: "tick".into(),
            final_result: if running {
                None
            } else {
                Some(CommandResult {
                    namespace: "USER".into(),
                    command: "h 600".into(),
                    // joined with the sync frame-join, unlike `output`
                    output: "tick\r\n".into(),
                    prompt: "USER>".into(),
                    output_truncated: false,
                    output_omitted_chars: 0,
                })
            },
        }
    }

    /// Issue-A regression pin (unit level): the completed payload MUST be
    /// the stored final outcome, never a reconstruction from the streaming
    /// tail — live-proof showed the tail lacks the sync frame-join for
    /// multi-frame commands.
    #[test]
    fn completed_payload_uses_final_result_not_streaming_tail() {
        let info = job(false, false, None);
        assert_ne!(
            info.output,
            info.final_result.as_ref().unwrap().output,
            "fixture must make tail and final differ"
        );
        let dt = detailed_task(&info);
        let TaskPayload::Completed { result } = &dt.payload else {
            panic!("expected completed");
        };
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("tick\\r\\n"), "joined output served: {text}");
        assert!(!text.contains("\"tick\""));
    }

    #[test]
    fn iso8601_known_values() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z", "epoch");
        assert_eq!(iso8601(1_759_480_000), "2025-10-03T08:26:40Z", "recent");
        assert_eq!(iso8601(1_735_689_600), "2025-01-01T00:00:00Z", "year flip");
        assert_eq!(
            iso8601(1_704_067_199),
            "2023-12-31T23:59:59Z",
            "leap boundary"
        );
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z", "unix 1e9");
        // century-leap boundary: the `yoe/100` term in doy. Regression pin
        // for the off-by-3-days bug (2200 is NOT a leap year).
        assert_eq!(iso8601(7_258_118_400), "2200-01-01T00:00:00Z", "2200");
    }

    /// Regression pin: `DetailedTask::new` overwrites the task status from
    /// the payload variant, so a Cancelled status must be paired with the
    /// Cancelled payload or the wire silently says "completed".
    #[test]
    fn wire_status_matches_state_for_every_shape() {
        for (info, want) in [
            (job(true, false, None), "working"),
            (job(false, false, None), "completed"),
            (job(false, true, None), "cancelled"),
            (job(false, false, Some("boom")), "failed"),
        ] {
            let v = serde_json::to_value(detailed_task(&info)).unwrap();
            assert_eq!(v["status"], want, "wire status for {want}");
        }
    }

    #[test]
    fn state_table() {
        assert_eq!(task_state(&job(true, false, None)).0, TaskStatus::Working);
        assert_eq!(
            task_state(&job(false, false, None)).0,
            TaskStatus::Completed
        );
        assert_eq!(task_state(&job(false, true, None)).0, TaskStatus::Cancelled);
        assert_eq!(
            task_state(&job(false, false, Some("boom"))).0,
            TaskStatus::Failed
        );
    }

    #[test]
    fn failed_carries_error_object() {
        let dt = detailed_task(&job(false, false, Some("terminal timeout")));
        let TaskPayload::Failed { error } = &dt.payload else {
            panic!("expected failed");
        };
        assert_eq!(error["message"], "terminal timeout");
        assert_eq!(error["code"], -32000);
    }

    #[test]
    fn completed_result_matches_sync_shape() {
        let dt = detailed_task(&job(false, false, None));
        let TaskPayload::Completed { result } = &dt.payload else {
            panic!("expected completed");
        };
        // A CallToolResult: content blocks (+ optional isError), per spec.
        assert!(result.contains_key("content"), "result carries content");
        let v = serde_json::to_value(&dt).expect("serializable");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["taskId"], "rism-test-0");
        assert_eq!(v["ttlMs"], 1_800_000, "retention advertised");
        assert_eq!(v["pollIntervalMs"], POLL_INTERVAL_MS);
        assert!(
            v["createdAt"].as_str().is_some_and(|s| s.ends_with('Z')),
            "createdAt is ISO 8601 UTC"
        );
        let expected =
            serde_json::to_string_pretty(&sync_result(&job(false, false, None))).unwrap();
        assert_eq!(
            v["result"]["content"][0]["text"].as_str().unwrap(),
            expected,
            "completed payload is the sync result verbatim"
        );
    }

    #[test]
    fn create_task_seed_is_working_with_progress_message() {
        let v = serde_json::to_value(create_task(&job(true, false, None))).expect("serializable");
        assert_eq!(v["resultType"], "task", "seed discriminator");
        assert_eq!(v["status"], "working");
        assert_eq!(v["statusMessage"], "streamed 42 bytes");
        assert_eq!(v["ttlMs"], 1_800_000);
    }
}
