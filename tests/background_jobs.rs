//! Live MCP-surface contract for background terminal work via MCP Tasks
//! (SEP-2663).
//!
//! Talks JSON-RPC over stdio to the built `rism mcp` binary — the exact
//! contract a Tasks-capable MCP client sees: schemas + annotations,
//! task-mode start (non-blocking), streamed growth between polls via
//! `tasks/get`, real server-side cancel via `tasks/cancel` with a frozen
//! final state, the sync-result equivalence of completed tasks, the
//! Rail-A refusal for non-declaring clients, cap/unknown-id errors.
//! Gated `#[ignore]`: CI runs it in the live-smoke job against a fresh
//! IRIS; locally, `cargo test -- --ignored` with a dev container up.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    // json!() temporaries by value is the natural shape of a test RPC client
    clippy::needless_pass_by_value
)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Minimal line-delimited JSON-RPC client over the MCP stdio door.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// The `initialize` response — what the server advertised.
    init: Value,
}

impl Mcp {
    /// Client that declares the SEP-2663 Tasks extension.
    fn spawn() -> Self {
        Self::spawn_with(serde_json::json!({
            "extensions": {"io.modelcontextprotocol/tasks": {}}
        }))
    }

    /// Client that does NOT declare Tasks (Rail-A path).
    fn spawn_plain() -> Self {
        Self::spawn_with(serde_json::json!({}))
    }

    fn spawn_with(capabilities: Value) -> Self {
        // locate the real binary: cargo sets CARGO_BIN_EXE_rism for tests
        let exe = env!("CARGO_BIN_EXE_rism");
        let mut child = Command::new(exe)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn rism mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut m = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
            init: Value::Null,
        };
        m.init = m.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                "clientInfo": {"name": "integration", "version": "0"},
                "capabilities": capabilities
            }),
        );
        m.notify("notifications/initialized", serde_json::json!({}));
        m
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(serde_json::json!({"jsonrpc":"2.0","method":method,"params":params}));
    }

    /// One request; returns the full response message of the matching id,
    /// skipping interleaved responses (rmcp may answer out of order).
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        for _ in 0..64 {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).unwrap();
            assert!(read != 0, "mcp stdout closed");
            let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
        panic!("no response for id {id}")
    }

    /// `tools/call` → (`is_error`, payload, text). `is_error` covers BOTH
    /// failure channels: the tool-level `result.isError` and a JSON-RPC
    /// `error` object (which task-creation refusals use — no task exists
    /// to carry the failure). Text is the content block or error message.
    fn call(&mut self, tool: &str, args: Value) -> (bool, Value, String) {
        let msg = self.rpc(
            "tools/call",
            serde_json::json!({"name": tool, "arguments": args}),
        );
        if let Some(err) = msg.get("error") {
            let text = err["message"].as_str().unwrap_or_default().to_string();
            return (true, Value::Null, text);
        }
        let res = msg.get("result").cloned().unwrap_or(Value::Null);
        let is_err = res.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let text = res["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let parsed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
        (is_err, parsed, text)
    }

    /// `execute_command(background=true)` as a Tasks client: must answer
    /// with a task handle (resultType "task", status "working"), and
    /// returns the full `CreateTaskResult` payload.
    fn task_start(&mut self, args: Value) -> Value {
        let msg = self.rpc(
            "tools/call",
            serde_json::json!({
                "name": "execute_command",
                "arguments": {
                    "background": true,
                    "command": args["command"],
                    "timeout_secs": args["timeout_secs"],
                }
            }),
        );
        let res = msg.get("result").cloned().unwrap_or(Value::Null);
        assert_eq!(res["resultType"], "task", "expected task handle: {res}");
        res
    }

    fn task_id(res: &Value) -> String {
        res["taskId"].as_str().unwrap().to_string()
    }

    /// `tasks/get` → the `DetailedTask` result (flattened wire shape:
    /// taskId/status/statusMessage/... + result/error per status).
    fn task(&mut self, id: &str) -> Value {
        self.rpc("tasks/get", serde_json::json!({"taskId": id}))
            .get("result")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn task_cancel(&mut self, id: &str) -> Value {
        self.rpc("tasks/cancel", serde_json::json!({"taskId": id}))
    }

    /// Poll `tasks/get` until the status leaves `working` (budgeted).
    fn wait_terminal(&mut self, id: &str, budget: Duration) -> Value {
        let t0 = Instant::now();
        loop {
            let t = self.task(id);
            if t["status"] != "working" {
                return t;
            }
            assert!(t0.elapsed() < budget, "task {id} never finished");
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// Byte count carried by a statusMessage ("streamed N bytes").
    fn streamed_count(task: &Value) -> u64 {
        Self::message_count(task["statusMessage"].as_str().unwrap_or_default())
    }

    /// Parse the count from "streamed N bytes" or the cancelled variant
    /// "cancelled (interrupted), streamed N bytes" — locate the token
    /// AFTER the word `streamed`, never by fixed position (a positional
    /// nth(1) reads `(interrupted),` as 0 and silently vacuums the
    /// frozen-after-cancel assertion).
    fn message_count(msg: &str) -> u64 {
        let mut it = msg.split_whitespace();
        while let Some(w) = it.next() {
            if w == "streamed" {
                return it
                    .next()
                    .and_then(|n| n.trim_end_matches(',').parse().ok())
                    .unwrap_or(0);
            }
        }
        0
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn message_count_parser_handles_both_shapes() {
    assert_eq!(Mcp::message_count("streamed 82 bytes"), 82);
    assert_eq!(
        Mcp::message_count("cancelled (interrupted), streamed 153 bytes"),
        153,
        "cancelled shape parsed positionally would yield 0"
    );
    assert_eq!(Mcp::message_count("completed"), 0);
}

/// `YYYY-MM-DDTHH:MM:SSZ` check without pulling in a date crate. The year
/// floor catches the iso8601 off-by-days bug live (it drifted to 1969).
fn is_iso8601(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |mut r: std::ops::Range<usize>| r.all(|i| b[i].is_ascii_digit());
    b.len() == 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && b[19] == b'Z'
        && s[0..4].parse::<u32>().unwrap_or(0) >= 2024
}

/// ~20 s of server-side work: 400 ticks, 50 ms apart (braces keep `h`
/// INSIDE the loop body — without them it completes instantly).
const LONG: &str = "for i=1:1:400 { write \"bg:\",i,!  h .05 }";

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn annotations_and_task_surface_on_the_wire() {
    let mut m = Mcp::spawn();
    let msg = m.rpc("tools/list", serde_json::json!({}));
    let tools = msg["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 25, "tool count contract (Tasks migration)");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // The three custom poll-tools are GONE (full removal, no fallbacks).
    for removed in [
        "execute_command_background",
        "command_status",
        "command_cancel",
    ] {
        assert!(!names.contains(&removed), "{removed} must be removed");
    }
    let find = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .clone()
    };
    let ec = find("execute_command");
    assert_eq!(
        ec["annotations"]["destructiveHint"],
        serde_json::json!(true),
        "ObjectScript can mutate data"
    );
    assert!(
        ec["annotations"]["title"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "execute_command needs a title"
    );
    assert!(
        ec["inputSchema"]["properties"]["background"].is_object(),
        "background param must be in the schema"
    );
    // Tasks extension advertised on initialize (checked by every spawn())
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn server_advertises_tasks_extension() {
    // A fresh handshake (spawn already did it) is the proof: the server's
    // initialize response must carry the extension + correct serverInfo.
    let m = Mcp::spawn();
    let caps = &m.init["result"]["capabilities"];
    assert!(
        caps["extensions"]["io.modelcontextprotocol/tasks"].is_object(),
        "server must advertise the Tasks extension: {caps}"
    );
    assert!(
        caps["tools"].is_object(),
        "tools capability survives the manual get_info(): {caps}"
    );
    assert_eq!(m.init["result"]["serverInfo"]["name"], "rism");
    // version = env!("CARGO_PKG_VERSION"), not the old hard-coded 0.1.0
    let want = env!("CARGO_PKG_VERSION");
    assert_eq!(
        m.init["result"]["serverInfo"]["version"].as_str(),
        Some(want),
        "serverInfo.version must track the crate version"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn task_lifecycle_start_stream_cancel_frozen() {
    let mut m = Mcp::spawn();
    // start is non-blocking even for a ~20 s workload
    let t0 = Instant::now();
    let res = m.task_start(serde_json::json!({"command": LONG, "timeout_secs": 120}));
    assert!(t0.elapsed() < Duration::from_secs(2), "start blocked");
    assert_eq!(res["status"], "working");
    let id = Mcp::task_id(&res);
    assert_eq!(
        res["ttlMs"],
        serde_json::json!(1_800_000),
        "retention as ttlMs"
    );
    assert!(res["pollIntervalMs"].is_number(), "poll cadence advertised");
    assert!(is_iso8601(res["createdAt"].as_str().unwrap()));

    // streaming: strictly more output between two polls while working
    std::thread::sleep(Duration::from_secs(2));
    let a = m.task(&id);
    assert_eq!(a["status"], "working", "still running");
    let ca = Mcp::streamed_count(&a);
    assert!(ca > 0, "output did not stream: {a}");
    std::thread::sleep(Duration::from_secs(2));
    let b = m.task(&id);
    let cb = Mcp::streamed_count(&b);
    assert!(cb > ca, "output did not grow between polls: {ca} -> {cb}");

    // cancel: real server-side interrupt, well before natural end
    let t0 = Instant::now();
    let c = m.task_cancel(&id);
    assert!(c.get("error").is_none(), "tasks/cancel failed: {c}");
    let done = m.wait_terminal(&id, Duration::from_secs(15));
    assert!(t0.elapsed() < Duration::from_secs(10), "cancel too slow");
    assert_eq!(done["status"], "cancelled");
    assert!(
        done.get("result").is_none() && done.get("error").is_none(),
        "cancelled payload carries neither result nor error: {done}"
    );
    assert!(done["lastUpdatedAt"].as_str().is_some_and(is_iso8601));

    // frozen state: output stops growing (loop truly dead server-side)
    let chars = Mcp::streamed_count(&done);
    std::thread::sleep(Duration::from_secs(2));
    let later = m.task(&id);
    assert_eq!(
        Mcp::streamed_count(&later),
        chars,
        "server-side loop still ticking after cancel"
    );
    // cancel on a terminal task: safe no-op
    let c2 = m.task_cancel(&id);
    assert!(c2.get("error").is_none(), "re-cancel must not error: {c2}");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn quick_task_completes_with_the_sync_result() {
    let mut m = Mcp::spawn();
    // MULTI-frame on purpose: each `write !` inside the loop is a separate
    // terminal frame, and the sync path joins frames with \n while the
    // streaming tail concatenates them raw. A single-frame command would
    // pass both paths identically and mask the divergence (it did, once).
    let cmd = "for i=1:1:4 { write \"quick\",i,!  h .15 }";
    let res = m.task_start(serde_json::json!({"command": cmd}));
    let id = Mcp::task_id(&res);
    let done = m.wait_terminal(&id, Duration::from_secs(15));
    assert_eq!(done["status"], "completed");
    assert_eq!(done["resultType"], "complete");
    // Spec MUST: the completed payload is the EXACT CallToolResult the
    // synchronous call would have returned — prove it by running the sync
    // command and diffing the two CallToolResults.
    let payload = &done["result"];
    assert_eq!(payload["isError"], serde_json::json!(false));
    let task_out: Value =
        serde_json::from_str(payload["content"][0]["text"].as_str().unwrap()).unwrap();
    let (is_err, sync_out, _) = m.call("execute_command", serde_json::json!({"command": cmd}));
    assert!(!is_err);
    assert_eq!(
        task_out["output"], sync_out["output"],
        "task result must equal the sync result"
    );
    assert_eq!(task_out["namespace"], sync_out["namespace"]);
    assert!(task_out["output"].as_str().unwrap().contains("quick"));
    assert!(
        task_out["prompt"]
            .as_str()
            .unwrap_or_default()
            .contains('>')
    );
    // task bookkeeping survives completion
    assert!(done["createdAt"].as_str().is_some_and(is_iso8601));
    assert!(is_iso8601(done["lastUpdatedAt"].as_str().unwrap()));
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn rail_a_and_bad_task_ids_are_errors_not_crashes() {
    // Rail A: non-declaring client asking for background gets an honest
    // tool-level error (never a silent 60-s timeout).
    let mut plain = Mcp::spawn_plain();
    let (is_err, _, text) = plain.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true}),
    );
    assert!(is_err, "background must refuse for non-declaring clients");
    assert!(
        text.lowercase_contains("tasks extension"),
        "refusal must name the Tasks extension: {text}"
    );
    // Rail-A fires BEFORE schema validation: even a payload that would
    // fail deny_unknown_fields gets the guidance, not a bare -32602
    // (a declared client with the same broken payload still gets -32602
    // — the parse branch runs only inside the declared path).
    let (is_err, _, text) = plain.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true, "force": true}),
    );
    assert!(is_err);
    assert!(
        text.lowercase_contains("tasks extension"),
        "schema-invalid + non-declaring must still get Rail-A: {text}"
    );
    // tasks/* methods from a non-declaring client: protocol error (rmcp
    // validates the capability before the handler runs).
    let msg = plain.rpc("tasks/get", serde_json::json!({"taskId": "whatever"}));
    assert!(msg.get("error").is_some(), "must reject: {msg}");
    drop(plain);

    // Declared client: unknown ids get a clean JSON-RPC error with the
    // task-facing phrasing.
    let mut m = Mcp::spawn();
    let msg = m.rpc("tasks/get", serde_json::json!({"taskId": "bogus"}));
    assert!(msg.get("error").is_some());
    let get_err = msg["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        get_err.lowercase_contains("unknown or expired"),
        "hint missing: {msg}"
    );
    let msg = m.rpc("tasks/cancel", serde_json::json!({"taskId": "bogus"}));
    assert!(msg.get("error").is_some());
    // tasks/update: never offers input — honest refusal, not a hang.
    let msg = m.rpc(
        "tasks/update",
        serde_json::json!({"taskId": "bogus", "requestId": "r", "input": {}}),
    );
    assert!(msg.get("error").is_some(), "update_task must refuse: {msg}");
    // unknown field rejected (deny_unknown_fields)
    let (is_err, _, _) = m.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "force": true}),
    );
    assert!(is_err, "deny_unknown_fields must reject");
    // missing required command rejected
    let (is_err, _, _) = m.call("execute_command", serde_json::json!({}));
    assert!(is_err);
    // server still healthy
    let (is_err, p, _) = m.call(
        "execute_command",
        serde_json::json!({"command": "write \"healthy\",!"}),
    );
    assert!(!is_err, "door died: {p}");
    assert!(p["output"].as_str().unwrap().contains("healthy"));
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn running_cap_enforced_then_releasable() {
    let mut m = Mcp::spawn();
    let mut live = Vec::new();
    // ~15 s jobs so they outlive the start loop itself
    for _ in 0..16 {
        let res = m.task_start(serde_json::json!({
            "command": "for j=1:1:300 { write \"cap:\",j,!  h .05 }",
            "timeout_secs": 60
        }));
        live.push(Mcp::task_id(&res));
    }
    // the 17th must be refused, actionably (task-creation refusal is a
    // JSON-RPC error — there is no task to carry it)
    let msg = m.rpc(
        "tools/call",
        serde_json::json!({
            "name": "execute_command",
            "arguments": {"command": "write 1", "timeout_secs": 60, "background": true}
        }),
    );
    let err_text = msg
        .get("error")
        .map(|e| e["message"].as_str().unwrap_or_default().to_lowercase())
        .unwrap_or_default();
    assert!(
        !err_text.is_empty(),
        "cap not enforced (17th started): {msg}"
    );
    assert!(
        err_text.contains("too many"),
        "cap error must say too many: {err_text}"
    );
    // releases: cancelling frees slots (cancel-all, verify all terminal)
    for id in &live {
        let c = m.task_cancel(id);
        assert!(c.get("error").is_none(), "cancel {id} failed: {c}");
    }
    for id in &live {
        let t = m.wait_terminal(id, Duration::from_secs(20));
        assert_eq!(t["status"], "cancelled", "task {id} must land cancelled");
    }
    let res = m.task_start(serde_json::json!({
        "command": "write \"slot-free\",!",
        "timeout_secs": 30
    }));
    let id = Mcp::task_id(&res);
    let done = m.wait_terminal(&id, Duration::from_secs(15));
    assert_eq!(
        done["status"], "completed",
        "slot not reclaimed after cancels"
    );
}

/// Small helper so error-text asserts read naturally.
trait TextExt {
    fn lowercase_contains(&self, needle: &str) -> bool;
}
impl TextExt for String {
    fn lowercase_contains(&self, needle: &str) -> bool {
        self.to_lowercase().contains(needle)
    }
}
