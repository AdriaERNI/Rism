//! Live MCP-surface contract for the background terminal jobs.
//!
//! Talks JSON-RPC over stdio to the built `rism mcp` binary — the exact
//! contract an MCP client sees: schemas + annotations, non-blocking start,
//! streamed output growth between polls, real server-side cancel with a
//! frozen final state, roster/cap/unknown-id errors. Gated `#[ignore]`:
//! CI runs it in the live-smoke job against a fresh IRIS; locally,
//! `cargo test -- --ignored` with a dev container up.
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
}

impl Mcp {
    fn spawn() -> Self {
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
        };
        m.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "clientInfo": {"name": "integration", "version": "0"},
                "capabilities": {}
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

    /// One request; returns the result/error payload of the matching id,
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

    /// `tools/call` → (`is_error`, payload). The payload is the text content
    /// block, JSON-parsed when possible (`map_json`'s Err branch is plain
    /// text — callers assert on `is_error` + the raw string in that case).
    fn call(&mut self, tool: &str, args: Value) -> (bool, Value, String) {
        let msg = self.rpc(
            "tools/call",
            serde_json::json!({"name": tool, "arguments": args}),
        );
        let res = msg.get("result").cloned().unwrap_or(Value::Null);
        let is_err = res.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let text = res["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let parsed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
        (is_err, parsed, text)
    }

    fn job(&mut self, id: &str) -> Value {
        let (_, p, _) = self.call("command_status", serde_json::json!({"job_id": id}));
        p.get("job").cloned().unwrap_or(Value::Null)
    }

    fn wait_done(&mut self, id: &str, budget: Duration) -> Value {
        let t0 = Instant::now();
        loop {
            let j = self.job(id);
            if j.get("running").and_then(Value::as_bool) == Some(false) {
                return j;
            }
            assert!(t0.elapsed() < budget, "job {id} never finished");
            std::thread::sleep(Duration::from_millis(150));
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b""); // no-op; closing happens below
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// ~20 s of server-side work: 400 ticks, 50 ms apart (braces keep `h`
/// INSIDE the loop body — without them it completes instantly).
const LONG: &str = "for i=1:1:400 { write \"bg:\",i,!  h .05 }";

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn annotations_exposed_on_the_wire() {
    let mut m = Mcp::spawn();
    let msg = m.rpc("tools/list", serde_json::json!({}));
    let tools = msg["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 28, "tool count contract");
    let find = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .clone()
    };
    let ann = |name: &str| find(name)["annotations"].clone();
    assert_eq!(
        ann("command_status")["readOnlyHint"],
        serde_json::json!(true)
    );
    assert_eq!(
        ann("command_cancel")["destructiveHint"],
        serde_json::json!(false)
    );
    assert_eq!(
        ann("command_cancel")["idempotentHint"],
        serde_json::json!(true)
    );
    assert_eq!(
        ann("execute_command")["destructiveHint"],
        serde_json::json!(true)
    );
    assert_eq!(
        ann("execute_command_background")["destructiveHint"],
        serde_json::json!(true)
    );
    // titles present for human-readable UIs
    for n in [
        "execute_command",
        "execute_command_background",
        "command_status",
        "command_cancel",
    ] {
        assert!(
            find(n)["annotations"]["title"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "{n} needs a title"
        );
    }
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn background_lifecycle_start_stream_cancel_frozen() {
    let mut m = Mcp::spawn();
    // start is non-blocking even for a ~20 s workload
    let t0 = Instant::now();
    let (is_err, p, _) = m.call(
        "execute_command_background",
        serde_json::json!({"command": LONG, "timeout_secs": 120}),
    );
    assert!(!is_err, "start must succeed: {p}");
    assert!(t0.elapsed() < Duration::from_secs(2), "start blocked");
    assert_eq!(p["running"], serde_json::json!(true));
    let id = p["job_id"].as_str().unwrap().to_string();
    assert!(
        !p["namespace"].as_str().unwrap().is_empty(),
        "namespace reported"
    );

    // streaming: strictly more output between two polls while running
    std::thread::sleep(Duration::from_secs(2));
    let a = m.job(&id);
    assert_eq!(a["running"], serde_json::json!(true), "still running");
    assert!(a["output"].as_str().unwrap().contains("bg:"), "live tail");
    std::thread::sleep(Duration::from_secs(2));
    let b = m.job(&id);
    assert!(
        b["output_chars"].as_u64().unwrap() > a["output_chars"].as_u64().unwrap(),
        "output did not grow between polls: {} -> {}",
        a["output_chars"],
        b["output_chars"]
    );

    // cancel: real server-side interrupt, well before natural end
    let t0 = Instant::now();
    let (is_err, _, _) = m.call("command_cancel", serde_json::json!({"job_id": &id}));
    assert!(!is_err);
    let done = m.wait_done(&id, Duration::from_secs(15));
    assert!(t0.elapsed() < Duration::from_secs(10), "cancel too slow");
    assert_eq!(done["interrupted"], serde_json::json!(true));
    assert_eq!(done["running"], serde_json::json!(false));
    assert!(done["error"].is_null(), "cancel is not an error: {done}");
    assert!(done["finished_unix"].is_number());

    // frozen state: output stops growing (loop truly dead server-side)
    let chars = done["output_chars"].as_u64().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let later = m.job(&id);
    assert_eq!(
        later["output_chars"].as_u64().unwrap(),
        chars,
        "server-side loop still ticking after cancel"
    );
    // cancel on a finished job: safe no-op
    let (is_err, p, _) = m.call("command_cancel", serde_json::json!({"job_id": &id}));
    assert!(!is_err, "re-cancel must not error: {p}");
    assert_eq!(p["running"], serde_json::json!(false));
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn quick_job_completes_clean_and_listable() {
    let mut m = Mcp::spawn();
    let (_, p, _) = m.call(
        "execute_command_background",
        serde_json::json!({"command": "write \"quick\",!"}),
    );
    let id = p["job_id"].as_str().unwrap().to_string();
    let done = m.wait_done(&id, Duration::from_secs(15));
    assert_eq!(done["running"], serde_json::json!(false));
    assert_eq!(done["interrupted"], serde_json::json!(false));
    assert!(done["error"].is_null());
    assert!(done["output"].as_str().unwrap().contains("quick"));
    assert!(done["prompt"].as_str().unwrap_or_default().contains('>'));
    // roster: present, no bodies, newest-first timestamps
    let (_, lst, _) = m.call("command_status", serde_json::json!({}));
    let jobs = lst["jobs"].as_array().unwrap();
    assert!(jobs.iter().any(|j| j["id"].as_str() == Some(&id)));
    assert!(
        jobs.iter()
            .all(|j| j["output"].as_str().unwrap_or_default().is_empty()),
        "list view must omit output bodies"
    );
    let ts: Vec<u64> = jobs
        .iter()
        .map(|j| j["started_unix"].as_u64().unwrap())
        .collect();
    assert!(
        ts.windows(2).all(|w| w[0] >= w[1]),
        "not newest-first: {ts:?}"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn unknown_job_and_bad_input_are_tool_errors_not_crashes() {
    let mut m = Mcp::spawn();
    let (is_err, _, text) = m.call("command_status", serde_json::json!({"job_id": "bogus"}));
    assert!(is_err);
    assert!(
        text.lowercase_contains("unknown job"),
        "hint missing: {text}"
    );
    let (is_err, _, text) = m.call("command_cancel", serde_json::json!({"job_id": "bogus"}));
    assert!(is_err);
    assert!(text.lowercase_contains("unknown job"));
    // unknown field rejected (deny_unknown_fields)
    let (is_err, _, _) = m.call(
        "command_cancel",
        serde_json::json!({"job_id": "x", "force": true}),
    );
    assert!(is_err, "deny_unknown_fields must reject");
    // missing required command rejected
    let (is_err, _, _) = m.call("execute_command_background", serde_json::json!({}));
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
        let (is_err, p, _) = m.call(
            "execute_command_background",
            serde_json::json!({
                "command": "for j=1:1:300 { write \"cap:\",j,!  h .05 }",
                "timeout_secs": 60
            }),
        );
        assert!(!is_err, "start {}/16 failed: {p}", live.len() + 1);
        live.push(p["job_id"].as_str().unwrap().to_string());
    }
    // the 17th must be refused, actionably
    let (is_err, _, text) = m.call(
        "execute_command_background",
        serde_json::json!({"command": "write 1", "timeout_secs": 60}),
    );
    assert!(is_err, "cap not enforced (17th started)");
    assert!(
        text.lowercase_contains("too many"),
        "cap error must say too many: {text}"
    );
    // releases: cancelling frees slots (cancel-all, verify none left running)
    for id in &live {
        let (is_err, _, _) = m.call("command_cancel", serde_json::json!({"job_id": id}));
        assert!(!is_err, "cancel {id} failed");
    }
    for id in &live {
        m.wait_done(id, Duration::from_secs(20));
    }
    let (is_err, p, _) = m.call(
        "execute_command_background",
        serde_json::json!({"command": "write \"slot-free\",!", "timeout_secs": 30}),
    );
    assert!(!is_err, "slot not reclaimed after cancels: {p}");
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
