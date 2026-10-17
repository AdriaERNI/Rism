//! DBGP (`XDebug`) protocol client over the Atelier WebSocket.
//!
//! Endpoint: `ws(s)://host/api/atelier/vN/%25SYS/debug` (namespace MUST be
//! `%SYS` on the wire — a `/USER/debug` path never sends init, verified live).
//! Auth: `Authorization: Basic` header — no session cookies needed (§probe).
//! Wire format: newline-terminated commands
//! `cmd -i TXID [-arg value ...] [-- base64data]`; replies come back as
//! `length|base64(xml)` frames.
//!
//! Two IRIS quirks are encoded in the command layer (root-caused by reading
//! the %Atelier.v1.XDebugAgent source live):
//! 1. `context_get` crashes the agent (`<exit>` + closed socket) unless a
//!    `stack_get` ran since the last stop — it fills `StackLevelMappings`.
//! 2. Property rendering needs `feature_set max_data` first — it fills the
//!    agent's `Features` array. See [`crate::tools::debugger`] for the order.

use std::fmt::Write as _;
use std::time::Duration;

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use xmltree::Element;

use crate::error::{Error, Result};
use crate::iris::http::IrisClient;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One live DBGP connection (a debug session's transport).
pub struct DbgpConnection {
    ws: Ws,
    tx_id: u32,
}

impl DbgpConnection {
    /// Open the debug WebSocket and read the `<init>` packet.
    ///
    /// Retries connection-level failures with increasing backoff (4 attempts):
    /// IRIS may reset the socket while releasing a prior `XDebug` agent
    /// (Prism's `attach_session` parity — verified: orphaned debug jobs hold the
    /// slot and the first connect dies mid-handshake).
    ///
    /// # Errors
    /// [`Error::Debug`] on connect/timeout/transport failures.
    pub async fn connect(client: &IrisClient, timeout: Duration) -> Result<Self> {
        let mut last: Option<Error> = None;
        for attempt in 0..4u64 {
            match Self::connect_once(client, timeout).await {
                Ok(c) => return Ok(c),
                Err(e @ Error::Dbgp { .. }) => return Err(e),
                Err(e) => {
                    last = Some(e);
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(1250 * (attempt + 1))).await;
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| Error::Debug("debug connect failed".to_string())))
    }

    async fn connect_once(client: &IrisClient, timeout: Duration) -> Result<Self> {
        let st = client.settings();
        let url = debug_ws_url(st.iris_base_url.trim_end_matches('/'), &client.api_prefix());

        let creds = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", st.iris_username, st.iris_password));

        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::Debug(format!("bad ws url: {e}")))?;
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Basic {creds}"))
                .map_err(|_| Error::Debug("bad auth header".to_string()))?,
        );

        let (mut ws, _resp) =
            tokio::time::timeout(timeout, tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| Error::Debug("debug ws connect timed out".to_string()))?
                .map_err(|e| Error::Debug(format!("debug ws connect: {e}")))?;

        // The agent introduces itself before accepting commands.
        let init = recv_frame(&mut ws, timeout).await?;
        if !init.contains("<init") {
            return Err(Error::Debug(format!("expected <init>, got: {init}")));
        }
        Ok(Self { ws, tx_id: 0 })
    }

    /// Send one command and parse the XML reply.
    ///
    /// `args` are rendered as `-key value`; `data` is appended as
    /// `-- base64(data_bytes)` (DBGP base64 data payload).
    ///
    /// # Errors
    /// [`Error::Dbgp`] on `<error>` replies (code 201 "Breakpoint Cannot Be
    /// Mapped", 6709 "`Target not stopped`", …); [`Error::Debug`] on transport
    /// or agent-crash (`<exit>`) frames.
    pub async fn command(
        &mut self,
        name: &str,
        args: &[(&str, String)],
        data: Option<&[u8]>,
    ) -> Result<Element> {
        // The DBGP command line is space-separated and newline-terminated:
        // a command name or raw arg carrying whitespace or a control
        // character is either corrupt or a command injection into the
        // agent's parser (a `\n` inside a breakpoint method name made the
        // agent execute the tail as a SECOND command — verified live).
        // Refuse before anything goes on the socket. Values that are
        // legitimately free-form (targets, expressions, conditions) travel
        // base64 in `data`, never as raw args.
        let wire_safe =
            |s: &str| !s.is_empty() && s.chars().all(|c| !c.is_whitespace() && !c.is_control());
        if !wire_safe(name) || args.iter().any(|(k, v)| !wire_safe(k) || !wire_safe(v)) {
            return Err(Error::Config(format!(
                "invalid DBGP command or argument (wire guard): {name}"
            )));
        }
        self.tx_id += 1;
        let mut line = String::new();
        let _ = write!(line, "{name} -i {}", self.tx_id);
        for (k, v) in args {
            let _ = write!(line, " -{k} {v}");
        }
        if let Some(bytes) = data {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let _ = write!(line, " -- {b64}");
        }
        // The agent frames commands by newline — without it nothing replies.
        line.push('\n');

        self.ws
            .send(Message::Text(line.into()))
            .await
            .map_err(|e| Error::Debug(format!("ws send: {e}")))?;

        // Route by transaction id: a stale reply from an earlier command
        // whose recv timed out (a `run` that answered late, e.g.) can still
        // be queued here. Frames that do not carry our id are drained
        // inside the same 30 s budget; frames without a parseable id are
        // the agent's own and pass through untouched (parity with the
        // pre-routing behavior).
        let expect = self.tx_id;
        let started = std::time::Instant::now();
        let budget = Duration::from_secs(30);
        let root = loop {
            let left = budget.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(Error::Debug("dbgp recv timed out".to_string()));
            }
            let raw = recv_frame(&mut self.ws, left).await?;
            if raw.starts_with("<exit")
                || raw.contains("&lt;EXIT&gt;")
                || raw.contains("<UNDEFINED>")
            {
                return Err(Error::Debug(format!(
                    "XDebug agent crashed on '{name}': {}",
                    raw.chars().take(200).collect::<String>()
                )));
            }
            let root = Element::parse(std::io::Cursor::new(raw.as_bytes()))
                .map_err(|e| Error::Debug(format!("bad DBGP xml for '{name}': {e}")))?;
            if matches!(attr(&root, "transaction_id").and_then(|t| t.parse::<u32>().ok()), Some(tx) if tx != expect)
            {
                continue; // stale reply of an earlier command — drain it
            }
            break root;
        };

        // <error code='N'><message>…</message></error> or <response><error…>
        if let Some(err) = find_error(&root) {
            let code: u32 = attr(err, "code").and_then(|c| c.parse().ok()).unwrap_or(0);
            let message = elements(err)
                .find(|c| c.name == "message")
                .and_then(text_of)
                .unwrap_or_else(|| "unknown DBGP error".to_string());
            return Err(Error::Dbgp { code, message });
        }
        Ok(root)
    }

    /// Close the debug socket (frees the agent for the next session).
    pub async fn close(&mut self) {
        let _ = self.ws.close(None).await;
    }

    /// Test seam: a connection over an already-connected TCP socket.
    /// Lets unit tests drive `run_and_drain`'s frame routing with a
    /// loopback peer that scripts replies (a queued stale frame, then the
    /// matching one) — no IRIS, no debugger slot.
    #[cfg(test)]
    async fn for_test(stream: tokio::net::TcpStream) -> Self {
        let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            tokio_tungstenite::MaybeTlsStream::Plain(stream),
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        Self { ws, tx_id: 0 }
    }

    /// Wait (without sending anything) for the reply to the LAST command
    /// this connection issued, up to `timeout`. When a `run` answer did not
    /// arrive inside its window the target is still executing and its frame
    /// will surface at the NEXT stop (or at the job's end); re-sending
    /// `run` then answers `998/6709 Target not stopped` (verified live), so
    /// the correct move is to drain for the pending id — never to `stop`,
    /// which would ABORT the running job. Frames under other ids are
    /// stale replies and get discarded; a lapped window returns `None`
    /// (still running), mirroring [`Self::run_and_drain`].
    ///
    /// # Errors
    /// [`Error`] on transport failures or a DBGP `<error>` under our id.
    pub async fn await_reply(&mut self, timeout: Duration) -> Result<Option<Element>> {
        let expect = self.tx_id;
        let started = std::time::Instant::now();
        loop {
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(None);
            }
            let raw = match recv_frame(&mut self.ws, left).await {
                Ok(r) => r,
                // A recv timeout is the expected "job still running" case.
                Err(Error::Debug(ref m)) if m.contains("timed out") => return Ok(None),
                Err(e) => return Err(e),
            };
            if raw.starts_with("<exit")
                || raw.contains("&lt;EXIT&gt;")
                || raw.contains("<UNDEFINED>")
            {
                return Err(Error::Debug(format!(
                    "XDebug agent crashed awaiting a reply: {}",
                    raw.chars().take(200).collect::<String>()
                )));
            }
            let Ok(root) = Element::parse(std::io::Cursor::new(raw.as_bytes())) else {
                continue; // not XML we can route — drop it
            };
            let Some(tx) = attr(&root, "transaction_id").and_then(|t| t.parse::<u32>().ok()) else {
                continue;
            };
            if tx != expect {
                continue; // stale reply of an earlier command
            }
            if let Some(err) = find_error(&root) {
                let code: u32 = attr(err, "code").and_then(|c| c.parse().ok()).unwrap_or(0);
                let message = elements(err)
                    .find(|c| c.name == "message")
                    .and_then(text_of)
                    .unwrap_or_else(|| "unknown DBGP error".to_string());
                return Err(Error::Dbgp { code, message });
            }
            return Ok(Some(root));
        }
    }

    /// Send `run` and wait for a reply whose `transaction_id` matches our
    /// command, discarding earlier queued frames. A resumed job replies
    /// only when it NEXT stops; any stale reply from a previously timed-out
    /// command arrives FIRST, and a socket read that finds nothing simply
    /// times out. Returns `None` when no matching reply arrived in time
    /// (target still running), `Some(Err)` on a real DBGP error frame.
    ///
    /// # Errors
    /// [`Error`] on transport failures or a DBGP `<error>` under our id.
    pub async fn run_and_drain(&mut self, timeout: Duration) -> Result<Option<Element>> {
        self.tx_id += 1;
        let expect = self.tx_id;
        let mut line = format!("run -i {expect}");
        line.push('\n');
        self.ws
            .send(Message::Text(line.into()))
            .await
            .map_err(|e| Error::Debug(format!("ws send: {e}")))?;
        let started = std::time::Instant::now();
        loop {
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(None);
            }
            let raw = match recv_frame(&mut self.ws, left).await {
                Ok(r) => r,
                // A recv timeout is the expected "job still running" case.
                Err(Error::Debug(ref m)) if m.contains("timed out") => return Ok(None),
                Err(e) => return Err(e),
            };
            if raw.starts_with("<exit")
                || raw.contains("&lt;EXIT&gt;")
                || raw.contains("<UNDEFINED>")
            {
                return Err(Error::Debug(format!(
                    "XDebug agent crashed on 'run': {}",
                    raw.chars().take(200).collect::<String>()
                )));
            }
            let Ok(root) = Element::parse(std::io::Cursor::new(raw.as_bytes())) else {
                continue; // not XML we can route — drop it
            };
            let Some(tx) = attr(&root, "transaction_id").and_then(|t| t.parse::<u32>().ok()) else {
                continue;
            };
            if tx != expect {
                continue; // stale reply of an earlier command
            }
            if let Some(err) = find_error(&root) {
                let code: u32 = attr(err, "code").and_then(|c| c.parse().ok()).unwrap_or(0);
                let message = elements(err)
                    .find(|c| c.name == "message")
                    .and_then(text_of)
                    .unwrap_or_else(|| "unknown DBGP error".to_string());
                return Err(Error::Dbgp { code, message });
            }
            return Ok(Some(root));
        }
    }
}

/// WS URL for the debug endpoint — always routed under `%25SYS`.
fn debug_ws_url(base: &str, prefix: &str) -> String {
    let base = base.trim_end_matches('/');
    let (scheme, rest) = match base.split_once("://") {
        Some(("https", r)) => ("wss", r),
        Some(("http", r)) => ("ws", r),
        _ => ("ws", base),
    };
    format!("{scheme}://{rest}{prefix}/%25SYS/debug")
}

async fn recv_frame(ws: &mut Ws, timeout: Duration) -> Result<String> {
    loop {
        let msg = tokio::time::timeout(timeout, ws.next())
            .await
            .map_err(|_| Error::Debug("dbgp recv timed out".to_string()))?
            .ok_or_else(|| Error::Debug("dbgp socket closed by server".to_string()))?
            .map_err(|e| Error::Debug(format!("dbgp recv: {e}")))?;
        match msg {
            Message::Text(t) => return Ok(unframe(&t)),
            Message::Binary(b) => return Ok(unframe(&String::from_utf8_lossy(&b))),
            Message::Close(_) => return Err(Error::Debug("dbgp socket closed".to_string())),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

/// `length|base64(xml)` → `xml` (plain XML passes through untouched).
fn unframe(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some((_, payload)) = trimmed.split_once('|') {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload) {
            // ISO-8859-1 per the DBGP spec's typical encoding.
            return bytes.iter().map(|b| *b as char).collect();
        }
    }
    trimmed.to_string()
}

/// Attribute of an element as `&str` (empty-avoiding).
pub fn attr<'a>(elem: &'a Element, key: &str) -> Option<&'a str> {
    elem.attributes.get(key).map(String::as_str)
}

/// First non-empty text child, trimmed.
#[must_use]
pub fn text_of(elem: &Element) -> Option<String> {
    elem.children.iter().find_map(|n| match n {
        xmltree::XMLNode::Text(t) => {
            let t = t.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        _ => None,
    })
}

/// Child element iterator (xmltree wraps children in `XMLNode`).
pub fn elements(elem: &Element) -> impl Iterator<Item = &Element> {
    elem.children
        .iter()
        .filter_map(xmltree::XMLNode::as_element)
}

fn find_error(node: &Element) -> Option<&Element> {
    if node.name == "error" {
        return Some(node);
    }
    elements(node).find_map(find_error)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn unframe_decodes_length_pipe_base64() {
        let xml = "<response status='break'/>";
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml);
        assert_eq!(unframe(&format!("{}|{b64}", xml.len())), xml);
        assert_eq!(unframe(xml), xml);
    }

    #[test]
    fn ws_url_always_under_sys() {
        assert_eq!(
            debug_ws_url("https://h:443", "/api/atelier/v1"),
            "wss://h:443/api/atelier/v1/%25SYS/debug"
        );
        assert_eq!(
            debug_ws_url("http://h:52773/", "/api/atelier/v1"),
            // trailing slash trimmed by caller; double slash tolerated by WS
            "ws://h:52773/api/atelier/v1/%25SYS/debug"
        );
    }

    #[test]
    fn error_element_detected() {
        let doc = Element::parse(std::io::Cursor::new(
            "<response command='breakpoint_set' transaction_id='6'><error code='201'><message>Breakpoint Cannot Be Mapped</message></error></response>",
        ))
        .expect("parses");
        let err = find_error(&doc).expect("error child");
        assert_eq!(attr(err, "code"), Some("201"));
    }

    /// `run_and_drain` must route by transaction id: a stale reply queued
    /// from an earlier timed-out command arrives FIRST and must be
    /// discarded, never handed back as the `run` answer. Remove the
    /// `tx != expect` check (first frame wins) and this test goes red.
    #[tokio::test]
    async fn run_and_drain_discards_a_stale_frame_then_returns_the_match() {
        use futures_util::StreamExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::net::TcpStream::connect(addr)
            .await
            .expect("loopback connect");
        let (server, _) = listener.accept().await.expect("accept");
        let mut conn = DbgpConnection::for_test(client).await;
        let mut peer = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                server,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            ),
        )
        .await
        .expect("peer handshake");
        // Pretend tx 7 was an earlier command whose reply timed out — its
        // answer is still sitting on the socket when `run` goes out as tx 8.
        conn.tx_id = 7;
        let scripted = tokio::spawn(async move {
            let _cmd = peer.next().await; // the `run -i 8` line
            peer.send(Message::Text(
                "<response command='step_over' transaction_id='7' status='break'/>".into(),
            ))
            .await
            .expect("stale frame");
            peer.send(Message::Text(
                "<response command='run' transaction_id='8' status='break'><message/></response>"
                    .into(),
            ))
            .await
            .expect("fresh frame");
            peer
        });
        let root = conn
            .run_and_drain(Duration::from_millis(500))
            .await
            .expect("no error")
            .expect("the matching frame must be returned, not None");
        assert_eq!(attr(&root, "transaction_id"), Some("8"));
        assert_eq!(attr(&root, "command"), Some("run"));
        drop(scripted.await.expect("script task"));
    }

    /// Same routing inside plain `command`: a late reply queued from a
    /// timed-out `run` must not be mistaken for this command's answer.
    #[tokio::test]
    async fn command_discards_a_stale_frame_before_the_matching_reply() {
        use futures_util::StreamExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::net::TcpStream::connect(addr)
            .await
            .expect("loopback connect");
        let (server, _) = listener.accept().await.expect("accept");
        let mut conn = DbgpConnection::for_test(client).await;
        let mut peer = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                server,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            ),
        )
        .await
        .expect("peer handshake");
        conn.tx_id = 9;
        let scripted = tokio::spawn(async move {
            let _cmd = peer.next().await;
            peer.send(Message::Text(
                "<response command='run' transaction_id='8' status='break'/>".into(),
            ))
            .await
            .expect("stale frame");
            peer.send(Message::Text(
                "<response command='status' transaction_id='10' status='break'><message/></response>"
                    .into(),
            ))
            .await
            .expect("fresh frame");
            peer
        });
        let root = conn
            .command("status", &[], None)
            .await
            .expect("stale frame must be drained, not returned");
        assert_eq!(attr(&root, "transaction_id"), Some("10"));
        assert_eq!(attr(&root, "command"), Some("status"));
        drop(scripted.await.expect("script task"));
    }

    /// The reply to a lapsed `run` arrives AT the next stop, under the run's
    /// own transaction id: `await_reply` must find it (draining any stale
    /// frame queued before it). Accept-first-frame or drop-the-id-check and
    /// this test goes red.
    #[tokio::test]
    async fn await_reply_picks_up_the_late_run_frame_after_the_window() {
        use futures_util::StreamExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::net::TcpStream::connect(addr)
            .await
            .expect("loopback connect");
        let (server, _) = listener.accept().await.expect("accept");
        let mut conn = DbgpConnection::for_test(client).await;
        let mut peer = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                server,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            ),
        )
        .await
        .expect("peer handshake");
        conn.tx_id = 7;
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let scripted = tokio::spawn(async move {
            let _cmd = peer.next().await; // `run -i 8`
            // Stay silent while the step window lapses (the target keeps
            // running); THEN queue a stale frame of an older command plus
            // the run's own answer, delivered when the target stops.
            let _ = release_rx.await;
            peer.send(Message::Text(
                "<response command='step_over' transaction_id='6' status='break'/>".into(),
            ))
            .await
            .expect("stale frame");
            peer.send(Message::Text(
                "<response command='run' transaction_id='8' status='break'><message/></response>"
                    .into(),
            ))
            .await
            .expect("late run frame");
            peer
        });
        let lapped = conn
            .run_and_drain(Duration::from_millis(150))
            .await
            .expect("a lapped window is not an error");
        assert!(lapped.is_none(), "still running inside the window");
        release_tx.send(()).expect("release the scripted frames");
        let root = conn
            .await_reply(Duration::from_secs(2))
            .await
            .expect("transport ok")
            .expect("the late run frame must be routed to us");
        assert_eq!(attr(&root, "transaction_id"), Some("8"));
        assert_eq!(attr(&root, "command"), Some("run"));
        drop(scripted.await.expect("script task"));
    }

    /// A raw arg carrying whitespace/newline must refuse BEFORE a single
    /// byte goes on the socket (the agent parses `cmd -k v` space-split and
    /// newline-terminated — an injected `\n` executes as a second command,
    /// verified live). Remove the wire guard in `command` and the peer's
    /// read hangs, making this test red.
    #[tokio::test]
    async fn command_refuses_wire_hostile_args_before_sending() {
        use futures_util::StreamExt as _;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::net::TcpStream::connect(addr)
            .await
            .expect("loopback connect");
        let (server, _) = listener.accept().await.expect("accept");
        let mut peer = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                server,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            ),
        )
        .await
        .expect("peer handshake");
        let mut conn = DbgpConnection::for_test(client).await;
        for (name, args) in [
            ("breakpoint_set", vec![("m", "Go\nstatus".to_string())]),
            ("breakpoint_remove", vec![("d", "1\nrun".to_string())]),
            ("feature_set", vec![("n", "max data".to_string())]),
        ] {
            let pairs: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.clone())).collect();
            let res = conn.command(name, &pairs, None).await;
            assert!(
                matches!(res, Err(Error::Config(_))),
                "{name} with hostile raw arg must be a Config refusal, got {res:?}"
            );
        }
        // Nothing reached the socket: the peer's first read must time out.
        assert!(
            tokio::time::timeout(Duration::from_millis(150), peer.next())
                .await
                .is_err(),
            "no command line may be sent for refused args"
        );
    }

    /// No matching reply inside the window is the expected "job still
    /// running" case: `None`, not a transport error.
    #[tokio::test]
    async fn run_and_drain_reports_still_running_when_the_window_laps() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind");
        let addr = listener.local_addr().expect("addr");
        let client = tokio::net::TcpStream::connect(addr)
            .await
            .expect("loopback connect");
        let (server, _) = listener.accept().await.expect("accept");
        drop(listener);
        let peer = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::WebSocketStream::from_raw_socket(
                server,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            ),
        )
        .await
        .expect("peer handshake");
        let _silent_peer = peer; // never replies — the window must lap
        let mut conn = DbgpConnection::for_test(client).await;
        assert!(
            conn.run_and_drain(Duration::from_millis(200))
                .await
                .expect("a recv timeout is not an error")
                .is_none()
        );
    }
}
