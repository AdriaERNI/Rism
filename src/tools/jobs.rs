//! Background terminal-job registry for MCP Tasks (SEP-2663).
//!
//! A background job runs `ObjectScript` on its own terminal session inside
//! the MCP server process: `execute_command(background=true)` returns a
//! `taskId` immediately (this module's `start`), `tasks/get` polls state +
//! streamed output (via `status` + the projection in `crate::mcp::tasks`),
//! and `tasks/cancel` trips the cancel flag whose next frame poll sends the
//! protocol `{"type":"interrupt"}` — a real server-side break (verified
//! against the Atelier agent: the child unwinds with `<INTERRUPT>` in
//! milliseconds). Jobs live only as long as the server process; finished
//! jobs are garbage-collected after [`RETENTION`] (advertised as `ttlMs`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::iris::IrisClient;
use crate::iris::terminal as api;
use crate::tools::command::CommandResult;

/// Tail kept per job (ring buffer; total produced is reported separately).
pub const OUTPUT_CAP: usize = 100_000;
/// Finished jobs older than this are dropped from the registry.
pub const RETENTION: Duration = Duration::from_secs(30 * 60);
/// Upper bound on concurrently running jobs (leak guard).
pub const MAX_RUNNING: usize = 16;
/// Upper bound on registry entries (finished or not); oldest finished jobs
/// are evicted first. Bounds idle memory within retention.
pub const MAX_ENTRIES: usize = 256;

/// One tracked background command (serializable view).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct JobInfo {
    /// Opaque handle — the MCP `taskId` for polling / cancellation.
    pub id: String,
    /// The command as sent.
    pub command: String,
    /// Namespace the job runs in.
    pub namespace: String,
    /// Wall-clock start (unix seconds).
    pub started_unix: u64,
    /// Wall-clock finish (unix seconds), when done.
    pub finished_unix: Option<u64>,
    /// True while the command is still executing.
    pub running: bool,
    /// True when the job was cancelled via a protocol interrupt.
    pub interrupted: bool,
    /// Terminal error (transport/timeout), when the job failed.
    pub error: Option<String>,
    /// Prompt seen after completion.
    pub prompt: Option<String>,
    /// Total output produced so far, in bytes (may exceed `output`).
    pub output_bytes: usize,
    /// Streaming output tail (up to [`OUTPUT_CAP`] bytes), concatenated raw
    /// frames. Progress display only — the equivalent-of-sync output after
    /// finish lives in [`Self::final_result`].
    pub output: String,
    /// The exact synchronous `CommandResult`, stored by `finish` from the
    /// terminal outcome — a completed task's result payload is built from
    /// THIS (SEP-2663 equivalence). None while running. Not serialized:
    /// the wire contract is the task projection (`crate::mcp::tasks`).
    #[serde(skip)]
    pub final_result: Option<CommandResult>,
}

/// Output sink shared between the task and readers.
#[derive(Clone)]
struct SharedBuf {
    tail: Arc<Mutex<String>>,
    total: Arc<AtomicUsize>,
}

impl SharedBuf {
    fn new() -> Self {
        Self {
            tail: Arc::new(Mutex::new(String::new())),
            total: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn push(&self, text: &str) {
        self.total.fetch_add(text.len(), Ordering::Relaxed);
        let mut b = self
            .tail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        b.push_str(text);
        if b.len() > OUTPUT_CAP {
            // floor to a char boundary: drain() panics mid-char
            let mut cut = b.len() - OUTPUT_CAP;
            while cut > 0 && !b.is_char_boundary(cut) {
                cut -= 1;
            }
            b.drain(..cut);
        }
    }
    /// `(total_bytes, tail_snapshot)`
    fn snapshot(&self) -> (usize, String) {
        let tail = self
            .tail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        (self.total.load(Ordering::Relaxed), tail)
    }
}

struct JobSlot {
    meta: Mutex<JobMeta>,
    buf: SharedBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
struct JobMeta {
    command: String,
    namespace: String,
    started_unix: u64,
    finished_unix: Option<u64>,
    running: bool,
    interrupted: bool,
    error: Option<String>,
    prompt: Option<String>,
    /// Joined final output stored by `finish()`; None while running.
    final_result: Option<CommandResult>,
}

type Registry = HashMap<String, Arc<JobSlot>>;

static JOBS: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_reg() -> RegGuard {
    // OPLOCK serializes whole registry operations (clear/insert/list) so
    // parallel tests on the shared static cannot interleave.
    static OPLOCK: Mutex<()> = Mutex::new(());
    RegGuard(
        OPLOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// Registry handle holding both mutexes for the operation's duration.
struct RegGuard(
    #[expect(dead_code, reason = "guard exists only for its Drop")]
    std::sync::MutexGuard<'static, ()>,
    std::sync::MutexGuard<'static, Registry>,
);

impl std::ops::Deref for RegGuard {
    type Target = Registry;
    fn deref(&self) -> &Registry {
        &self.1
    }
}

impl std::ops::DerefMut for RegGuard {
    fn deref_mut(&mut self) -> &mut Registry {
        &mut self.1
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_id() -> String {
    // SEP-2663 Security: task ids MUST be unguessable (they double as
    // bearer tokens for the stored task). pid keeps the id traceable in
    // logs; the 64-bit CSPRNG draw kills enumeration.
    format!(
        "rism-{:x}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    )
}

/// Start `command` on a dedicated terminal session; returns its job id
/// immediately. Output streams into the registry as frames arrive.
///
/// # Errors
/// [`Error::Config`] when [`MAX_RUNNING`] jobs are already active.
pub fn start(
    client: &IrisClient,
    namespace: Option<String>,
    command: String,
    timeout: Duration,
) -> Result<JobInfo> {
    let ns = namespace.unwrap_or_else(|| client.settings().iris_namespace.clone());
    let slot = Arc::new(JobSlot {
        meta: Mutex::new(JobMeta {
            command: command.clone(),
            namespace: ns.clone(),
            started_unix: unix_now(),
            finished_unix: None,
            running: true,
            interrupted: false,
            error: None,
            prompt: None,
            final_result: None,
        }),
        buf: SharedBuf::new(),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let id = new_id();
    {
        let mut reg = lock_reg();
        gc(&mut reg);
        if reg.values().filter(|s| s.meta().running).count() >= MAX_RUNNING {
            return Err(Error::Config(format!(
                "too many running background jobs ({MAX_RUNNING}); cancel one first"
            )));
        }
        reg.insert(id.clone(), slot.clone());
    }

    let client = client.clone();
    let task_slot = slot.clone();
    tokio::spawn(async move {
        // Panic-safe finalize: a task panic (e.g. allocation in a wild frame)
        // would otherwise leave running=true forever — a GC-proof zombie
        // burning one of the 16 slots. Drop runs even on unwind.
        let mut guard = FinishOnDrop {
            slot: task_slot,
            done: false,
        };
        let outcome = run_job(client, ns, command, timeout, guard.slot.clone()).await;
        finish(&guard.slot, outcome);
        guard.done = true;
    });
    Ok(snapshot(&id, &slot, true))
}

impl JobSlot {
    fn meta(&self) -> JobMeta {
        self.meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Task body: one private session, streaming into the slot's shared buffer.
async fn run_job(
    client: IrisClient,
    namespace: String,
    command: String,
    timeout: Duration,
    slot: Arc<JobSlot>,
) -> std::result::Result<api::TerminalOutcome, String> {
    let mut session = match api::open(&client, &namespace, timeout).await {
        Ok(s) => s,
        Err(e) => return Err(e.to_string()),
    };
    let mut hooks = JobHooks {
        buf: slot.buf.clone(),
        cancel: slot.cancel.clone(),
    };
    // open and run_with are EACH bounded by `timeout` (worst case ~2x
    // wall-clock); a trip → protocol interrupt, and the error path also
    // breaks the child, so a hung command can never outlive the job.
    match session.run_with(&command, &mut hooks).await {
        Ok(o) => Ok(o),
        Err(e) => Err(e.to_string()),
    }
}

struct JobHooks {
    buf: SharedBuf,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl api::StreamHooks for JobHooks {
    fn emit(&mut self, text: &str) -> crate::error::Result<()> {
        self.buf.push(text);
        Ok(())
    }
    fn read_prompt(&mut self) -> crate::error::Result<String> {
        // no human is typing: answer server-side reads with an empty line
        Ok(String::new())
    }
    fn cancel_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.cancel.clone()
    }
}

struct FinishOnDrop {
    slot: Arc<JobSlot>,
    done: bool,
}

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        if !self.done {
            finish(&self.slot, Err("job task panicked".to_string()));
        }
    }
}

fn finish(slot: &Arc<JobSlot>, outcome: std::result::Result<api::TerminalOutcome, String>) {
    {
        let mut m = slot
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        m.running = false;
        m.finished_unix = Some(unix_now());
        match outcome {
            Ok(o) => {
                m.interrupted = o.interrupted;
                m.prompt = Some(o.prompt.clone());
                m.final_result = Some(CommandResult {
                    namespace: m.namespace.clone(),
                    command: m.command.clone(),
                    output: o.output,
                    prompt: o.prompt,
                    output_truncated: o.truncated,
                    output_omitted_chars: o.omitted_chars,
                });
            }
            Err(e) => m.error = Some(e),
        }
    }
}

/// Snapshot one job (detail view: with output tail).
///
/// # Errors
/// [`Error::Config`] when the id is unknown (GC'd or never existed).
pub fn status(id: &str) -> Result<JobInfo> {
    let mut reg = lock_reg();
    gc(&mut reg);
    let slot = reg
        .get(id)
        .cloned()
        .ok_or_else(|| Error::Config(format!("unknown job id: {id}")))?;
    Ok(snapshot(id, &slot, true))
}

fn snapshot(id: &str, slot: &Arc<JobSlot>, with_output: bool) -> JobInfo {
    let m = slot.meta();
    let (total, tail) = slot.buf.snapshot();
    JobInfo {
        id: id.to_string(),
        command: m.command,
        namespace: m.namespace,
        started_unix: m.started_unix,
        finished_unix: m.finished_unix,
        running: m.running,
        interrupted: m.interrupted,
        error: m.error,
        prompt: m.prompt,
        // live buffer values — JobMeta carries no streaming output fields
        output_bytes: total,
        output: if with_output { tail } else { String::new() },
        final_result: m.final_result,
    }
}

/// Trip a running job's cancel flag (its next frame poll sends the protocol
/// interrupt). Already-finished jobs are returned untouched.
///
/// # Errors
/// [`Error::Config`] when the id is unknown.
pub fn cancel(id: &str) -> Result<JobInfo> {
    let mut reg = lock_reg();
    gc(&mut reg);
    let slot = reg
        .get(id)
        .cloned()
        .ok_or_else(|| Error::Config(format!("unknown job id: {id}")))?;
    if slot.meta().running {
        slot.cancel.store(true, Ordering::Relaxed);
    }
    Ok(snapshot(id, &slot, false))
}

/// Drop finished jobs past retention. Caller holds the lock.
fn gc(reg: &mut Registry) {
    let now = unix_now();
    reg.retain(|_, s| match s.meta().finished_unix {
        None => true,
        Some(t) => now.saturating_sub(t) < RETENTION.as_secs(),
    });
    if reg.len() > MAX_ENTRIES {
        let mut finished: Vec<(u64, String)> = reg
            .iter()
            .filter_map(|(k, s)| s.meta().finished_unix.map(|t| (t, k.clone())))
            .collect();
        finished.sort();
        for (_, id) in finished.into_iter().take(reg.len() - MAX_ENTRIES) {
            reg.remove(&id);
        }
    }
}

// ── tool-facing API ──────────────────────────────────────────────────────
// Background execution is exposed through MCP Tasks (SEP-2663), not extra
// tools: `execute_command(background=true)` materializes a task from this
// registry via `start`, `tasks/get` projects `status`, `tasks/cancel` calls
// `cancel`. The registry itself is transport-agnostic.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    /// Registry-touching tests are multi-operation sequences (clear,
    /// insert, act); OPLOCK only serializes single ops. This outer lock
    /// makes each test's whole body atomic against other tests sharing
    /// the process-global registry.
    static TESTLOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        TESTLOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn clear() {
        lock_reg().clear();
    }

    fn insert(id: &str, running: bool, finished: Option<u64>) -> Arc<JobSlot> {
        let slot = Arc::new(JobSlot {
            meta: Mutex::new(JobMeta {
                command: "write 1".into(),
                namespace: "USER".into(),
                started_unix: unix_now(),
                finished_unix: finished,
                running,
                interrupted: false,
                error: None,
                prompt: None,
                final_result: None,
            }),
            buf: SharedBuf::new(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        lock_reg().insert(id.to_string(), slot.clone());
        slot
    }

    #[test]
    fn finish_on_drop_guard_finalizes_panicking_task() {
        let _s = serial();
        // A job task that unwinds before finish() must still land on a
        // terminal state, or the slot is a running=true zombie forever
        // (GC keeps it, the 16-cap counts it, cancel has no poller).
        clear();
        let slot = insert("zombie-x", true, None);
        let guard_slot = slot.clone();
        let _ = std::panic::catch_unwind(move || {
            let mut guard = FinishOnDrop {
                slot: guard_slot,
                done: false,
            };
            // simulate the panic BEFORE finish() runs:
            let _ = &mut guard;
            panic!("boom");
        });
        let m = slot.meta();
        assert!(!m.running, "guard must finalize the slot on unwind");
        assert!(m.finished_unix.is_some());
        assert!(m.error.as_deref().unwrap_or_default().contains("panicked"));
    }

    #[test]
    fn gc_evicts_oldest_beyond_entry_cap() {
        let _s = serial();
        clear();
        let mut reg = lock_reg();
        for i in 0..(MAX_ENTRIES + 40) {
            let slot = Arc::new(JobSlot {
                meta: Mutex::new(JobMeta {
                    command: "w 1".into(),
                    namespace: "USER".into(),
                    // i=0 oldest, i=N newest — inside RETENTION for all
                    started_unix: unix_now() - u64::try_from(MAX_ENTRIES + 40 - i).unwrap_or(0) - 2,
                    finished_unix: Some(
                        unix_now() - u64::try_from(MAX_ENTRIES + 39 - i).unwrap_or(0),
                    ),
                    running: false,
                    interrupted: false,
                    error: None,
                    prompt: None,
                    final_result: None,
                }),
                buf: SharedBuf::new(),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            reg.insert(format!("cap-{i:05}"), slot);
        }
        // one running job must never be evicted by the cap
        reg.insert(
            "keep-running".to_string(),
            Arc::new(JobSlot {
                meta: Mutex::new(JobMeta {
                    command: "w 1".into(),
                    namespace: "USER".into(),
                    started_unix: unix_now(),
                    finished_unix: None,
                    running: true,
                    interrupted: false,
                    error: None,
                    prompt: None,
                    final_result: None,
                }),
                buf: SharedBuf::new(),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }),
        );
        gc(&mut reg);
        assert!(reg.len() <= MAX_ENTRIES, "cap enforced: {}", reg.len());
        assert!(
            reg.contains_key("keep-running"),
            "running job never evicted"
        );
        assert!(
            reg.contains_key(&format!("cap-{:05}", MAX_ENTRIES + 39)),
            "newest finished survives"
        );
        assert!(
            reg.len() >= MAX_ENTRIES - 1,
            "eviction is bounded: {}",
            reg.len()
        );
        assert!(!reg.contains_key("cap-00000"), "oldest finished evicted");
    }

    #[test]
    fn ids_unique_and_shaped() {
        let _s = serial();
        // SEP-2663 entropy MUST: shape `rism-{pid:x}-{16 hex random}` and
        // 1000 draws all distinct (with seq-based ids this loop was the
        // bug: enumeration by construction).
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = new_id();
            // no expect() (repo denies clippy::expect_used): a shape break
            // must fail the assert, not panic outside it.
            let ok = match id.strip_prefix("rism-").and_then(|r| r.split_once('-')) {
                Some((pid, rand)) => {
                    !pid.is_empty()
                        && pid.chars().all(|c| c.is_ascii_hexdigit())
                        && rand.len() == 16
                        && rand.chars().all(|c| c.is_ascii_hexdigit())
                }
                None => false,
            };
            assert!(ok, "id shape must be rism-{{pid:x}}-{{16 hex}}: {id}");
            assert!(ids.insert(id.clone()), "duplicate task id: {id}");
        }
    }

    #[test]
    fn gc_keeps_running_and_fresh_drops_stale() {
        let _s = serial();
        let mut reg = Registry::new();
        let mk = |id: &str, running: bool, finished: Option<u64>, started: u64| {
            (
                id.to_string(),
                Arc::new(JobSlot {
                    meta: Mutex::new(JobMeta {
                        command: String::new(),
                        namespace: "USER".into(),
                        started_unix: started,
                        finished_unix: finished,
                        running,
                        interrupted: false,
                        error: None,
                        prompt: None,
                        final_result: None,
                    }),
                    buf: SharedBuf::new(),
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                }),
            )
        };
        reg.extend([
            mk("a", true, None, 0),
            mk("b", false, Some(unix_now()), 0),
            mk("c", false, Some(unix_now().saturating_sub(40 * 60)), 0),
        ]);
        gc(&mut reg);
        assert!(reg.contains_key("a") && reg.contains_key("b"));
        assert!(!reg.contains_key("c"), "stale finished job must be dropped");
    }

    /// Round-5 pin for the retention clock's two halves (the `ttlMs`
    /// divergence is documented in docs/mcp-tools.md, not code):
    /// - MUST half (SEP-2663): a task is retrievable at least until
    ///   `createdAt + ttlMs`. Slot `lower` finished 29 min ago and started
    ///   29 min + 1 s ago: BOTH clocks say keep, but the assert guards the
    ///   creation-time floor against a future gc that forgets running-
    ///   length jobs (a `started`-older-than-retention drop here would be
    ///   the spec-MUST breach of early expiry).
    /// - Upper half (documented divergence): retention is measured from
    ///   FINISH — slot `late` started 31 min ago, finished 1 s ago: an
    ///   exact-advertise gc would drop it; the shipped contract keeps it.
    /// - The long-running half: `runner` is 31 min past creation and still
    ///   running — gc never orphans it (bounded by its own timeout).
    #[test]
    fn gc_retention_clock_pins_both_halves() {
        let _s = serial();
        let mut reg = Registry::new();
        let now = unix_now();
        let mk = |id: &str, running: bool, started: u64, finished: Option<u64>| {
            (
                id.to_string(),
                Arc::new(JobSlot {
                    meta: Mutex::new(JobMeta {
                        command: String::new(),
                        namespace: "USER".into(),
                        started_unix: started,
                        finished_unix: finished,
                        running,
                        interrupted: false,
                        error: None,
                        prompt: None,
                        final_result: None,
                    }),
                    buf: SharedBuf::new(),
                    cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                }),
            )
        };
        reg.extend([
            mk("lower", false, now - 29 * 60, Some(now - 29 * 60 + 1)),
            mk("late", false, now - 31 * 60, Some(now - 1)),
            mk("runner", true, now - 31 * 60, None),
        ]);
        gc(&mut reg);
        assert!(
            reg.contains_key("lower"),
            "createdAt+ttlMs lower bound must never expire early"
        );
        assert!(
            reg.contains_key("late"),
            "retention is finish-based by documented contract: late must survive"
        );
        assert!(
            reg.contains_key("runner"),
            "a running task past the advertised TTL is never orphaned"
        );
    }

    #[test]
    fn sharedbuf_keeps_tail_and_total() {
        let _s = serial();
        let buf = SharedBuf::new();
        for _ in 0..(OUTPUT_CAP / 50 + 10) {
            buf.push(&"x".repeat(50));
        }
        let (total, tail) = buf.snapshot();
        assert_eq!(total, (OUTPUT_CAP / 50 + 10) * 50);
        assert_eq!(tail.len(), OUTPUT_CAP, "tail capped exactly");
    }

    #[test]
    fn ring_buffer_multibyte_straddle_no_panic() {
        let _s = serial();
        let buf = SharedBuf::new();
        // 2-byte chars around the cap boundary: the trim must floor to a
        // char boundary, never panic the job task (which would leave
        // running=true forever in meta)
        for _ in 0..(OUTPUT_CAP / 4 + 2) {
            buf.push("éééé");
        }
        let (total, tail) = buf.snapshot();
        assert_eq!(total, (OUTPUT_CAP / 4 + 2) * 8);
        assert!(tail.len() <= OUTPUT_CAP + 1); // floor: at most 1 straddle byte
        assert!(tail.is_char_boundary(0) && tail.is_char_boundary(tail.len()));
        assert!(tail.chars().all(|c| c == 'é')); // still valid UTF-8
    }

    #[test]
    fn cancel_trips_flag_and_status_reports() {
        let _s = serial();
        clear();
        let slot = insert("job-x", true, None);
        let info = cancel("job-x").unwrap();
        assert!(info.running, "cancel returns the pre-finish snapshot");
        assert!(slot.cancel.load(Ordering::Relaxed));
        assert!(info.output.is_empty(), "cancel is a list-view snapshot");

        // finish it, then status shows the tail
        slot.buf.push("hello");
        finish(
            &slot,
            Ok(api::TerminalOutcome {
                output: String::new(),
                prompt: "USER>".into(),
                truncated: false,
                omitted_chars: 0,
                interrupted: true,
            }),
        );
        let s = status("job-x").unwrap();
        assert!(!s.running);
        assert!(s.interrupted);
        assert!(s.error.is_none());
        assert_eq!(s.prompt.as_deref(), Some("USER>"));
        assert_eq!(s.output, "hello");
        assert_eq!(s.output_bytes, 5);

        assert!(status("nope").is_err());
        assert!(cancel("nope").is_err());
    }
}
