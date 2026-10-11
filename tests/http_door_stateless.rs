//! HTTP door, non-ignored contract (SPEC T2): the exact shipped wiring
//! (`serve_http_on`) on an ephemeral loopback port with NO IRIS needed —
//! `negotiate_version` is best-effort, so startup must succeed against a
//! dead URL and every tool call must ANSWER (a tool-level error), never
//! hang (the general error-path rule). Pins:
//!   - the per-request factory (rmcp calls it per session/request; the
//!     cheap-clone handle must produce working services repeatedly),
//!   - the stateful `Mcp-Session-Id` header contract (row D),
//!   - 25 tools on the wire, stdout untouched.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value
)]

use std::io::Read;
use std::time::{Duration, Instant};

/// P-07 / M8 pin: the ready line must arrive IMMEDIATELY AFTER BIND, not
/// after the handler build. `base_url` is a socket that accepts but never
/// answers, so the negotiate GET blocks for the full 30 s client timeout;
/// with the line at its OLD position it could only print at t~30 s, and
/// this asserts t < 3 s. Mutation M8 (move it back) goes RED.
#[test]
fn ready_line_precedes_handler_negotiate() {
    let blackhole = std::net::TcpListener::bind("127.0.0.1:0").expect("blackhole bind");
    let bh_addr = blackhole.local_addr().expect("addr");
    // Accept + hold every connection; never write a response (the thread
    // ends with the test binary).
    std::thread::spawn(move || {
        let _held: Vec<_> = blackhole.incoming().flatten().collect();
    });
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rism"))
        .args([
            "mcp",
            "--transport",
            "http",
            "--port",
            "0",
            "--url",
            &format!("http://{bh_addr}"),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rism http door");
    let mut reader = child.stderr.take().expect("stderr piped");
    let t0 = Instant::now();
    let mut acc = String::new();
    loop {
        let mut chunk = [0u8; 512];
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break, // EOF or reset: nothing more will come
            Ok(n) => acc.push_str(&String::from_utf8_lossy(&chunk[..n])),
        }
        if acc.contains("listening on http://") {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(15),
            "no ready line within 15 s while the 30 s blackhole negotiate \
             is pending — the line is at its old post-negotiate position; \
             stderr so far: {acc}"
        );
    }
    let at = t0.elapsed();
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        acc.contains("listening on http://"),
        "ready line present: {acc}"
    );
    assert!(
        at < Duration::from_secs(3),
        "ready line came at {at:?} — must precede the negotiate (30 s)"
    );
}

use serde_json::{Value, json};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        // Dead-IRIS tool calls must answer; a hang must fail THIS test,
        // never spin forever.
        .timeout(Duration::from_secs(25))
        .build()
        .expect("client")
}

async fn init_session(url: &str) -> Option<String> {
    let resp = client()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "stateless-door-test", "version": "0"},
                "capabilities": {}
            }
        }))
        .send()
        .await
        .expect("initialize must be accepted by the factory path");
    assert_eq!(resp.status(), 200, "initialize answered");
    let sid = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp.text().await.expect("init body");
    assert!(
        body.contains("\"protocolVersion\""),
        "initialize result present: {body}"
    );
    sid
}

/// POST one JSON-RPC frame into a session; returns (status, first payload).
async fn post(url: &str, sid: &str, body: Value) -> (u16, String) {
    let resp = client()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", sid)
        .json(&body)
        .send()
        .await
        .expect("request answered");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("body");
    // SSE envelope: first non-empty `data:` line carries the frame.
    let payload = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .find(|d| !d.is_empty() && d.starts_with('{'))
        .unwrap_or(text.trim())
        .to_string();
    (status, payload)
}

#[tokio::test]
async fn http_door_factory_sessions_and_error_path() {
    // Dead, unroutable-but-instant-refused URL: IrisClient::new validates
    // fine, negotiate is best-effort (http.rs:196 — errors are swallowed).
    let settings = rism::settings::Settings::default().with_base_url("http://127.0.0.1:1");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let url = format!("http://{addr}/mcp");
    let serve = tokio::spawn(rism::mcp::http::serve_http_on(listener, settings, false));

    // TWO independent sessions: the factory runs per session; a shared
    // broken clone would fail the second one.
    for round in 0..2 {
        let sid = init_session(&url)
            .await
            .unwrap_or_else(|| panic!("round {round}: stateful door must issue Mcp-Session-Id"));
        let (status, _) = post(
            &url,
            &sid,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await;
        assert_eq!(status, 202, "initialized notification accepted");

        let (_, payload) = post(
            &url,
            &sid,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .await;
        let n = payload.matches("\"inputSchema\"").count();
        assert_eq!(n, 25, "round {round}: the HTTP door serves 25 tools");

        // Error-path rule: a tools/call against the dead URL must ANSWER a
        // tool-level error (isError:true) — the reqwest timeout above is
        // the backstop; a hang fails THIS test loudly instead of hanging CI.
        let (_, payload) = post(
            &url,
            &sid,
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "execute_sql", "arguments": {"query": "select 1 as ok"}}
            }),
        )
        .await;
        assert!(
            payload.contains("\"isError\":true") && payload.contains("\"id\":3"),
            "dead-IRIS call must answer with a tool-level error, got: {payload}"
        );

        // Row-D DELETE = session teardown (202 Accepted), and the id must
        // be DEAD afterwards: a follow-up POST answers 404 (spec MUST for
        // terminated sessions) — asserted, not just commented.
        let del = client()
            .delete(&url)
            .header("Mcp-Session-Id", &sid)
            .send()
            .await
            .expect("delete answered");
        assert_eq!(del.status(), 202, "DELETE tears the session down");
        let (status, _) = post(
            &url,
            &sid,
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}),
        )
        .await;
        assert_eq!(status, 404, "deleted session id must be dead");
    }

    // Shutdown path: cancelling the door's token exits serve (no zombie).
    serve.abort();
    let _ = tokio::time::timeout(Duration::from_secs(5), serve).await;
    // Rebinding the same port after serve died proves no listener leaked.
    let rebind = tokio::net::TcpListener::bind(addr)
        .await
        .expect("same port rebinds after shutdown");
    drop(rebind);
}
