//! Live HTTP-door contract (SPEC T1, matrix row D): spawn the ACTUAL
//! binary `rism mcp --transport http --port 0`, learn the real port from
//! the stderr ready line, drive initialize → session header → tools/list
//! (25) → tools/call `execute_sql` against real IRIS, then kill and prove the
//! listener died. `#[ignore]`: CI live-smoke (docker IRIS) + VM row D.
//!
//! stderr stays OPEN and DRAINED on a thread for the child's whole life —
//! the stdio harness lesson: a pipe nobody drains (or whose reader was
//! dropped) deadlocks/resets a chatty server. VM row D redirects stderr to
//! a file, which satisfies the same contract.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value
)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Log(Arc<Mutex<String>>);

impl Log {
    fn text(&self) -> String {
        self.0.lock().expect("lock").clone()
    }
}

fn post_req(
    client: &reqwest::Client,
    url: &str,
    sid: Option<&str>,
    body: Value,
) -> reqwest::RequestBuilder {
    let mut req = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(s) = sid {
        req = req.header("Mcp-Session-Id", s);
    }
    req.json(&body)
}

/// SSE envelope: first non-empty `{`-payload.
fn payload_of(text: &str) -> String {
    text.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .find(|d| !d.is_empty() && d.starts_with('{'))
        .unwrap_or(text.trim())
        .to_string()
}

async fn post(
    client: &reqwest::Client,
    url: &str,
    sid: Option<&str>,
    body: Value,
    log: &Log,
) -> (u16, String) {
    let resp = post_req(client, url, sid, body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("request answered: {e}\nserver stderr:\n{}", log.text()));
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("body");
    (status, payload_of(&text))
}

#[tokio::test]
#[ignore = "requires the built binary + live IRIS (CI live-smoke / VM row D)"]
#[allow(clippy::too_many_lines)] // one linear wire protocol walkthrough
async fn http_door_live_binary_roundtrip() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rism"))
        .args(["mcp", "--transport", "http", "--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rism mcp --transport http");
    let stderr = child.stderr.take().expect("stderr piped");
    let sink = Arc::new(Mutex::new(String::new()));
    {
        let sink = sink.clone();
        std::thread::spawn(move || {
            let mut reader = stderr;
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => sink
                        .lock()
                        .expect("lock")
                        .push_str(&String::from_utf8_lossy(&chunk[..n])),
                }
            }
        });
    }
    let log = Log(sink.clone());
    let start = Instant::now();
    let url = loop {
        if let Some(rest) = log.text().split("listening on ").nth(1)
            && let Some(u) = rest.split_whitespace().next()
            && u.ends_with("/mcp")
        {
            break u.to_string();
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "no ready line: {}",
            log.text()
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "loopback ready URL: {url}"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        // A pooled connection can be closed server-side between turns;
        // one fresh connection per POST (what real MCP clients do).
        .pool_max_idle_per_host(0)
        .build()
        .expect("client");

    let resp = post_req(
        &client,
        &url,
        None,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "live-door-test", "version": "0"},
                "capabilities": {}
            }
        }),
    )
    .send()
    .await
    .expect("initialize answered");
    assert_eq!(resp.status(), 200, "initialize accepted");
    let sid = resp
        .headers()
        .get("mcp-session-id")
        .expect("stateful door issues Mcp-Session-Id")
        .to_str()
        .expect("ascii id")
        .to_string();
    assert!(
        payload_of(&resp.text().await.expect("body")).contains("\"protocolVersion\""),
        "initialize result"
    );

    let (status, _) = post(
        &client,
        &url,
        Some(&sid),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        &log,
    )
    .await;
    assert_eq!(status, 202);

    let (_, payload) = post(
        &client,
        &url,
        Some(&sid),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        &log,
    )
    .await;
    assert_eq!(payload.matches("\"inputSchema\"").count(), 25, "25 tools");

    let (_, payload) = post(
        &client,
        &url,
        Some(&sid),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "execute_sql", "arguments": {"query": "select 1 as ok"}}
        }),
        &log,
    )
    .await;
    assert!(
        payload.contains("\"isError\":false") && payload.contains("ok"),
        "execute_sql over the HTTP door: {payload}"
    );

    // stdout purity: the HTTP door answers over HTTP; process stdout EMPTY.
    let _ = child.kill();
    let out = child.wait_with_output().expect("reaped");
    assert!(out.stdout.is_empty(), "HTTP door must never write stdout");

    // No leftover listener: the same URL now refuses connections.
    let probe = client.get(&url).send().await;
    assert!(probe.is_err(), "killed server must free the port");
}
