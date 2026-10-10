//! HTTP door status matrix (P-05b/P-05c + F-01 rows): every 4xx family the
//! R1 review measured is now PINNED by a test, plus the Host-guard toggle
//! (live on a loopback bind, disabled by --allow-all-interfaces) and the
//! Origin enforcement rows. Raw tokio sockets for the header overrides
//! (Host/Origin must reach the wire exactly as written; client stacks
//! normalize) and the shipped `serve_http_on` wiring for everything else.
//! No IRIS needed: `base_url` is a refused port, so negotiate fails fast and
//! every answer below is header-logic, not tool logic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value
)]

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","clientInfo":{"name":"matrix","version":"0"},"capabilities":{}}}"#;
const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
const ACCEPT_BOTH: &str = "application/json, text/event-stream";

/// One raw HTTP/1.1 exchange: write `request`, read until the server closes
/// (every request sets Connection: close or gets one small response), and
/// return the status code from the first line.
async fn raw_status(port: u16, request: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect door");
    // A huge body can hit EPIPE once the server 413s and drops the
    // connection mid-write; the response itself still arrives.
    let _ = stream.write_all(request.as_bytes()).await;
    let mut acc = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut acc)).await;
    let head = String::from_utf8_lossy(&acc).to_string();
    let line = head.lines().next().unwrap_or_default().to_string();
    line.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no HTTP status in response head: {line:?}"))
}

/// A POST frame; `host` overrides the Host header, `origin` adds Origin.
fn post(
    body: &str,
    accept: &str,
    ct: &str,
    sid: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
) -> String {
    format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: {ct}\r\nAccept: {accept}\r\n\
         Connection: close\r\nContent-Length: {}\r\n{}{}\r\n{body}",
        host.unwrap_or("127.0.0.1"),
        body.len(),
        sid.map(|s| format!("Mcp-Session-Id: {s}\r\n"))
            .unwrap_or_default(),
        origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default(),
    )
}

fn get(accept: &str, sid: Option<&str>) -> String {
    format!(
        "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAccept: {accept}\r\n{}\r\n",
        sid.map(|s| format!("Mcp-Session-Id: {s}\r\n"))
            .unwrap_or_default()
    )
}

async fn spawn_door(all_interfaces: bool) -> (u16, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let settings = rism::settings::Settings::default().with_base_url("http://127.0.0.1:1");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let serve = tokio::spawn(rism::mcp::http::serve_http_on(
        listener,
        settings,
        all_interfaces,
    ));
    for _ in 0..80 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    (port, serve)
}

/// initialize without any Origin/session header; returns the session id
/// (this IS F-01 row (b): an absent-Origin native client must get 200).
async fn initialize(port: u16) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    stream
        .write_all(post(INIT, ACCEPT_BOTH, "application/json", None, None, None).as_bytes())
        .await
        .expect("write");
    let mut acc = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut acc)).await;
    let head = String::from_utf8_lossy(&acc).to_lowercase();
    assert!(
        head.contains("200 ok"),
        "initialize must answer 200:\n{head}"
    );
    head.lines()
        .find_map(|l| {
            l.strip_prefix("mcp-session-id:")
                .map(|v| v.trim().to_string())
        })
        .expect("stateful door issues Mcp-Session-Id")
}

async fn stop(serve: tokio::task::JoinHandle<anyhow::Result<()>>) {
    serve.abort();
    let _ = tokio::time::timeout(Duration::from_secs(5), serve).await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // the matrix is one linear pin list
async fn status_matrix_is_pinned() {
    let (port, serve) = spawn_door(false).await;
    let sid = initialize(port).await;

    // -- 406: POST must accept BOTH json and event-stream -----------------
    assert_eq!(
        raw_status(
            port,
            &post(
                LIST,
                "application/json",
                "application/json",
                Some(&sid),
                None,
                None
            )
        )
        .await,
        406,
        "POST accepting only application/json"
    );
    assert_eq!(
        raw_status(port, &get("application/json", None)).await,
        406,
        "GET no-sid with a json-only Accept"
    );
    // -- 400: session id required (GET + DELETE legs) ---------------------
    assert_eq!(
        raw_status(port, &get("text/event-stream", None)).await,
        400,
        "GET without a session id"
    );
    assert_eq!(
        raw_status(
            port,
            "DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .await,
        400,
        "DELETE without a session id"
    );
    // -- 404: unknown session id (terminated-session contract) ------------
    assert_eq!(
        raw_status(
            port,
            &post(
                LIST,
                ACCEPT_BOTH,
                "application/json",
                Some("dead-beef-0000"),
                None,
                None
            )
        )
        .await,
        404,
        "POST on an unknown session id"
    );
    // -- 405: PUT rides the method-match arm ------------------------------
    assert_eq!(
        raw_status(
            port,
            "PUT /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .await,
        405,
        "PUT is not a door method"
    );
    // -- 415: wrong Content-Type -------------------------------------------
    assert_eq!(
        raw_status(
            port,
            &post(LIST, ACCEPT_BOTH, "text/plain", Some(&sid), None, None)
        )
        .await,
        415,
        "POST with text/plain"
    );
    // -- 413: body over rmcp's 4 MiB cap (DEFAULT_MAX_REQUEST_BODY_BYTES) --
    let pad = "A".repeat(4 * 1024 * 1024 + 1024);
    let big = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-03-26\",\"clientInfo\":{{\"name\":\"{pad}\",\"version\":\"0\"}},\"capabilities\":{{}}}}}}"
    );
    assert_eq!(
        raw_status(
            port,
            &post(&big, ACCEPT_BOTH, "application/json", None, None, None)
        )
        .await,
        413,
        ">4 MiB body must be refused"
    );
    // -- F-01 (a): cross-origin browser request => 403 ---------------------
    assert_eq!(
        raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some("https://evil.test"),
                None
            )
        )
        .await,
        403,
        "evil Origin must be refused (M1 pin: reverting the F-01 chain flips this to 200)"
    );
    // -- F-01 (c): same-origin => 200 (M2 pin: emptying the list flips it) -
    let same_origin = format!("http://127.0.0.1:{port}");
    assert_eq!(
        raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(&same_origin),
                None
            )
        )
        .await,
        200,
        "same-origin POST must pass"
    );

    stop(serve).await;
}

/// R2 adversarial lane: the full Origin bypass list the attack matrix ran
/// live (testing-notes/R2_ATTACKS.md), pinned against the shipped loopback
/// posture. Every browser-reachable cross-origin shape must 403; the two
/// documented 200 quirks are upstream `http`-crate authority parsing
/// (userinfo, first-value-wins duplicates) and carry NO browser vector —
/// the Fetch spec never sends a portless default-port Origin and never
/// lets a page compose an Origin at all, and the Host guard answers the
/// rebinding shape the userinfo string would otherwise ride in on.
#[tokio::test]
#[allow(clippy::too_many_lines)] // the bypass matrix is one linear pin list
async fn origin_bypass_matrix_denies_every_browser_shape() {
    let (port, serve) = spawn_door(false).await;
    let p = port;
    let same = format!("http://127.0.0.1:{p}");

    // (A) browser-reachable / hostile cross-origin shapes => 403 ----------
    for (label, origin) in [
        ("evil https", "https://evil.test"),
        ("browser-null", "null"),
        ("NULL uppercase", "NULL"),
        ("undefined", "undefined"),
        ("empty value", ""),
        ("file scheme", "file:///"),
        ("chrome-extension scheme", "chrome-extension://abcdefghij"),
        ("schemeless host", "127.0.0.1:1"),
        ("v6 without brackets", "http://::1:1"),
        ("unicode homoglyph host", "http://lοcalhost:1"),
        ("trailing-dot host", "http://127.0.0.1.:1"),
    ] {
        let code = raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(origin),
                None,
            ),
        )
        .await;
        assert_eq!(
            code, 403,
            "{label} Origin shape must be refused, got {code}"
        );
    }
    // port-strictness: the allowlist carries explicit ports, so a stray
    // page on ANY other localhost port (or the portless default) is out.
    for (label, origin) in [
        ("portless loopback", "http://127.0.0.1"),
        ("wrong port", &format!("http://127.0.0.1:{}", p + 1)),
        ("https on loopback", &format!("https://127.0.0.1:{p}")),
    ] {
        let code = raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(origin),
                None,
            ),
        )
        .await;
        assert_eq!(code, 403, "{label} must be refused, got {code}");
    }

    // (B) same-origin under every spelling a browser can actually produce => 200
    for (label, origin) in [
        ("IPv4 literal", same.as_str()),
        ("localhost name", &format!("http://localhost:{p}")),
        ("host case-insensitive", &format!("http://LOCALHOST:{p}")),
        ("scheme case-insensitive", &format!("HTTP://127.0.0.1:{p}")),
        ("IPv6 bracketed", &format!("http://[::1]:{p}")),
        ("trailing slash", &format!("{same}/")),
        ("path appended", &format!("{same}/steal")),
    ] {
        let code = raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(origin),
                None,
            ),
        )
        .await;
        assert_eq!(code, 200, "same-origin via {label} must pass, got {code}");
    }

    // (C) enforcement runs BEFORE routing (tower.rs check order): a hostile
    // Origin on any method leg gets the same 403, never the 400/404/405 the
    // leg would earn header-less.
    let evil = "https://evil.test";
    assert_eq!(
        raw_status(
            port,
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
             Accept: text/event-stream\r\nOrigin: https://evil.test\r\n\r\n"
        )
        .await,
        403,
        "evil Origin on the GET-stream leg must deny before the session-id check"
    );
    assert_eq!(
        raw_status(
            port,
            "DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
             Origin: https://evil.test\r\n\r\n"
        )
        .await,
        403,
        "evil Origin on the DELETE leg must deny before the session-id check"
    );
    assert_eq!(
        raw_status(
            port,
            "PUT /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\
             Connection: close\r\nOrigin: https://evil.test\r\n\r\n"
        )
        .await,
        403,
        "evil Origin on a non-door method must deny before the method-match arm"
    );
    // (D) header-layer shapes: the two documented upstream quirks.
    // Duplicate Origin headers: rmcp reads `headers.get(ORIGIN)` = the
    // FIRST value only; a second hostile Origin on the wire is invisible to
    // the check, so an allowlisted FIRST value passes (measured 200). No
    // browser can produce this (the Fetch spec owns the single Origin
    // slot), so it is a documented accepted shape, pinned as 200 so a
    // future rmcp that AND-mingles duplicates shows up here as a flip.
    let dup = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: {ACCEPT_BOTH}\r\nConnection: close\r\nContent-Length: {}\r\n\
         Origin: {same}\r\nOrigin: {evil}\r\n\r\n{INIT}",
        INIT.len()
    );
    assert_eq!(
        raw_status(port, &dup).await,
        200,
        "duplicate Origin (allowlisted first) — upstream first-value-wins, documented"
    );
    // Same with the hostile value FIRST: it decides (403) — the order in
    // which duplicates are consulted must never let a later allowed value
    // rescue a hostile first one.
    let dup_evil_first = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Accept: {ACCEPT_BOTH}\r\nConnection: close\r\nContent-Length: {}\r\n\
         Origin: {evil}\r\nOrigin: {same}\r\n\r\n{INIT}",
        INIT.len()
    );
    assert_eq!(
        raw_status(port, &dup_evil_first).await,
        403,
        "duplicate Origin (hostile first) must 403"
    );
    // Origin with userinfo `http://evil@127.0.0.1:PORT`: the `http` crate
    // strips userinfo before host() ⇒ host == 127.0.0.1 ⇒ accepted (measured
    // 200). A browser never sends a userinfo Origin; the Host guard covers
    // the rebinding shape. Pinned as 200 so the semantics are on record.
    let userinfo = format!("http://evil@127.0.0.1:{p}");
    assert_eq!(
        raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(&userinfo),
                None
            )
        )
        .await,
        200,
        "userinfo Origin — upstream authority parsing strips userinfo; documented, no browser vector"
    );

    // (E) Host x Origin combos on the DEFAULT door: Host is checked first
    // (DNS-rebinding), a foreign Host denies even with an allowlisted
    // Origin; a good Host + hostile Origin denies on Origin.
    assert_eq!(
        raw_status(
            port,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some(&same),
                Some("evil.example")
            )
        )
        .await,
        403,
        "foreign Host + same-origin must still deny (Host guard first)"
    );

    // (F) header-NAME case: HTTP field names are case-insensitive (RFC
    // 9110 §5.1) and hyper's HeaderMap is too — `origin:`/`ORIGIN:` must
    // deny exactly like `Origin:`.
    for name in ["Origin", "ORIGIN", "origin", "origIN"] {
        let frame = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Accept: {ACCEPT_BOTH}\r\nConnection: close\r\nContent-Length: {}\r\n\
             {name}: {evil}\r\n\r\n{INIT}",
            INIT.len()
        );
        assert_eq!(
            raw_status(port, &frame).await,
            403,
            "hostile value under header name {name:?} must deny (case-insensitive field name)"
        );
    }
    stop(serve).await;
}

#[tokio::test]
async fn host_guard_and_origin_posture_toggle_with_bind() {
    // Guard ON (default loopback door): a foreign Host is a DNS-rebinding
    // shape => 403. Mutation M5 (deleting the disable_allowed_hosts branch)
    // must NOT be able to flip the allow-all leg below to 403.
    let (port_on, serve_on) = spawn_door(false).await;
    assert_eq!(
        raw_status(
            port_on,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                None,
                Some("evil.example")
            )
        )
        .await,
        403,
        "loopback door must enforce the Host guard"
    );
    stop(serve_on).await;

    // Guard OFF (--allow-all-interfaces posture): the bind it would guard
    // is already public, so any Host answers — but Origin enforcement
    // REMAINS active (empty list + the flag deny every PRESENT Origin;
    // absent-Origin clients passed at row (b) above on BOTH doors).
    let (port_off, serve_off) = spawn_door(true).await;
    assert_eq!(
        raw_status(
            port_off,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                None,
                Some("evil.example")
            )
        )
        .await,
        200,
        "allow-all door lets any Host through (documented opt-out)"
    );
    assert_eq!(
        raw_status(
            port_off,
            &post(
                INIT,
                ACCEPT_BOTH,
                "application/json",
                None,
                Some("https://evil.test"),
                None
            )
        )
        .await,
        403,
        "allow-all door still validates Origin when present"
    );
    stop(serve_off).await;
}
