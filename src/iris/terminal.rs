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
    /// True when the command was aborted server-side via a protocol
    /// `{"type":"interrupt"}` (real break; the child unwinds with
    /// `<INTERRUPT>` in milliseconds instead of running to completion).
    pub interrupted: bool,
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Streaming callbacks a [`TerminalSession::run_with`] caller can hook into.
/// All three are synchronous by design (the REPL's editor/stdin reads block
/// on purpose; the caller's cancel flag is polled, not awaited).
pub trait StreamHooks {
    /// One raw output frame, as it arrives.
    ///
    /// # Errors
    /// Propagated to abort the command (e.g. broken stdout).
    fn emit(&mut self, text: &str) -> Result<()>;
    /// Program hit `read`/`readchar`: produce one input line (no newline).
    ///
    /// # Errors
    /// Propagated to abort the command.
    fn read_prompt(&mut self) -> Result<String>;
    /// Shared cancel flag polled while waiting for frames; true aborts the
    /// command server-side with a protocol interrupt.
    fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool>;
}

/// Hooks that drop output and answer `read` with an empty line (one-shot
/// `execute`/background: nobody is typing, never leave the child blocked).
struct NullHooks;

impl StreamHooks for NullHooks {
    fn emit(&mut self, _text: &str) -> Result<()> {
        Ok(())
    }
    fn read_prompt(&mut self) -> Result<String> {
        Ok(String::new())
    }
    fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
    }
}

/// Back-compat adapter: closure sink, empty read replies, no cancel.
struct SinkHooks<F>(F, std::sync::Arc<std::sync::atomic::AtomicBool>);

impl<F: FnMut(&str) -> Result<()>> StreamHooks for SinkHooks<F> {
    fn emit(&mut self, text: &str) -> Result<()> {
        (self.0)(text)
    }
    fn read_prompt(&mut self) -> Result<String> {
        Ok(String::new())
    }
    fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.1.clone()
    }
}

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
        self.run_with(command, &mut NullHooks).await
    }

    /// Run one command with streaming/read/cancel hooks (the REPL and MCP
    /// background paths). A cancel-flag trip sends the protocol
    /// `{"type":"interrupt"}` — a real server-side break — and the outcome
    /// marks `interrupted` once the prompt returns.
    ///
    /// # Errors
    /// [`Error::Terminal`] on protocol/timeout/server-error frames; hook
    /// errors abort immediately.
    pub async fn run_with<H: StreamHooks + ?Sized>(
        &mut self,
        command: &str,
        hooks: &mut H,
    ) -> Result<TerminalOutcome> {
        let r = self.run_with_inner(command, hooks).await;
        if r.is_err() {
            // The child may still be executing (timeout/socket error):
            // break it and resync to a prompt so nothing mutates the
            // namespace after we gave up on the command.
            let _ = send(&mut self.ws, &json!({"type": "interrupt"})).await;
            let _ = self.drain().await;
        }
        r
    }

    /// `(retained_bytes, dropped_chars)` for one output frame against the
    /// byte cap. Pure so the accounting is unit-testable without a server.
    /// `retained` is BYTES on purpose (cap math; the truncation cut is
    /// char-boundary-floored in [`bound_outcome`]); `dropped` counts CHARS
    /// because it feeds the wire field `output_omitted_chars` ("chars
    /// omitted") — issue #16 F2: summing `text.len()` there lied for
    /// multibyte output.
    fn account_frame(text: &str, streamed: usize, bound: usize) -> (bool, usize, usize) {
        if bound == 0 || streamed < bound {
            let take = if bound == 0 {
                text.len()
            } else {
                bound - streamed
            };
            (true, text.len().min(take), 0)
        } else {
            (false, 0, text.chars().count())
        }
    }

    async fn run_with_inner<H: StreamHooks + ?Sized>(
        &mut self,
        command: &str,
        hooks: &mut H,
    ) -> Result<TerminalOutcome> {
        send(&mut self.ws, &json!({"type": "prompt", "input": command})).await?;
        // checked_add: an absurd timeout must NEVER overflow the deadline
        // math (issue #16: `Instant + Duration` panic => the request hung
        // without answering). Ingresses clamp to MAX_TIMEOUT_SECS, so this
        // fallback is defense-in-depth for future callers; now + 7d cannot
        // itself overflow.
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .unwrap_or_else(|| {
                tokio::time::Instant::now() + Duration::from_secs(crate::tools::MAX_TIMEOUT_SECS)
            });
        let mut lines: Vec<String> = Vec::new();
        let mut streamed = 0usize;
        let mut dropped_chars = 0usize;
        let mut interrupted = false;
        let prompt = loop {
            match self.next_or_cancel(deadline, hooks.cancel_flag()).await? {
                Next::Frame(msg) => match classify(&msg)? {
                    Frame::Output(text) => {
                        // under-bound only: once the cap is reached, keep
                        // consuming frames (protocol sync) but neither emit
                        // nor RETAIN them — unbounded lines would defeat
                        // terminal_max_output_chars on chatty commands
                        let (emit, take, dropped) =
                            Self::account_frame(&text, streamed, self.bound);
                        if emit {
                            hooks.emit(&text)?;
                            streamed += take;
                            lines.push(text);
                        } else {
                            dropped_chars += dropped;
                        }
                    }
                    Frame::Prompt(p) => break p,
                    Frame::Read => {
                        // A running program hit read/readchar: answer it or
                        // the child blocks until our timeout (Prism lesson:
                        // NEVER drop this frame).
                        let input = hooks.read_prompt()?;
                        send(&mut self.ws, &json!({"type": "read", "input": input})).await?;
                    }
                    Frame::Ignored => {}
                },
                Next::Cancelled => {
                    // Server-side break (interrupt child), then the agent
                    // emits <INTERRUPT> and the prompt — wait for it so the
                    // session is clean for the next command.
                    interrupted = true;
                    hooks.emit("\n")?;
                    send(&mut self.ws, &json!({"type": "interrupt"})).await?;
                    // Fresh bounded window for the break to settle: a cancel
                    // landing at 99% of the command deadline must still
                    // interrupt (not time out mid-interrupt).
                    let settle = tokio::time::Instant::now() + Duration::from_secs(5);
                    break self.await_prompt(settle).await?;
                }
            }
        };
        if interrupted {
            lines.push("<INTERRUPT>".to_string());
        }
        let mut outcome = bound_outcome(&lines, &prompt, self.bound);
        if dropped_chars > 0 {
            // frames past the cap were consumed but never retained
            outcome.truncated = true;
            outcome.omitted_chars += dropped_chars;
        }
        outcome.interrupted = interrupted;
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
    /// Read frames get an empty reply (nothing is typing); use
    /// [`TerminalSession::run_with`] for full REPL semantics.
    ///
    /// # Errors
    /// [`Error::Terminal`] on protocol/timeout/server-error frames; sink
    /// errors abort immediately.
    pub async fn run_stream<F>(&mut self, command: &str, sink: F) -> Result<TerminalOutcome>
    where
        F: FnMut(&str) -> Result<()>,
    {
        self.run_with(
            command,
            &mut SinkHooks(
                sink,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ),
        )
        .await
    }

    /// After an abandoned command (Ctrl+C), swallow any late frames so the
    /// next `run` does not see stale output. Bounded wait, then give up.
    /// A pending `read` gets an empty reply (nothing is typing).
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
                        match classify(&msg) {
                            Ok(Frame::Prompt(p)) => {
                                self.prompt = clean_text(&p);
                                return Ok(());
                            }
                            Ok(Frame::Read) => {
                                send(&mut self.ws, &json!({"type": "read", "input": ""})).await?;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    /// Wait for frames honoring a cancel flag; `Ok(Next::Cancelled)` when
    /// the flag trips first. Deadline applies to both. The flag is checked
    /// at every iteration, so a flood of output frames cannot starve cancel.
    async fn next_or_cancel(
        &mut self,
        deadline: tokio::time::Instant,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Next> {
        use std::sync::atomic::Ordering;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(Next::Cancelled);
            }
            let budget = deadline.saturating_duration_since(tokio::time::Instant::now());
            if budget.is_zero() {
                return Err(Error::Terminal("terminal timed out".to_string()));
            }
            // 50ms tick keeps cancel latency low without busy-looping hard.
            let tick = std::cmp::min(budget, Duration::from_millis(50));
            match tokio::time::timeout(tick, self.ws.next()).await {
                Err(_elapsed) => {} // re-loop: flag checked at top
                Ok(Some(Ok(Message::Text(t)))) => {
                    return serde_json::from_str(&t)
                        .map(Next::Frame)
                        .map_err(|e| Error::Terminal(format!("bad frame json: {e}")));
                }
                Ok(None) => return Err(Error::Terminal("websocket closed".to_string())),
                Ok(Some(Err(_) | Ok(Message::Close(_)))) => {
                    return Err(Error::Terminal("websocket closed".to_string()));
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    /// Pull frames until a prompt, replying to reads; used post-interrupt.
    /// Ignores the caller's cancel flag (it is still tripped at this point —
    /// honoring it here would spin); the deadline is the only escape.
    async fn await_prompt(&mut self, deadline: tokio::time::Instant) -> Result<String> {
        let settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        loop {
            match self.next_or_cancel(deadline, settled.clone()).await? {
                Next::Frame(msg) => match classify(&msg)? {
                    Frame::Prompt(p) => return Ok(p),
                    Frame::Read => {
                        send(&mut self.ws, &json!({"type": "read", "input": ""})).await?;
                    }
                    Frame::Output(_) | Frame::Ignored => {}
                },
                Next::Cancelled => unreachable!("flag is never set"),
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
    /// The child hit `read`/`readchar`: the client must answer with a
    /// `{"type":"read","input":…}` message or the program blocks.
    Read,
    /// readchar-style variants we do not react to.
    Ignored,
}

/// Outcome of a cancel-aware frame wait.
enum Next {
    Frame(Value),
    Cancelled,
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
        Some("read") => Ok(Frame::Read),
        Some("error") => Err(Error::Terminal(format!(
            "server error: {}",
            msg.get("text").and_then(Value::as_str).unwrap_or("unknown")
        ))),
        Some("init") => Err(Error::Terminal("unexpected init mid-session".into())),
        _ => Ok(Frame::Ignored),
    }
}

/// Largest char boundary at or below `want` (MSRV-safe `floor_char_boundary`,
/// the std method needs 1.91).
fn floor_char_boundary(s: &str, want: usize) -> usize {
    if want >= s.len() {
        return s.len();
    }
    let mut i = want;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Join + clean + bound-apply frames into one outcome (shared by run paths).
fn bound_outcome(lines: &[String], prompt: &str, bound: usize) -> TerminalOutcome {
    let joined = clean_text(&lines.join("\n"));
    let (output, truncated, omitted_chars) = if bound > 0 && joined.len() > bound {
        // floor to a char boundary: a multibyte char may straddle `bound`
        let cut = floor_char_boundary(&joined, bound);
        // chars beyond the cut, not bytes (issue #16 F2: the field feeding
        // `output_omitted_chars` is documented "chars omitted")
        (
            joined[..cut].to_string(),
            true,
            joined[cut..].chars().count(),
        )
    } else {
        (joined, false, 0)
    };
    TerminalOutcome {
        output,
        prompt: clean_text(prompt),
        truncated,
        omitted_chars,
        interrupted: false,
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
    // timeout caps a TCP write stall (peer zero window) that would otherwise
    // hang the run loop past its deadline; 10s is generous for prompt frames.
    tokio::time::timeout(
        Duration::from_secs(10),
        ws.send(Message::Text(msg.to_string().into())),
    )
    .await
    .map_err(|_| Error::Terminal("ws send stalled".to_string()))?
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
/// A pending `read` gets an empty reply (one-shot path: nothing is typing).
async fn wait_prompt(ws: &mut Ws, timeout: Duration) -> Result<(Vec<String>, String)> {
    let mut lines = Vec::new();
    loop {
        let msg = next_msg(ws, timeout).await?;
        match classify(&msg)? {
            Frame::Output(text) => lines.push(text),
            Frame::Prompt(prompt) => return Ok((lines, prompt)),
            Frame::Read => send(ws, &json!({"type": "read", "input": ""})).await?,
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
                // CSI: parameter bytes, then a final byte @..~  (SGR 'm' and
                // erase-line 'K', cursor moves, etc. — all must vanish whole)
                let mut j = i + 2;
                while j < chars.len()
                    && (chars[j].is_ascii_digit()
                        || matches!(chars[j], ';' | '<' | '=' | '>' | '?'))
                {
                    j += 1;
                }
                if j < chars.len() && ('@'..='~').contains(&chars[j]) {
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
        // read frames now demand a reply (Frame::Read), not silence
        assert!(matches!(
            classify(&json!({ "type": "read" })),
            Ok(Frame::Read)
        ));
        assert!(matches!(
            classify(&json!({ "type": "readchar" })),
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

    #[test]
    fn bound_outcome_multibyte_straddle_no_panic() {
        // 2-byte chars straddling the byte bound must floor, not panic
        let lines = vec!["ééé".to_string()]; // 6 bytes, bound 3 lands mid-char
        let o = bound_outcome(&lines, "USER>", 3);
        assert!(o.truncated);
        assert_eq!(o.output, "é");
        // 2 CHARS omitted (4 bytes) — issue #16 F2: this field is
        // documented "chars omitted"; the old assertion pinned the byte lie.
        assert_eq!(o.omitted_chars, 2);
    }

    #[test]
    fn account_frame_counts_chars_dropped_bytes_retained() {
        // under the cap: retain whole frames, byte cap math
        assert_eq!(TerminalSession::account_frame("abcd", 0, 10), (true, 4, 0));
        // partial fill: retained is capped to remaining budget
        assert_eq!(TerminalSession::account_frame("abcd", 8, 10), (true, 2, 0));
        // past the cap: multibyte drops count CHARS, not bytes (F2)
        assert_eq!(TerminalSession::account_frame("éé", 10, 10), (false, 0, 2));
        // bound 0 = unbounded: everything retained, nothing dropped
        assert_eq!(
            TerminalSession::account_frame("héllo", 999, 0),
            (true, 6, 0)
        );
    }
}
