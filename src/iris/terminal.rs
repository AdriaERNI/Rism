//! Interactive terminal over the Atelier WebSocket (iris-atelier.md §7).
//! Protocol verified live (init → config → prompt → output*/prompt), mirrors
//! Prism's battle-tested `iris/api/terminal.py`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::error::{Error, Result};
use crate::iris::http::IrisClient;

/// Output of one terminal command.
#[derive(Debug, Clone)]
pub struct TerminalOutcome {
    /// Joined, cleaned output text.
    pub output: String,
    /// Prompt seen after completion (e.g. `USER>`), ANSI-stripped.
    pub prompt: String,
    /// True when output exceeded the configured bound.
    pub truncated: bool,
    /// Chars omitted when truncated.
    pub omitted_chars: usize,
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A live terminal WebSocket: one authenticated session, many commands.
///
/// State set by a command (`set`/`do`) persists across [`TerminalSession::run`]
/// calls — the same server object the one-shot path creates per command, kept
/// open. This is what makes the interactive REPL possible; the one-shot
/// [`execute`] is exactly `open` + one `run` + `close`.
pub struct TerminalSession {
    ws: Ws,
    bound: usize,
    timeout: Duration,
    prompt: String,
}

/// Open a terminal session in `namespace`, authenticating via a dedicated
/// GET for session cookies: sharing an HTTP session across concurrent WS
/// terminals loses output (verified Prism lesson).
///
/// # Errors
/// [`Error::Terminal`] on protocol/timeout failures; transport errors from
/// the cookie handshake otherwise.
pub async fn open(
    client: &IrisClient,
    namespace: &str,
    command_timeout: Duration,
) -> Result<TerminalSession> {
    let st = client.settings();
    let bound = st.terminal_max_output_chars;
    let url = ws_url(st.iris_base_url.trim_end_matches('/'), &client.api_prefix());

    let cookies = client.auth_cookies().await?;
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::Terminal(format!("bad ws url: {e}")))?;
    request.headers_mut().insert(
        "Cookie",
        HeaderValue::from_str(&cookie_header)
            .map_err(|_| Error::Terminal("cookie header unparsable".to_string()))?,
    );

    let (mut ws, _resp) =
        tokio::time::timeout(command_timeout, tokio_tungstenite::connect_async(request))
            .await
            .map_err(|_| Error::Terminal("ws connect timed out".to_string()))?
            .map_err(|e| Error::Terminal(format!("ws connect: {e}")))?;

    let init = next_msg(&mut ws, command_timeout).await?;
    if init.get("type").and_then(Value::as_str) != Some("init") {
        return Err(Error::Terminal(format!("expected init, got {init}")));
    }
    send(
        &mut ws,
        &json!({"type": "config", "namespace": namespace, "rawMode": false}),
    )
    .await?;
    // initial prompt echo, captured so the REPL can show it before any run
    let (_echo, initial_prompt) = wait_prompt(&mut ws, command_timeout).await?;

    Ok(TerminalSession {
        ws,
        bound,
        timeout: command_timeout,
        prompt: clean_text(&initial_prompt),
    })
}

impl TerminalSession {
    /// Run one command to completion: send, then collect every output frame
    /// until the next prompt. Bound applied; full output is returned.
    ///
    /// # Errors
    /// [`Error::Terminal`] on protocol/timeout/server-error frames.
    pub async fn run(&mut self, command: &str) -> Result<TerminalOutcome> {
        send(&mut self.ws, &json!({"type": "prompt", "input": command})).await?;
        let (lines, prompt) = wait_prompt(&mut self.ws, self.timeout).await?;
        let outcome = bound_outcome(&lines, &prompt, self.bound);
        self.prompt.clone_from(&outcome.prompt);
        Ok(outcome)
    }

    /// Current prompt string (e.g. `USER>`), updated after every run.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Run one command streaming: `sink` receives each output frame (raw,
    /// uncleaned) as it arrives; returns the outcome after the prompt frame.
    /// Up to `bound` chars are forwarded; beyond that the stream keeps being
    /// consumed (session stays sane) but is dropped, counted in the outcome.
    ///
    /// # Errors
    /// [`Error::Terminal`] on protocol/timeout/server-error frames; sink
    /// errors abort immediately.
    pub async fn run_stream<F>(&mut self, command: &str, mut sink: F) -> Result<TerminalOutcome>
    where
        F: FnMut(&str) -> Result<()>,
    {
        send(&mut self.ws, &json!({"type": "prompt", "input": command})).await?;
        let mut lines: Vec<String> = Vec::new();
        let mut streamed = 0usize;
        let prompt = loop {
            let msg = next_msg(&mut self.ws, self.timeout).await?;
            match classify(&msg)? {
                Frame::Output(text) => {
                    // outcome keeps the full text (same contract as run);
                    // the bound here only caps what the sink receives.
                    if self.bound == 0 || streamed < self.bound {
                        sink(&text)?;
                        let take = if self.bound == 0 {
                            text.len()
                        } else {
                            self.bound - streamed
                        };
                        streamed += text.len().min(take);
                    }
                    lines.push(text);
                }
                Frame::Prompt(p) => break p,
                Frame::Ignored => {}
            }
        };
        Ok(bound_outcome(&lines, &prompt, self.bound))
    }

    /// After an abandoned command (Ctrl+C), swallow any late frames so the
    /// next `run` does not see stale output. Bounded wait, then give up.
    ///
    /// # Errors
    /// [`Error::Terminal`] if the socket dies during the drain.
    pub async fn drain(&mut self) -> Result<()> {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), self.ws.next()).await {
                Err(_elapsed) => return Ok(()), // quiet: session idle again
                Ok(None | Some(Err(_))) => {
                    return Err(Error::Terminal("websocket closed".to_string()));
                }
                Ok(Some(Ok(Message::Text(t)))) => {
                    if let Ok(msg) = serde_json::from_str::<Value>(&t) {
                        if matches!(classify(&msg), Ok(Frame::Prompt(_))) {
                            return Ok(());
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    /// Close the session politely.
    ///
    /// # Errors
    /// Transport errors from the close frame exchange.
    pub async fn close(mut self) -> Result<()> {
        self.ws
            .close(None)
            .await
            .map_err(|e| Error::Terminal(format!("ws close: {e}")))
    }
}

/// Classify one protocol frame (pure decision, unit-tested without a socket).
enum Frame {
    Output(String),
    Prompt(String),
    /// read/readchar/unknown: consume and continue (Prism parity).
    Ignored,
}

fn classify(msg: &Value) -> Result<Frame> {
    match msg.get("type").and_then(Value::as_str) {
        Some("output") => Ok(Frame::Output(
            msg.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        Some("prompt") => Ok(Frame::Prompt(
            msg.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        Some("error") => Err(Error::Terminal(format!(
            "server error: {}",
            msg.get("text").and_then(Value::as_str).unwrap_or("unknown")
        ))),
        Some("init") => Err(Error::Terminal("unexpected init mid-session".into())),
        _ => Ok(Frame::Ignored),
    }
}

/// Join + clean + bound-apply frames into one outcome (shared by run paths).
fn bound_outcome(lines: &[String], prompt: &str, bound: usize) -> TerminalOutcome {
    let joined = clean_text(&lines.join("\n"));
    let (output, truncated, omitted_chars) = if bound > 0 && joined.len() > bound {
        (joined[..bound].to_string(), true, joined.len() - bound)
    } else {
        (joined, false, 0)
    };
    TerminalOutcome {
        output,
        prompt: clean_text(prompt),
        truncated,
        omitted_chars,
    }
}

/// Run `command` in `namespace` on a fresh one-shot terminal session
/// (open → run → close). The scripting/MCP contract: no state persists
/// between calls.
///
/// # Errors
/// [`Error::Terminal`] on protocol/timeout/server-error frames; transport
/// errors from the cookie handshake otherwise.
pub async fn execute(
    client: &IrisClient,
    namespace: &str,
    command: &str,
    timeout: Duration,
) -> Result<TerminalOutcome> {
    open(client, namespace, timeout).await?.run(command).await
}

fn ws_url(base: &str, prefix: &str) -> String {
    let (scheme, rest) = match base.split_once("://") {
        Some(("https", r)) => ("wss", r),
        Some(("http", r)) => ("ws", r),
        _ => ("ws", base),
    };
    format!("{scheme}://{rest}{prefix}/%25SYS/terminal")
}

async fn send(ws: &mut Ws, msg: &Value) -> Result<()> {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .map_err(|e| Error::Terminal(format!("ws send: {e}")))
}

async fn next_msg(ws: &mut Ws, timeout: Duration) -> Result<Value> {
    let msg = tokio::time::timeout(timeout, ws.next())
        .await
        .map_err(|_| Error::Terminal("terminal timed out".to_string()))?
        .ok_or_else(|| Error::Terminal("websocket closed".to_string()))?
        .map_err(|e| Error::Terminal(format!("ws recv: {e}")))?;
    match msg {
        Message::Text(t) => {
            serde_json::from_str(&t).map_err(|e| Error::Terminal(format!("bad frame json: {e}")))
        }
        Message::Close(_) => Err(Error::Terminal("websocket closed".to_string())),
        _ => Ok(Value::Null),
    }
}

/// Consume frames until a prompt arrives; returns (output chunks, prompt).
async fn wait_prompt(ws: &mut Ws, timeout: Duration) -> Result<(Vec<String>, String)> {
    let mut lines = Vec::new();
    loop {
        let msg = next_msg(ws, timeout).await?;
        match classify(&msg)? {
            Frame::Output(text) => lines.push(text),
            Frame::Prompt(prompt) => return Ok((lines, prompt)),
            Frame::Ignored => {}
        }
    }
}

/// Strip ANSI SGR sequences and stray control chars (keep \n \r \t).
fn clean_text(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' {
            // try full CSI ... m (SGR) skip
            if i + 1 < chars.len() && chars[i + 1] == '[' {
                let mut j = i + 2;
                while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == ';') {
                    j += 1;
                }
                if j < chars.len() && chars[j] == 'm' {
                    i = j + 1;
                    continue;
                }
            }
            i += 1; // drop bare ESC
            continue;
        }
        let c = chars[i];
        if matches!(c, '\n' | '\r' | '\t') || (!c.is_control() && c != '\x7f') {
            out.push(c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_shapes() {
        assert_eq!(
            ws_url("http://h:52773", "/api/atelier/v8"),
            "ws://h:52773/api/atelier/v8/%25SYS/terminal"
        );
        assert_eq!(
            ws_url("https://h", "/api/atelier/v7"),
            "wss://h/api/atelier/v7/%25SYS/terminal"
        );
    }

    #[test]
    fn clean_strips_ansi_keeps_text() {
        assert_eq!(
            clean_text("\x1b[31;1m<NOROUTINE>\x1b[0m *Foo"),
            "<NOROUTINE> *Foo"
        );
        assert_eq!(clean_text("a\x00b\nc"), "ab\nc");
    }

    #[test]
    fn classify_maps_frame_types() {
        assert!(matches!(
            classify(&json!({"type": "output", "text": "hi"})),
            Ok(Frame::Output(t)) if t == "hi"
        ));
        assert!(matches!(
            classify(&json!({"type": "prompt", "text": "USER>"})),
            Ok(Frame::Prompt(t)) if t == "USER>"
        ));
        // read/readchar/unknown ignored (Prism parity)
        assert!(matches!(
            classify(&json!({"type": "read"})),
            Ok(Frame::Ignored)
        ));
        assert!(classify(&json!({"type": "error", "text": "boom"})).is_err());
        assert!(classify(&json!({"type": "init"})).is_err());
    }

    #[test]
    fn bound_outcome_applies_cap_once() {
        // frames join with \n: "aaa\nbbb" capped at 4 chars
        let lines = vec!["aaa".to_string(), "bbb".to_string()];
        let o = bound_outcome(&lines, "USER>", 4);
        assert_eq!(o.output, "aaa\n");
        assert!(o.truncated);
        assert_eq!(o.omitted_chars, 3);
        let o = bound_outcome(&lines, "USER>", 0);
        assert_eq!(o.output, "aaa\nbbb");
        assert!(!o.truncated);
        assert_eq!(o.prompt, "USER>");
    }
}
