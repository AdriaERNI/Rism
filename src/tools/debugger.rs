//! Interactive `ObjectScript` debugging over DBGP/`XDebug` — NO uploads, pure
//! Atelier protocol. Nine tools mirroring Prism's debugger surface, plus a
//! one-shot scripted session helper for the CLI.
//!
//! Protocol facts verified live against IRIS 2025.3 (see `src/iris/dbgp.rs`):
//! - the `/debug` WS route exists only under `%25SYS` (Basic auth, `len|b64`
//!   frames);
//! - `feature_set max_data` must precede any property render (agent's
//!   `Features` array) and `stack_get` must precede any `context_get` after a
//!   stop (agent's `StackLevelMappings`) — skipping either CRASHES the agent;
//! - stop location comes from `stack_get` (`run`/step replies carry none);
//! - entry breakpoints need offset 1 (offset 0 → error 201 on 2025.3; this is
//!   why Prism's `stop_on_entry` silently no-ops).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{Error, Result};
use crate::iris::dbgp::{DbgpConnection, attr, elements, text_of};
use crate::iris::http::IrisClient;

/// Idle sessions are reaped after this long (Prism parity).
pub const IDLE_TIMEOUT_SECS: u64 = 300;

// ── Data types ────────────────────────────────────────────────────────

/// One DBGP `<property>` (variable/child).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Variable {
    /// Full name.
    pub name: String,
    /// DBGP type (string, object, …).
    pub r#type: String,
    /// Value text (base64-decoded when the agent encoded it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Nested children (object/array).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Variable>,
    /// Total children (can exceed `children.len()` at `max_depth`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_children: Option<u32>,
}

/// Execution location parsed from a `dbgp://` URI or NS:doc pair.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Location {
    /// Namespace when decodable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Document name (e.g. `My.Class.cls`) or label reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
    /// Line / method-offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Method for the frame, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

/// One `stack_get` frame.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StackFrame {
    /// IDE level (IRIS starts at 1 — NOT 0).
    pub level: u32,
    /// Frame type (`file`, `eval`, …).
    pub r#type: String,
    /// Location.
    pub location: Location,
}

/// Breakpoint info as reported/set.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BreakpointInfo {
    /// Agent id.
    pub id: String,
    /// `line` / `conditional`.
    pub r#type: String,
    /// `enabled` / `disabled`.
    pub state: String,
    /// Where it applies.
    pub location: Location,
    /// Times hit.
    pub hit_count: u32,
}

/// A running IRIS job (`GetJobs`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProcessInfo {
    /// Job pid.
    pub pid: u32,
    /// Namespace.
    pub namespace: String,
    /// Current routine.
    pub routine: String,
    /// State text.
    pub state: String,
    /// Device.
    pub device: String,
}

// ── Session registry ──────────────────────────────────────────────────

/// A live debug session: WS transport + metadata.
pub struct DebugSession {
    /// Registry id.
    pub id: String,
    /// DBGP transport (one logical connection).
    pub conn: AsyncMutex<DbgpConnection>,
    /// Target expression or `PID:n`.
    pub target: String,
    /// Namespace used to prefix the target.
    pub namespace: String,
    /// starting | running | break | ended (guarded).
    state: Mutex<String>,
    /// Idle timer (guarded).
    touched: Mutex<Instant>,
}

impl DebugSession {
    fn set_state(&self, s: &str) {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = s.to_string();
    }
    fn touch(&self) {
        *self
            .touched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }
    fn expired(&self) -> bool {
        self.touched
            .lock()
            .map_or(true, |t| t.elapsed().as_secs() > IDLE_TIMEOUT_SECS)
    }
}

type Map = Mutex<HashMap<String, Arc<DebugSession>>>;

fn sessions() -> &'static Map {
    static S: OnceLock<Map> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(0));
    let x = now ^ (N.fetch_add(1, Ordering::Relaxed) << 32).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut x = x;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    format!("{x:012x}")
}

/// Register a session (max 1 live — Prism parity). Closes expired ones first.
async fn create_session(
    conn: DbgpConnection,
    target: &str,
    namespace: &str,
) -> Result<Arc<DebugSession>> {
    reap_expired().await;
    let session = Arc::new(DebugSession {
        id: next_id(),
        conn: AsyncMutex::new(conn),
        target: target.to_string(),
        namespace: namespace.to_string(),
        state: Mutex::new("starting".into()),
        touched: Mutex::new(Instant::now()),
    });
    {
        let mut map = sessions()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !map.is_empty() {
            return Err(Error::Debug(
                "a debug session is already active — call debug_stop first".to_string(),
            ));
        }
        map.insert(session.id.clone(), session.clone());
    }
    Ok(session)
}

async fn reap_expired() {
    let expired: Vec<Arc<DebugSession>> = {
        let mut map = sessions()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, s)| s.expired())
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| map.remove(id)).collect()
    };
    for s in expired {
        let mut conn = s.conn.lock().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.close()).await;
    }
}

/// Look up + touch a live session.
fn get_session(id: &str) -> Result<Arc<DebugSession>> {
    let map = sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match map.get(id) {
        Some(s) if !s.expired() => {
            s.touch();
            Ok(s.clone())
        }
        _ => Err(Error::Debug(format!(
            "no active debug session with ID '{id}' (sessions expire after {IDLE_TIMEOUT_SECS}s idle)"
        ))),
    }
}

fn remove_session_now(id: &str) -> Option<Arc<DebugSession>> {
    sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(id)
}

// ── Command choreography helpers ──────────────────────────────────────

/// `feature_set` `max_data` → … → `debug_target` (b64 for expressions, plain for
/// PID attach where the VS Code order matters on Windows IRIS — we follow
/// Prism: `debug_target` FIRST for attaches).
async fn configure_features(conn: &mut DbgpConnection) -> Result<()> {
    for (n, v) in [
        ("max_data", "8192"),
        ("max_children", "32"),
        ("max_depth", "2"),
        ("step_granularity", "line"),
    ] {
        conn.command("feature_set", &[("n", n.into()), ("v", v.into())], None)
            .await?;
    }
    Ok(())
}

async fn set_debug_target_b64(conn: &mut DbgpConnection, wire: &str) -> Result<()> {
    // `-v_base64 <b64>` as an ARGUMENT (not the `-- data` payload): the agent
    // reads feature values from arguments only (verified: payload form leaves
    // the target empty and the agent dies in Launch).
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(wire);
    conn.command(
        "feature_set",
        &[("n", "debug_target".into()), ("v_base64", b64)],
        None,
    )
    .await?;
    Ok(())
}

async fn set_debug_target_plain(conn: &mut DbgpConnection, wire: &str) -> Result<()> {
    conn.command(
        "feature_set",
        &[("n", "debug_target".into()), ("v", wire.into())],
        None,
    )
    .await?;
    Ok(())
}

/// `stack_get` → frames (MUST precede `context_get`; fills agent mappings).
async fn fetch_stack(conn: &mut DbgpConnection) -> Result<Vec<StackFrame>> {
    let root = conn.command("stack_get", &[], None).await?;
    Ok(elements(&root)
        .filter(|e| e.name == "stack")
        .map(|node| StackFrame {
            level: attr(node, "level")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            r#type: attr(node, "type").unwrap_or_default().to_string(),
            location: parse_filename(
                attr(node, "filename").unwrap_or_default(),
                attr(node, "lineno"),
            ),
        })
        .collect())
}

/// `context_get` at the innermost mapped level.
async fn fetch_variables(
    conn: &mut DbgpConnection,
    context: u32,
    level: u32,
) -> Result<Vec<Variable>> {
    let root = conn
        .command(
            "context_get",
            &[("c", context.to_string()), ("d", level.to_string())],
            None,
        )
        .await?;
    Ok(elements(&root)
        .filter(|e| e.name == "property")
        .map(parse_property)
        .collect())
}

/// `stack_get` + innermost-level `context_get`. Errors from the variables fetch
/// are swallowed (location still useful), matching Prism's best-effort.
async fn break_context(conn: &mut DbgpConnection) -> Result<(Option<Location>, Vec<Variable>)> {
    let frames = fetch_stack(conn).await?;
    let location = frames.first().map(|f| f.location.clone());
    let level = frames.iter().map(|f| f.level).min().unwrap_or(1).max(1);
    let variables = fetch_variables(conn, 0, level).await.unwrap_or_default();
    Ok((location, variables))
}

fn parse_property(node: &xmltree::Element) -> Variable {
    let name = attr(node, "fullname")
        .or_else(|| attr(node, "name"))
        .unwrap_or_default()
        .to_string();
    let value = text_of(node).map(|t| {
        if attr(node, "encoding") == Some("base64") {
            use base64::Engine as _;
            match base64::engine::general_purpose::STANDARD.decode(&t) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(_) => t,
            }
        } else {
            t
        }
    });
    Variable {
        name,
        r#type: attr(node, "type").unwrap_or_default().to_string(),
        value,
        children: elements(node)
            .filter(|e| e.name == "property")
            .map(parse_property)
            .collect(),
        num_children: attr(node, "numchildren").and_then(|v| v.parse().ok()),
    }
}

fn state_of(status: &str) -> &'static str {
    match status {
        "break" => "break",
        "stopping" | "stopped" => "ended",
        "running" => "running",
        _ => "unknown",
    }
}

/// Root-cause note: IRIS reports file URIs as `dbgp://%7CUSER%7CX.cls`
/// (percent-encoded pipes). Decode then split on '|'; tolerate `NS:doc`.
fn parse_filename(raw: &str, lineno: Option<&str>) -> Location {
    let mut loc = Location {
        namespace: None,
        document: None,
        line: lineno.and_then(|v| v.trim().parse().ok()),
        method: None,
    };
    let decoded = percent_decode(raw);
    let path = decoded
        .strip_prefix("dbgp://")
        .unwrap_or(&decoded)
        .to_string();
    if let Some(rest) = path.strip_prefix('|') {
        let mut it = rest.splitn(2, '|');
        loc.namespace = it.next().map(str::to_string);
        loc.document = it.next().map(str::to_string);
    } else if let Some((ns, doc)) = path.split_once(':') {
        loc.namespace = Some(ns.to_string());
        loc.document = Some(doc.to_string());
    } else if !path.is_empty() {
        loc.document = Some(path);
    }
    loc
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `##class(Pkg.Cls).Method(args)` → (`Pkg.Cls`, `Method`).
fn parse_class_method(target: &str) -> Option<(String, String)> {
    let after = target.strip_prefix("##class(")?;
    let (cls, rest) = after.split_once(')')?;
    let method: String = rest
        .strip_prefix('.')?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let cls = cls.trim();
    if cls.is_empty() || method.is_empty() {
        None
    } else {
        Some((cls.to_string(), method))
    }
}

/// Percent-encode for the pipe-delimited file URI (IRIS expects encoded).
fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' | b'^' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn bp_file_uri(namespace: &str, doc: &str) -> String {
    format!("dbgp://|{}|{}", pct(namespace), pct(doc))
}

/// One breakpoint definition (`debug_start` / `debug_breakpoints` set).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BreakpointSpec {
    /// Class (without `.cls`) — pair with `method`.
    pub class: Option<String>,
    /// Method name for a class breakpoint.
    pub method: Option<String>,
    /// Routine name (`Label^Routine`) — alternative to class/method.
    pub routine: Option<String>,
    /// Line offset in method/routine (entry = 1; 0 fails on 2025.3).
    pub offset: Option<u32>,
    /// Condition expression → conditional breakpoint.
    pub condition: Option<String>,
}

async fn set_breakpoint(
    conn: &mut DbgpConnection,
    bp: &BreakpointSpec,
    namespace: &str,
) -> Result<BreakpointInfo> {
    let mut args: Vec<(&str, String)> = vec![("t", "line".into()), ("s", "enabled".into())];
    let (doc_name, method) = if let Some(class) = bp.class.clone() {
        let doc = format!("{class}.cls");
        args.push(("f", bp_file_uri(namespace, &doc)));
        if let Some(m) = &bp.method {
            args.push(("m", m.clone()));
        }
        (Some(doc), bp.method.clone())
    } else if let Some(routine) = bp.routine.clone() {
        args.push(("f", bp_file_uri(namespace, &routine)));
        (Some(routine), None)
    } else {
        (None, None)
    };
    args.push(("n", bp.offset.unwrap_or(0).to_string()));

    let data = bp.condition.as_ref().map(|c| {
        args[0] = ("t", "conditional".into());
        c.clone()
    });
    let resp = conn
        .command("breakpoint_set", &args, data.as_ref().map(String::as_bytes))
        .await?;
    Ok(BreakpointInfo {
        id: attr(&resp, "id").unwrap_or_default().to_string(),
        r#type: if bp.condition.is_some() {
            "conditional".into()
        } else {
            "line".into()
        },
        state: attr(&resp, "state").unwrap_or("enabled").to_string(),
        location: Location {
            namespace: Some(namespace.to_string()),
            document: doc_name,
            line: bp.offset,
            method,
        },
        hit_count: 0,
    })
}

async fn parse_breakpoint_list(conn: &mut DbgpConnection) -> Result<Vec<BreakpointInfo>> {
    let root = conn.command("breakpoint_list", &[], None).await?;
    Ok(elements(&root)
        .filter(|e| e.name == "breakpoint")
        .map(|node| BreakpointInfo {
            id: attr(node, "id").unwrap_or_default().to_string(),
            r#type: attr(node, "type").unwrap_or_default().to_string(),
            state: attr(node, "state").unwrap_or_default().to_string(),
            location: parse_filename(
                attr(node, "filename").unwrap_or_default(),
                attr(node, "lineno"),
            ),
            hit_count: attr(node, "hit_count")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        })
        .collect())
}

// ── Tools ─────────────────────────────────────────────────────────────

/// Arguments for [`debug_list_processes`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DebugListProcessesArgs {
    /// Filter jobs by namespace (client-side).
    pub namespace: Option<String>,
    /// Include system processes (default false).
    pub system: Option<bool>,
}

/// List running IRIS jobs via `GET /%SYS/jobs` (the documented `GetJobs`
/// Atelier call — no uploads, no `ObjectScript`).
///
/// # Errors
/// Transport/envelope errors.
pub async fn debug_list_processes(
    client: &IrisClient,
    args: &DebugListProcessesArgs,
) -> Result<Vec<ProcessInfo>> {
    let url = format!(
        "{}/jobs?system={}",
        client.api_url("%SYS"),
        u8::from(args.system.unwrap_or(false))
    );
    let env = client.get(&url).await?;
    let content = env
        .result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut procs: Vec<ProcessInfo> = content
        .iter()
        .map(|j| ProcessInfo {
            pid: j
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(0),
            namespace: j
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            routine: j
                .get("routine")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            state: j
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            device: j
                .get("device")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    if let Some(ns) = &args.namespace {
        procs.retain(|p| p.namespace.eq_ignore_ascii_case(ns));
    }
    Ok(procs)
}

/// Arguments for [`debug_start`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugStartArgs {
    /// `ObjectScript` target, e.g. `##class(MyApp.Utils).Add(2,3)`.
    pub target: String,
    /// Break at entry line (default true).
    pub stop_on_entry: Option<bool>,
    /// Breakpoints to set before the first `run`.
    pub breakpoints: Option<Vec<BreakpointSpec>>,
    /// Namespace to prefix the target with (default: configured).
    pub namespace: Option<String>,
}

/// Result of `debug_start` / `debug_attach`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DebugSessionInfo {
    /// Session id for the other debug_* tools.
    pub session_id: String,
    /// starting | running | break | ended.
    pub state: String,
    /// The target expression.
    pub target: String,
    /// Stop location (when break).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Variables at the stop (when break).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
    /// Breakpoints applied up front.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breakpoints: Vec<BreakpointInfo>,
}

/// Open a DBGP session and `run` `target` under the debugger.
///
/// # Errors
/// Transport errors, DBGP errors, or the one-session limit.
pub async fn debug_start(client: &IrisClient, args: &DebugStartArgs) -> Result<DebugSessionInfo> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace);
    let conn = DbgpConnection::connect(client, Duration::from_secs(30)).await?;
    let session = match create_session(conn, &args.target, &ns).await {
        Ok(s) => s,
        Err(e) => {
            drop(e.to_string()); // nothing closable; conn consumed inside create_session only on success
            return Err(e);
        }
    };
    let outcome = start_inner(&session, args, &ns).await;
    if outcome.is_err() {
        if let Some(s) = remove_session_now(&session.id) {
            s.conn.lock().await.close().await;
        }
    }
    outcome
}

async fn start_inner(
    session: &Arc<DebugSession>,
    args: &DebugStartArgs,
    ns: &str,
) -> Result<DebugSessionInfo> {
    let mut conn = session.conn.lock().await;
    configure_features(&mut conn).await?;
    let wire = format!("{ns}:{}", args.target);
    set_debug_target_b64(&mut conn, &wire).await?;

    let mut breakpoints = Vec::new();
    for bp in args.breakpoints.iter().flatten() {
        breakpoints.push(set_breakpoint(&mut conn, bp, ns).await?);
    }

    if args.stop_on_entry.unwrap_or(true)
        && let Some((class, method)) = parse_class_method(&args.target)
    {
        let entry = BreakpointSpec {
            class: Some(class),
            method: Some(method),
            routine: None,
            offset: Some(1),
            condition: None,
        };
        match set_breakpoint(&mut conn, &entry, ns).await {
            Ok(bp) => breakpoints.push(bp),
            // Unmappable entry (fast quit-only method) — fall through to `run`.
            Err(Error::Dbgp { code: 201, .. }) => {}
            Err(e) => return Err(e),
        }
    }

    let resp = conn.command("run", &[], None).await?;
    let status = attr(&resp, "status").unwrap_or("unknown").to_string();
    drop(conn);

    let state = state_of(&status).to_string();
    session.set_state(&state);
    let mut info = DebugSessionInfo {
        session_id: session.id.clone(),
        state: state.clone(),
        target: args.target.clone(),
        location: None,
        variables: Vec::new(),
        breakpoints,
    };
    if state == "break" {
        let mut conn = session.conn.lock().await;
        let (loc, vars) = break_context(&mut conn).await?;
        info.location = loc;
        info.variables = vars;
    }
    Ok(info)
}

/// Arguments for [`debug_attach`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugAttachArgs {
    /// Process id to attach to (from [`debug_list_processes`]).
    pub pid: u32,
    /// Namespace for the debug connection (default: configured).
    pub namespace: Option<String>,
}

/// Attach to a running job. VS Code's sequence: `debug_target` FIRST (plain,
/// not b64), two ignored `run` handshakes, `break`, then poll `step_into`
/// until status=break (IRIS error 998 / 6709 = "not stopped yet", retry).
///
/// # Errors
/// Transport errors, attach timeout, or the one-session limit.
pub async fn debug_attach(client: &IrisClient, args: &DebugAttachArgs) -> Result<DebugSessionInfo> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace);
    let target = format!("PID:{}", args.pid);
    let conn = DbgpConnection::connect(client, Duration::from_secs(30)).await?;
    let session = match create_session(conn, &target, &ns).await {
        Ok(s) => s,
        Err(e) => return Err(e),
    };
    let outcome = attach_inner(&session, args.pid).await;
    if outcome.is_err()
        && let Some(s) = remove_session_now(&session.id)
    {
        s.conn.lock().await.close().await;
    }
    outcome
}

async fn attach_inner(session: &Arc<DebugSession>, pid: u32) -> Result<DebugSessionInfo> {
    let mut conn = session.conn.lock().await;
    set_debug_target_plain(&mut conn, &format!("PID:{pid}")).await?;
    configure_features(&mut conn).await?;

    let timeout = Duration::from_secs(15);
    let started = Instant::now();
    let mut resp_status = String::new();
    // Two runs: IRIS ignores the first for non-CSP PID attach (VS Code parity).
    for _ in 0..2 {
        match conn.command("run", &[], None).await {
            Ok(r) => {
                resp_status = attr(&r, "status").unwrap_or("").to_string();
                if resp_status == "break" {
                    break;
                }
            }
            Err(Error::Dbgp { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    if resp_status != "break" {
        match conn.command("break", &[], None).await {
            Ok(r) => resp_status = attr(&r, "status").unwrap_or("").to_string(),
            Err(Error::Dbgp { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    // Poll `step_into` until the target actually stops.
    while resp_status != "break" && started.elapsed() < timeout {
        match conn.command("step_into", &[], None).await {
            Ok(r) => {
                resp_status = attr(&r, "status").unwrap_or("").to_string();
                if state_of(&resp_status) == "ended" {
                    return Err(Error::Debug(format!(
                        "attach failed: process {pid} returned status '{resp_status}' (it may have finished)"
                    )));
                }
            }
            Err(Error::Dbgp { code, ref message }) if code == 998 || message.contains("6709") => {
                drop(conn);
                tokio::time::sleep(Duration::from_secs(1)).await;
                conn = session.conn.lock().await;
                continue;
            }
            Err(e) => return Err(e),
        }
        if resp_status != "break" {
            drop(conn);
            tokio::time::sleep(Duration::from_secs(1)).await;
            conn = session.conn.lock().await;
        }
    }
    if resp_status != "break" {
        return Err(Error::Debug(format!(
            "attach timed out after 15s: process {pid} never reached 'break' (may not be attachable)"
        )));
    }

    let (loc, vars) = break_context(&mut conn).await?;
    drop(conn);
    session.set_state("break");
    Ok(DebugSessionInfo {
        session_id: session.id.clone(),
        state: "break".to_string(),
        target: format!("PID:{pid}"),
        location: loc,
        variables: vars,
        breakpoints: Vec::new(),
    })
}

/// Arguments for [`debug_step`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugStepArgs {
    /// Session id.
    pub session_id: String,
    /// `step_into` | `step_over` | `step_out` | `run` | break | stop (default `step_into`).
    pub action: Option<String>,
}

/// Outcome of a step action.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DebugStepResult {
    /// Session id.
    pub session_id: String,
    /// New state.
    pub state: String,
    /// Location (break only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Variables (break only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

/// Execute one DBGP step action in a live session.
///
/// # Errors
/// Unknown id / action, transport or DBGP errors.
pub async fn debug_step(_client: &IrisClient, args: &DebugStepArgs) -> Result<DebugStepResult> {
    let action = args
        .action
        .clone()
        .unwrap_or_else(|| "step_into".to_string());
    if !["step_into", "step_over", "step_out", "run", "break", "stop"].contains(&action.as_str()) {
        return Err(Error::Debug(format!("invalid action '{action}'")));
    }
    let session = get_session(&args.session_id)?;

    if action == "stop" {
        {
            let mut conn = session.conn.lock().await;
            let _ = conn.command("stop", &[], None).await;
        }
        if let Some(s) = remove_session_now(&args.session_id) {
            s.conn.lock().await.close().await;
        }
        return Ok(DebugStepResult {
            session_id: args.session_id.clone(),
            state: "ended".into(),
            location: None,
            variables: Vec::new(),
        });
    }

    let resp = {
        let mut conn = session.conn.lock().await;
        conn.command(&action, &[], None).await?
    };
    let status = attr(&resp, "status").unwrap_or("unknown").to_string();
    let state = state_of(&status).to_string();
    session.set_state(&state);

    let mut out = DebugStepResult {
        session_id: args.session_id.clone(),
        state: state.clone(),
        location: None,
        variables: Vec::new(),
    };
    if state == "break" {
        let mut conn = session.conn.lock().await;
        let (loc, vars) = break_context(&mut conn).await?;
        out.location = loc;
        out.variables = vars;
    } else if state == "ended"
        && let Some(s) = remove_session_now(&args.session_id)
    {
        s.conn.lock().await.close().await;
    }
    Ok(out)
}

/// Arguments for [`debug_variables`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugVariablesArgs {
    /// Session id.
    pub session_id: String,
    /// private | public | class (default private).
    pub context: Option<String>,
    /// Stack level (default: innermost, mapped through `stack_get`).
    pub stack_level: Option<u32>,
}

/// Result of `debug_variables`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DebugVariablesResult {
    /// Session id.
    pub session_id: String,
    /// Context name echoed.
    pub context: String,
    /// Level actually used.
    pub stack_level: u32,
    /// Variables.
    pub variables: Vec<Variable>,
}

/// List variables in a context/level (`stack_get` runs first — protocol must).
///
/// # Errors
/// Unknown id or DBGP errors.
pub async fn debug_variables(
    _client: &IrisClient,
    args: &DebugVariablesArgs,
) -> Result<DebugVariablesResult> {
    let session = get_session(&args.session_id)?;
    let context = match args
        .context
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("private") => 0,
        Some("public") => 1,
        Some("class") => 2,
        Some(other) => {
            return Err(Error::Debug(format!(
                "invalid context '{other}' (private|public|class)"
            )));
        }
    };
    let mut conn = session.conn.lock().await;
    let level = match args.stack_level {
        Some(l) => l,
        None => fetch_stack(&mut conn)
            .await?
            .iter()
            .map(|f| f.level)
            .min()
            .unwrap_or(1)
            .max(1),
    };
    let variables = fetch_variables(&mut conn, context, level).await?;
    Ok(DebugVariablesResult {
        session_id: args.session_id.clone(),
        context: ["private", "public", "class"][context as usize].to_string(),
        stack_level: level,
        variables,
    })
}

/// Arguments for [`debug_inspect`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugInspectArgs {
    /// Session id.
    pub session_id: String,
    /// Expression or variable name (evaluated via DBGP `eval`).
    pub expression: String,
}

/// Evaluate an expression in the break context.
///
/// # Errors
/// Unknown id or DBGP errors.
pub async fn debug_inspect(_client: &IrisClient, args: &DebugInspectArgs) -> Result<Variable> {
    let session = get_session(&args.session_id)?;
    let mut conn = session.conn.lock().await;
    // eval works for names AND expressions (property_get has arg-parsing
    // issues with IRIS's agent — Prism comment, confirmed in our probe).
    let root = conn
        .command("eval", &[], Some(args.expression.as_bytes()))
        .await?;
    let node = elements(&root)
        .find(|e| e.name == "property")
        .ok_or_else(|| Error::Debug("eval returned no property".to_string()))?;
    Ok(parse_property(node))
}

/// Arguments for [`debug_stack`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugStackArgs {
    /// Session id.
    pub session_id: String,
}

/// Call stack for the current break.
///
/// # Errors
/// Unknown id or DBGP errors.
pub async fn debug_stack(_client: &IrisClient, args: &DebugStackArgs) -> Result<Vec<StackFrame>> {
    let session = get_session(&args.session_id)?;
    let mut conn = session.conn.lock().await;
    fetch_stack(&mut conn).await
}

/// Arguments for [`debug_breakpoints`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugBreakpointsArgs {
    /// Session id.
    pub session_id: String,
    /// list | set | remove | enable | disable (default list).
    pub action: Option<String>,
    /// Breakpoint id (remove/enable/disable).
    pub id: Option<String>,
    /// Breakpoint spec (set).
    pub breakpoint: Option<BreakpointSpec>,
}

/// Manage breakpoints in a live session.
///
/// # Errors
/// Unknown id / bad action / DBGP errors.
pub async fn debug_breakpoints(_client: &IrisClient, args: &DebugBreakpointsArgs) -> Result<Value> {
    let session = get_session(&args.session_id)?;
    let action = args.action.clone().unwrap_or_else(|| "list".to_string());
    let mut conn = session.conn.lock().await;
    match action.as_str() {
        "list" => {
            let bps = parse_breakpoint_list(&mut conn).await?;
            Ok(serde_json::json!({"breakpoints": bps}))
        }
        "set" => {
            let bp = args
                .breakpoint
                .as_ref()
                .ok_or_else(|| Error::Debug("breakpoint spec required for 'set'".to_string()))?;
            let res = set_breakpoint(&mut conn, bp, &session.namespace).await?;
            Ok(serde_json::json!({"breakpoint": res}))
        }
        "remove" => {
            let id = args
                .id
                .as_deref()
                .ok_or_else(|| Error::Debug("id required for 'remove'".to_string()))?;
            conn.command("breakpoint_remove", &[("d", id.into())], None)
                .await?;
            Ok(serde_json::json!({"removed": id}))
        }
        "enable" | "disable" => {
            let id = args
                .id
                .as_deref()
                .ok_or_else(|| Error::Debug(format!("id required for '{action}'")))?;
            let state = if action == "enable" {
                "enabled"
            } else {
                "disabled"
            };
            conn.command(
                "breakpoint_update",
                &[("d", id.into()), ("s", state.into())],
                None,
            )
            .await?;
            Ok(serde_json::json!({"id": id, "state": state}))
        }
        other => Err(Error::Debug(format!("invalid breakpoint action '{other}'"))),
    }
}

/// Arguments for [`debug_stop`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugStopArgs {
    /// Session id.
    pub session_id: String,
}

/// End a session (stop command best-effort, socket closed, slot freed).
///
/// # Errors
/// Unknown id.
pub async fn debug_stop(_client: &IrisClient, args: &DebugStopArgs) -> Result<Value> {
    let session = get_session(&args.session_id)?;
    {
        let mut conn = session.conn.lock().await;
        let _ = conn.command("stop", &[], None).await;
    }
    if let Some(s) = remove_session_now(&args.session_id) {
        s.conn.lock().await.close().await;
    }
    Ok(serde_json::json!({"session_id": args.session_id, "state": "ended"}))
}

/// Arguments for [`debug_run`] (the scripted CLI session).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DebugRunArgs {
    /// `ObjectScript` target expression.
    pub target: String,
    /// Break at entry line (then continue).
    pub stop_on_entry: Option<bool>,
    /// One optional class/method breakpoint.
    pub breakpoint: Option<BreakpointSpec>,
    /// Namespace for the target.
    pub namespace: Option<String>,
    /// Cap on stops before forcing completion.
    pub max_stops: Option<u32>,
}

/// One recorded stop of a scripted `run`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RunStop {
    /// Break location.
    pub location: Location,
    /// Local variables at the stop.
    pub variables: Vec<Variable>,
}

/// Result of [`debug_run`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DebugRunResult {
    /// Target expression.
    pub target: String,
    /// stops | ended | running.
    pub state: String,
    /// Recorded stops (capped at `max_stops`).
    pub stops: Vec<RunStop>,
    /// True when the cap was reached without finishing.
    pub capped: bool,
}

/// Run a target to completion under the debugger in ONE call: start, walk
/// every stop, then always stop the session so the IRIS job resumes.
///
/// # Errors
/// Transport, DBGP, or session-registry errors.
pub async fn debug_run(client: &IrisClient, args: &DebugRunArgs) -> Result<DebugRunResult> {
    let start = DebugStartArgs {
        target: args.target.clone(),
        stop_on_entry: args.stop_on_entry,
        breakpoints: args.breakpoint.clone().map(|b| vec![b]),
        namespace: args.namespace.clone(),
    };
    let info = match debug_start(client, &start).await {
        Ok(i) => i,
        // A target that finishes inside `run` never breaks — nothing scripted.
        Err(e) => return Err(e),
    };
    let max = args.max_stops.unwrap_or(50).max(1);
    let mut stops = Vec::new();
    let mut state = info.state.clone();
    let mut capped = false;

    if state == "break" {
        stops.push(RunStop {
            location: info.location.clone().unwrap_or(Location {
                namespace: None,
                document: None,
                line: None,
                method: None,
            }),
            variables: info.variables.clone(),
        });
        for _ in 0..max {
            let step = debug_step(
                client,
                &DebugStepArgs {
                    session_id: info.session_id.clone(),
                    action: Some("run".to_string()),
                },
            )
            .await;
            if let Ok(r) = step {
                state = r.state;
                if state == "break" {
                    stops.push(RunStop {
                        location: r.location.clone().unwrap_or(Location {
                            namespace: None,
                            document: None,
                            line: None,
                            method: None,
                        }),
                        variables: r.variables.clone(),
                    });
                    continue;
                }
                break;
            }
            // Agent died (e.g. target raised): end the scripted `run`.
            state = "ended".to_string();
            break;
        }
        if state == "break" {
            capped = true;
        }
    }
    let _ = debug_stop(
        client,
        &DebugStopArgs {
            session_id: info.session_id.clone(),
        },
    )
    .await;
    Ok(DebugRunResult {
        target: args.target.clone(),
        state,
        stops,
        capped,
    })
}

// ── tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn class_method_target_parses() {
        assert_eq!(
            parse_class_method("##class(MyApp.Utils).Add(2,3)"),
            Some(("MyApp.Utils".to_string(), "Add".to_string()))
        );
        assert_eq!(parse_class_method("Do^Routine"), None);
        assert_eq!(parse_class_method("##class( ).M()"), None);
    }

    #[test]
    fn dbgp_uri_decodes_to_ns_doc() {
        let loc = parse_filename("dbgp://%7CUSER%7CRismDbgTest.Adder.cls", Some("1"));
        assert_eq!(loc.namespace.as_deref(), Some("USER"));
        assert_eq!(loc.document.as_deref(), Some("RismDbgTest.Adder.cls"));
        assert_eq!(loc.line, Some(1));
        // plain NS:doc form
        let loc2 = parse_filename("USER:Routine", None);
        assert_eq!(loc2.namespace.as_deref(), Some("USER"));
        assert_eq!(loc2.document.as_deref(), Some("Routine"));
    }

    #[test]
    fn breakpoint_uri_percent_encodes_pipes() {
        assert_eq!(bp_file_uri("%SYS", "A.B.cls"), "dbgp://|%25SYS|A.B.cls");
    }

    #[test]
    fn state_mapping_matches_prism() {
        assert_eq!(state_of("break"), "break");
        assert_eq!(state_of("stopped"), "ended");
        assert_eq!(state_of("stopping"), "ended");
        assert_eq!(state_of("running"), "running");
        assert_eq!(state_of("weird"), "unknown");
    }
}
