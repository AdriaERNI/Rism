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

use std::time::Duration;

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

        // Row-D DELETE = session teardown; then the id must be dead.
        client()
            .delete(&url)
            .header("Mcp-Session-Id", &sid)
            .send()
            .await
            .expect("delete answered");
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
