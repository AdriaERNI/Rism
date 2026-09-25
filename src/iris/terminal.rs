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

/// Run `command` in `namespace` on a fresh one-shot terminal session.
///
/// A dedicated authenticated GET supplies session cookies: sharing an HTTP
/// session across concurrent WS terminals loses output (verified Prism lesson).
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
    let st = client.settings();
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

    let (mut ws, _resp) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| Error::Terminal("ws connect timed out".to_string()))?
        .map_err(|e| Error::Terminal(format!("ws connect: {e}")))?;

    let finish = async {
        // 1. init
        let init = next_msg(&mut ws, timeout).await?;
        if init.get("type").and_then(Value::as_str) != Some("init") {
            return Err(Error::Terminal(format!("expected init, got {init}")));
        }

        // 2. config
        send(
            &mut ws,
            &json!({"type": "config", "namespace": namespace, "rawMode": false}),
        )
        .await?;

        // 3. initial prompt (discard echo)
        wait_prompt(&mut ws, timeout).await?;

        // 4. the command
        send(&mut ws, &json!({"type": "prompt", "input": command})).await?;

        // 5. collect until next prompt
        let (lines, prompt) = wait_prompt(&mut ws, timeout).await?;
        Ok::<_, Error>((lines, prompt))
    }
    .await;

    let _ = ws.close(None).await;
    let (lines, prompt) = finish?;

    let joined = clean_text(&lines.join("\n"));
    let bound = client.settings().terminal_max_output_chars;
    let (output, truncated, omitted_chars) = if bound > 0 && joined.len() > bound {
        (joined[..bound].to_string(), true, joined.len() - bound)
    } else {
        (joined, false, 0)
    };

    Ok(TerminalOutcome {
        output,
        prompt: clean_text(&prompt),
        truncated,
        omitted_chars,
    })
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
        match msg.get("type").and_then(Value::as_str) {
            Some("output") => {
                let text = msg.get("text").and_then(Value::as_str).unwrap_or_default();
                lines.push(text.to_string());
            }
            Some("prompt") => {
                return Ok((
                    lines,
                    msg.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ));
            }
            Some("error") => {
                return Err(Error::Terminal(format!(
                    "server error: {}",
                    msg.get("text").and_then(Value::as_str).unwrap_or("unknown")
                )));
            }
            Some("init") => return Err(Error::Terminal("unexpected init mid-session".into())),
            _ => {} // read/readchar/unknown: ignore (Prism parity)
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
}
