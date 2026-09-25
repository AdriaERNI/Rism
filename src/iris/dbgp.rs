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

        let raw = recv_frame(&mut self.ws, Duration::from_secs(30)).await?;

        if raw.starts_with("<exit") || raw.contains("&lt;EXIT&gt;") || raw.contains("<UNDEFINED>") {
            return Err(Error::Debug(format!(
                "XDebug agent crashed on '{name}': {}",
                raw.chars().take(200).collect::<String>()
            )));
        }

        let root = Element::parse(std::io::Cursor::new(raw.as_bytes()))
            .map_err(|e| Error::Debug(format!("bad DBGP xml for '{name}': {e}")))?;

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
}
