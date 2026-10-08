//! Shared line-delimited JSON-RPC client over the MCP stdio door.
//!
//! NOT a test target on its own (a `tests/` subdirectory is never
//! autodiscovered): consumers include it with
//! `#[path = "support/mcp_harness.rs"] mod harness; use harness::*;` so
//! `background_jobs` and `tool_matrix` exercise ONE harness, not two
//! forks (mission Scope B: no fork-duplication).
#![allow(
    dead_code, // helpers are shared; not every consumer uses all
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    // json!() temporaries by value is the natural shape of a test RPC client
    clippy::needless_pass_by_value
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
// re-exposed for the including test files (a private `use` is invisible
// to `use harness::*`)
pub use std::time::{Duration, Instant};

pub use serde_json::Value;

/// Minimal line-delimited JSON-RPC client over the MCP stdio door.
pub struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// The `initialize` response — what the server advertised.
    pub init: Value,
    /// Protocol version the server negotiated (initialize result).
    pub version: String,
    /// SEP-2575 per-request `_meta` injected into every `rpc` call when
    /// the session uses the inline lifecycle (None = classic initialize
    /// handshake). Tasks-capable `spawn` MUST use it: rmcp 3.4.1 can
    /// never negotiate >= 2026-06-30 over initialize (no such const — it
    /// falls back to 2025-11-25), and under that version SEP-2663 says
    /// the extension key MUST be inert. The inline form (no handshake,
    /// `io.modelcontextprotocol/protocolVersion: 2026-07-28` +
    /// clientCapabilities per request) is the only spec-coherent tasks
    /// path this SDK can serve today — verified live.
    meta: Option<Value>,
    /// Server stderr, appended live by a drain thread (Prism-parity DEBUG
    /// banners land here under `RUST_LOG=debug`; a pipe nobody drains fills
    /// and deadlocks a chatty server).
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Mcp {
    /// Tasks-capable client on the SEP-2575 inline lifecycle: NO
    /// initialize handshake; every request carries protocolVersion
    /// 2026-07-28 + the Tasks extension key in its `_meta`. That version
    /// is >= our 2026-06-30 floor, so tasks are live (SEP-2663 canonical
    /// row). Over stdio today the inline lifecycle is the ONLY way to
    /// reach a >= 2026-06-30 session — rmcp 3.4.1 cannot negotiate a 2026
    /// version via `initialize` (see docs in documentation/mcp.md).
    pub fn spawn() -> Self {
        Self::spawn_with_env(&[])
    }

    /// [`spawn`] plus extra environment for the server process — the
    /// spawned `rism mcp` re-reads env at startup, so settings a contract
    /// test must pin (`RISM_WORKSPACE` for the host file tools,
    /// `RISM_TERMINAL_MAX_OUTPUT_CHARS` for truncation) only steer the
    /// door when set HERE; the test process's own env does not reach it.
    pub fn spawn_with_env(extra_env: &[(&str, &str)]) -> Self {
        let mut m = Self::raw_with_env(extra_env);
        m.meta = Some(serde_json::json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "integration", "version": "0"},
            "io.modelcontextprotocol/clientCapabilities": {
                "extensions": {"io.modelcontextprotocol/tasks": {}}
            }
        }));
        m.version = "2026-07-28".to_string();
        m
    }

    /// Client that does NOT declare Tasks (Rail-A path), classic handshake.
    pub fn spawn_plain() -> Self {
        Self::spawn_with(serde_json::json!({}))
    }

    /// Handshake client declaring `capabilities` on initialize.
    pub fn spawn_with(capabilities: Value) -> Self {
        let mut m = Self::raw();
        m.init = m.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2026-06-30",
                "clientInfo": {"name": "integration", "version": "0"},
                "capabilities": capabilities
            }),
        );
        m.notify("notifications/initialized", serde_json::json!({}));
        m.version = m.init["result"]["protocolVersion"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        m
    }

    /// Spawn the server process with NO protocol traffic yet.
    pub fn raw() -> Self {
        Self::raw_with_env(&[])
    }

    /// [`raw`] with extra environment vars on the server process.
    pub fn raw_with_env(extra_env: &[(&str, &str)]) -> Self {
        // locate the real binary: cargo sets CARGO_BIN_EXE_rism for tests
        let exe = env!("CARGO_BIN_EXE_rism");
        let mut cmd = Command::new(exe);
        cmd.arg("mcp")
            .env("RUST_LOG", "debug") // Prism-parity banners land on stderr
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn rism mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr = Arc::new(Mutex::new(Vec::new()));
        {
            // Drain thread: a nobody-reads pipe fills (64 KB) and deadlocks
            // the server; chunks append live so assertions can read stderr
            // WHILE the server is still running.
            let sink = Arc::clone(&stderr);
            let mut pipe = child.stderr.take().unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => sink
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend_from_slice(&buf[..n]),
                    }
                }
            });
        }
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
            init: Value::Null,
            version: String::new(),
            meta: None,
            stderr,
        }
    }

    pub fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(serde_json::json!({"jsonrpc":"2.0","method":method,"params":params}));
    }

    /// One request; returns the full response message of the matching id,
    /// skipping interleaved responses (rmcp may answer out of order).
    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let params = match &self.meta {
            Some(meta) if method != "initialize" => {
                let mut p = params.clone();
                p["_meta"] = meta.clone();
                p
            }
            _ => params,
        };
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
    pub fn call(&mut self, tool: &str, args: Value) -> (bool, Value, String) {
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
    pub fn task_start(&mut self, args: Value) -> Value {
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

    pub fn task_id(res: &Value) -> String {
        res["taskId"].as_str().unwrap().to_string()
    }

    /// Everything the server has written to stderr so far (live snapshot;
    /// the drain thread appends as chunks arrive).
    pub fn stderr_text(&self) -> String {
        let guard = self
            .stderr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        String::from_utf8_lossy(&guard).to_string()
    }

    /// [`stderr_text`] until `needle` shows up or the budget runs out (log
    /// writes and stdout responses travel on different fds; the drain
    /// thread can lag the response by microseconds).
    pub fn wait_stderr_contains(&self, needle: &str, budget: Duration) -> bool {
        let t0 = Instant::now();
        loop {
            if self.stderr_text().contains(needle) {
                return true;
            }
            if t0.elapsed() >= budget {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `tasks/get` → the `DetailedTask` result (flattened wire shape:
    /// taskId/status/statusMessage/... + result/error per status).
    pub fn task(&mut self, id: &str) -> Value {
        self.rpc("tasks/get", serde_json::json!({"taskId": id}))
            .get("result")
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub fn task_cancel(&mut self, id: &str) -> Value {
        self.rpc("tasks/cancel", serde_json::json!({"taskId": id}))
    }

    /// Poll `tasks/get` until the status leaves `working` (budgeted).
    pub fn wait_terminal(&mut self, id: &str, budget: Duration) -> Value {
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
    pub fn streamed_count(task: &Value) -> u64 {
        Self::message_count(task["statusMessage"].as_str().unwrap_or_default())
    }

    /// Parse the count from "streamed N bytes" or the cancelled variant
    /// "cancelled (interrupted), streamed N bytes" — locate the token
    /// AFTER the word `streamed`, never by fixed position (a positional
    /// nth(1) reads `(interrupted),` as 0 and silently vacuums the
    /// frozen-after-cancel assertion).
    pub fn message_count(msg: &str) -> u64 {
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

/// `YYYY-MM-DDTHH:MM:SSZ` check without pulling in a date crate. The year
/// floor catches the iso8601 off-by-days bug live (it drifted to 1969).
pub fn is_iso8601(s: &str) -> bool {
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

/// Small helper so error-text asserts read naturally.
pub trait TextExt {
    fn lowercase_contains(&self, needle: &str) -> bool;
}
impl TextExt for String {
    fn lowercase_contains(&self, needle: &str) -> bool {
        self.to_lowercase().contains(needle)
    }
}
