//! Interactive `rism exec` REPL: persistent terminal session + rustyline
//! line editing (history, Ctrl+R reverse search, Ctrl+D exit) over the
//! [`crate::iris::terminal`] WebSocket.
//!
//! Two traits split the problem so each half is testable alone:
//! [`TerminalOps`] (transport; real impl `TerminalSession`) and [`ReplIo`]
//! (editing; real impls [`rusty::EditorIo`] / [`rusty::PipeIo`]).
//! [`run_repl`] is the pure state machine between them.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::error::Result;
use crate::iris::terminal as api;

/// Streaming context for one REPL command (built by [`run_repl`]).
pub struct ReplHooks<'a, Io: ReplIo + ?Sized> {
    io: &'a mut Io,
    cancel: Arc<AtomicBool>,
    pub(crate) streamed: bool,
    pub(crate) ends_nl: bool,
}

impl<Io: ReplIo + ?Sized> api::StreamHooks for ReplHooks<'_, Io> {
    fn emit(&mut self, text: &str) -> Result<()> {
        self.streamed |= !text.is_empty();
        self.ends_nl = text.ends_with('\n');
        self.io.emit(text)
    }
    fn read_prompt(&mut self) -> Result<String> {
        self.io.read_answer()
    }
    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
}

/// Everything the REPL needs from the transport side (real impl:
/// `api::TerminalSession`; tests script a fake).
pub trait TerminalOps {
    /// Run one command with hooks; returns the post-command prompt and
    /// whether the command was interrupted (Ctrl+C break sent server-side).
    fn run_line(
        &mut self,
        command: &str,
        hooks: &mut dyn api::StreamHooks,
    ) -> impl std::future::Future<Output = Result<(String, bool)>>;

    /// Swallow late frames after an abandoned command.
    fn drain_late(&mut self) -> impl std::future::Future<Output = Result<()>>;

    /// The session's current prompt (before the first run).
    fn current_prompt(&self) -> String;
}

impl TerminalOps for api::TerminalSession {
    async fn run_line(
        &mut self,
        command: &str,
        hooks: &mut dyn api::StreamHooks,
    ) -> Result<(String, bool)> {
        let out = self.run_with(command, hooks).await?;
        Ok((out.prompt, out.interrupted))
    }

    async fn drain_late(&mut self) -> Result<()> {
        self.drain().await
    }

    fn current_prompt(&self) -> String {
        self.prompt().to_string()
    }
}

/// Line editor abstraction: lets the loop be tested without a TTY or a
/// server.
pub trait ReplIo {
    /// Next user line (after history/editing). `Ok(None)` on EOF — Ctrl+D,
    /// end of piped stdin, or Ctrl+C at the prompt.
    ///
    /// # Errors
    /// [`Error::Terminal`] on editor/TTY failure (not EOF/cancel paths).
    fn next_line(&mut self, prompt: &str) -> Result<Option<String>>;
    /// Echo a line of session status (stderr; stdout is command data).
    fn notice(&mut self, line: &str);
    /// One output frame from the server, streamed as it arrives.
    ///
    /// # Errors
    /// Any write error aborts the command's streaming (reported to the loop).
    fn emit(&mut self, text: &str) -> Result<()>;
    /// Cancel requested during a running command (Ctrl+C mid-run).
    fn cancelled(&self) -> bool;
    /// Re-arm after handling a cancel.
    fn reset_cancel(&mut self);
    /// The live cancel flag, shared with the transport's frame wait.
    /// Default hands out a dead flag (nothing ever cancels this io).
    fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    /// A running program hit `read`: ask the user (or stdin) one line.
    /// Default refuses politely (empty reply) so nothing hangs.
    ///
    /// # Errors
    /// [`Error::Terminal`] on editor/IO failure.
    fn read_answer(&mut self) -> Result<String> {
        Ok(String::new())
    }
    /// Arm OS-level cancel detection just before a command runs
    /// (default: nothing).
    fn arm_cancel(&mut self) {}
    /// Disarm after the command returns (default: nothing).
    fn disarm_cancel(&mut self) {}
}

/// Session outcome.
#[derive(Debug, Clone)]
pub struct ReplReport {
    /// Commands actually run.
    pub commands: usize,
    /// Why the loop stopped.
    pub reason: ReplExit,
}

/// Why the REPL loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplExit {
    /// EOF (Ctrl+D / Ctrl+C at prompt / piped-stdin end).
    Eof,
    /// `exit` / `quit`.
    Quit,
    /// Transport or input died mid-loop.
    Fatal,
}

/// Run the REPL state machine over any transport/editor pair.
///
/// Empty lines pass straight to IRIS (harmless echo, native terminal
/// behavior) so ↑ history navigation always has content.
///
/// # Errors
/// Only on irrecoverable session loss; per-command errors are reported via
/// [`ReplIo::notice`] and the loop continues.
pub async fn run_repl<S: TerminalOps + ?Sized, Io: ReplIo + ?Sized>(
    session: &mut S,
    io: &mut Io,
) -> Result<ReplReport> {
    let mut commands = 0usize;
    let mut prompt = session.current_prompt();
    let reason = loop {
        let input = match io.next_line(&prompt) {
            Ok(Some(line)) => line,
            Ok(None) => break ReplExit::Eof,
            Err(e) => {
                io.notice(&format!("input error: {e}"));
                break ReplExit::Fatal;
            }
        };
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            break ReplExit::Quit;
        }
        io.reset_cancel(); // fresh start: watcher is not yet running
        io.arm_cancel();
        let io_cancel = io.cancel_handle();
        let mut hooks = ReplHooks {
            io,
            cancel: io_cancel,
            streamed: false,
            ends_nl: true,
        };
        let ran = session.run_line(&input, &mut hooks).await;
        // destructure to end the borrow of io held by the hooks
        let ReplHooks {
            io,
            streamed,
            ends_nl,
            cancel,
            ..
        } = hooks;
        drop(cancel);
        let was_cancelled = io.cancelled();
        io.disarm_cancel();
        // Always clear: a Ctrl+C that raced the prompt (set after the last
        // poll, before disarm) must not leak into command N+1.
        io.reset_cancel();
        match ran {
            Ok((p, interrupted)) => {
                if streamed && !ends_nl && !interrupted {
                    // IRIS frames carry no trailing newline; without this the
                    // next command's output glues onto this one's last frame.
                    let _ = io.emit("\n"); // a broken pipe surfaces next line anyway
                }
                if interrupted {
                    io.notice("<INTERRUPT> abandoned; session clean");
                }
                commands += 1;
                prompt = p;
            }
            Err(e) => {
                commands += 1;
                // Timeout or transport error: the server may still emit late
                // frames — drain to the next prompt before touching the
                // session again, or the next command reads this one's
                // leftovers.
                if was_cancelled {
                    io.notice("^C abandoned; session state may be partial");
                } else {
                    io.notice(&format!("error: {e}"));
                }
                // keep cancel detection alive during the drain: this is
                // exactly when the user wants to bail, and the watcher is
                // otherwise down until the next arm (default SIGINT would
                // kill us before session.close() runs)
                io.arm_cancel();
                let drained = session.drain_late().await;
                io.disarm_cancel();
                match drained {
                    Ok(()) => prompt = session.current_prompt(),
                    Err(e) => {
                        io.notice(&format!("session lost: {e}"));
                        break ReplExit::Fatal;
                    }
                }
            }
        }
    };
    Ok(ReplReport { commands, reason })
}

/// Production [`ReplIo`] implementations.
pub mod rusty {
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use rustyline::error::ReadlineError;
    use rustyline::history::FileHistory;
    use rustyline::{CompletionType, Config, Editor};

    use super::ReplIo;
    use crate::error::{Error, Result};

    /// Persistent history file: `<config dir>/rism/terminal_history.txt`.
    #[must_use]
    pub fn history_path() -> Option<PathBuf> {
        directories::BaseDirs::new()
            .map(|d| d.config_dir().join("rism").join("terminal_history.txt"))
    }

    fn config() -> Config {
        // history_ignore_dups is ON by default (dedup = what we want).
        Config::builder()
            .auto_add_history(true)
            .history_ignore_space(true)
            .completion_type(CompletionType::List)
            .bell_style(rustyline::config::BellStyle::None)
            .build()
    }

    /// TTY editor: history ↑/↓, Ctrl+R reverse search, Ctrl+D exit.
    pub struct EditorIo {
        ed: Editor<(), FileHistory>,
        cancel: Arc<AtomicBool>,
        watcher: Option<tokio::task::JoinHandle<()>>,
    }

    impl EditorIo {
        /// Build with persistent history; unreadable history degrades to
        /// empty (never blocks the REPL from starting).
        ///
        /// # Errors
        /// [`Error::Terminal`] if the editor cannot take over the TTY.
        pub fn new() -> Result<Self> {
            let cancel = Arc::new(AtomicBool::new(false));
            let mut ed = Editor::with_config(config())
                .map_err(|e| Error::Terminal(format!("editor: {e}")))?;
            if let Some(p) = history_path() {
                let _ = ed.load_history(&p);
            }
            Ok(Self {
                ed,
                cancel,
                watcher: None,
            })
        }
    }

    impl ReplIo for EditorIo {
        fn next_line(&mut self, prompt: &str) -> Result<Option<String>> {
            match self.ed.readline(prompt) {
                Ok(line) => Ok(Some(line)),
                // Ctrl+D exits; Ctrl+C at the prompt just discards the line
                // (bash semantics) — empty input is a harmless server echo.
                Err(ReadlineError::Eof) => Ok(None),
                Err(ReadlineError::Interrupted) => Ok(Some(String::new())),
                Err(e) => Err(Error::Terminal(format!("readline: {e}"))),
            }
        }

        fn notice(&mut self, line: &str) {
            eprintln!("{line}");
        }

        fn emit(&mut self, text: &str) -> Result<()> {
            // After Ctrl+C: suppress late output, bash-style.
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let mut out = std::io::stdout();
            out.write_all(text.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| Error::Terminal(format!("stdout: {e}")))
        }

        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::Relaxed)
        }

        fn reset_cancel(&mut self) {
            self.cancel.store(false, Ordering::Relaxed);
        }

        fn cancel_handle(&self) -> Arc<AtomicBool> {
            self.cancel.clone()
        }

        fn read_answer(&mut self) -> Result<String> {
            // A program hit `read`: reuse the editor (no prompt echo needed —
            // the program already printed its own). rustyline re-arms the
            // SIGINT handler for us, so Ctrl+C still cancels during a read.
            match self.ed.readline("") {
                Ok(line) => Ok(line),
                Err(ReadlineError::Eof | ReadlineError::Interrupted) => Ok(String::new()),
                Err(e) => Err(Error::Terminal(format!("readline: {e}"))),
            }
        }

        fn arm_cancel(&mut self) {
            // rustyline releases SIGINT once readline returns; without this
            // a Ctrl+C during command execution would kill the process.
            if self
                .watcher
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
            {
                let flag = self.cancel.clone();
                self.watcher = Some(tokio::spawn(async move {
                    while tokio::signal::ctrl_c().await.is_ok() {
                        flag.store(true, Ordering::Relaxed);
                    }
                }));
            }
        }

        fn disarm_cancel(&mut self) {
            if let Some(w) = self.watcher.take() {
                w.abort();
            }
        }
    }

    impl Drop for EditorIo {
        fn drop(&mut self) {
            if let Some(p) = history_path() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = self.ed.save_history(&p);
            }
        }
    }

    /// Piped-stdin mode: same loop, no TTY — editing/history skipped,
    /// which is correct for CI and `echo … | rism exec`.
    pub struct PipeIo;

    impl ReplIo for PipeIo {
        fn next_line(&mut self, _prompt: &str) -> Result<Option<String>> {
            use std::io::BufRead as _;
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            match lock.read_line(&mut line) {
                Ok(0) => Ok(None),
                Ok(_) => Ok(Some(line.trim_end_matches(['\r', '\n']).to_string())),
                Err(e) => Err(Error::Terminal(format!("stdin: {e}"))),
            }
        }

        fn notice(&mut self, line: &str) {
            eprintln!("{line}");
        }

        fn emit(&mut self, text: &str) -> Result<()> {
            let mut out = std::io::stdout();
            out.write_all(text.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|e| Error::Terminal(format!("stdout: {e}")))
        }

        fn cancelled(&self) -> bool {
            false
        }

        fn reset_cancel(&mut self) {}

        fn read_answer(&mut self) -> Result<String> {
            use std::io::BufRead as _;
            // A server-side read consumes the next piped line (script mode);
            // EOF answers empty so nothing ever hangs a stream.
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            match lock.read_line(&mut line) {
                Ok(0) | Err(_) => Ok(String::new()),
                Ok(_) => Ok(line.trim_end_matches(['\r', '\n']).to_string()),
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_panics_doc,
    // fakes resolve instantly; `unused_async` was renamed to
    // `unused_async_trait_impl` in 1.99 nightlies — silence whichever
    // name this toolchain uses without warning about the other
    unknown_lints,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
mod tests {
    use super::*;
    use crate::error::Error;

    struct FakeSession {
        lines_seen: Vec<String>,
        prompts: Vec<String>,
        fail_on: Option<String>,
        /// command during which a Ctrl+C "arrives" (flag set mid-run, like
        /// the real watcher racing the transport's last poll)
        cancel_during: Option<String>,
        drained: usize,
    }

    impl FakeSession {
        fn ok(prompts: &[&str]) -> Self {
            Self {
                lines_seen: Vec::new(),
                prompts: prompts.iter().map(|s| (*s).to_string()).collect(),
                fail_on: None,
                cancel_during: None,
                drained: 0,
            }
        }
    }

    impl TerminalOps for FakeSession {
        async fn run_line(
            &mut self,
            command: &str,
            hooks: &mut dyn api::StreamHooks,
        ) -> Result<(String, bool)> {
            self.lines_seen.push(command.to_string());
            if self.cancel_during.as_deref() == Some(command) {
                hooks
                    .cancel_flag()
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if self.fail_on.as_deref() == Some(command) {
                return Err(Error::Terminal("boom".into()));
            }
            // the real transport polls the cancel flag while waiting for
            // frames; the fake honors it the same way, checked at entry
            if hooks
                .cancel_flag()
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                hooks.emit("\n")?;
                let prompt = self.prompts.remove(0);
                return Ok((prompt, true));
            }
            hooks.emit("out:")?;
            hooks.emit(command)?;
            hooks.emit("\n")?;
            Ok((self.prompts.remove(0), false))
        }

        async fn drain_late(&mut self) -> Result<()> {
            self.drained += 1;
            Ok(())
        }

        fn current_prompt(&self) -> String {
            // before the first run, the session shows its launch prompt
            self.prompts.first().cloned().unwrap_or_default()
        }
    }

    struct FakeIo {
        lines: Vec<Option<String>>,
        idx: usize,
        emitted: String,
        notices: Vec<String>,
        cancel: std::sync::Arc<AtomicBool>,
    }

    impl FakeIo {
        fn new(lines: Vec<Option<String>>) -> Self {
            Self {
                lines,
                idx: 0,
                emitted: String::new(),
                notices: Vec::new(),
                cancel: std::sync::Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl ReplIo for FakeIo {
        fn next_line(&mut self, _prompt: &str) -> Result<Option<String>> {
            let v = self.lines.get(self.idx).cloned().unwrap_or(None);
            self.idx += 1;
            Ok(v)
        }
        fn notice(&mut self, line: &str) {
            self.notices.push(line.to_string());
        }
        fn emit(&mut self, text: &str) -> Result<()> {
            self.emitted.push_str(text);
            Ok(())
        }
        fn cancelled(&self) -> bool {
            self.cancel.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn reset_cancel(&mut self) {
            self.cancel
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        fn cancel_handle(&self) -> std::sync::Arc<AtomicBool> {
            self.cancel.clone()
        }
    }

    #[tokio::test]
    async fn repl_streams_and_exits_on_eof() {
        let mut s = FakeSession::ok(&["USER>", "SET>"]);
        let mut io = FakeIo::new(vec![Some("write 1".into()), Some("set x=1".into()), None]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.reason, ReplExit::Eof);
        assert_eq!(r.commands, 2);
        assert_eq!(s.lines_seen, vec!["write 1", "set x=1"]);
        assert!(io.emitted.contains("out:write 1"));
    }

    #[tokio::test]
    async fn prompt_threads_to_editor() {
        struct PromptCatcher {
            seen: Vec<String>,
            lines: Vec<Option<String>>,
        }
        impl ReplIo for PromptCatcher {
            fn next_line(&mut self, prompt: &str) -> Result<Option<String>> {
                self.seen.push(prompt.to_string());
                let i = self.seen.len() - 1;
                Ok(self.lines.get(i).cloned().unwrap_or(None))
            }
            fn notice(&mut self, _: &str) {}
            fn emit(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
            fn cancelled(&self) -> bool {
                false
            }
            fn reset_cancel(&mut self) {}
        }
        let mut s = FakeSession::ok(&["A>", "B>"]);
        let mut io = PromptCatcher {
            seen: Vec::new(),
            lines: vec![Some("c1".into()), Some("c2".into()), None],
        };
        run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(io.seen, vec!["A>", "A>", "B>"]);
    }

    #[tokio::test]
    async fn exit_command_stops_without_sending() {
        let mut s = FakeSession::ok(&["USER>"]);
        let mut io = FakeIo::new(vec![Some("  exit  ".into())]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.reason, ReplExit::Quit);
        assert_eq!(r.commands, 0);
        assert!(s.lines_seen.is_empty(), "exit must never reach the server");
    }

    #[tokio::test]
    async fn error_notices_do_not_stop_the_loop() {
        let mut s = FakeSession::ok(&["USER>", "SET>"]);
        s.fail_on = Some("bad".into());
        let mut io = FakeIo::new(vec![Some("bad".into()), Some("ok".into()), None]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.reason, ReplExit::Eof);
        assert_eq!(r.commands, 2, "failed command still counted");
        assert!(io.notices.iter().any(|n| n.contains("boom")));
    }

    #[tokio::test]
    async fn cancel_drains_and_continues() {
        let mut s = FakeSession::ok(&["SET>", "USER>"]);
        s.fail_on = Some("hang".into());
        s.cancel_during = Some("hang".into());
        let mut io = FakeIo::new(vec![Some("hang".into()), Some("recover".into()), None]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.reason, ReplExit::Eof);
        assert_eq!(s.drained, 1);
        assert!(io.notices.iter().any(|n| n.starts_with("^C")));
    }

    #[tokio::test]
    async fn interrupted_outcome_resyncs_without_drain() {
        let mut s = FakeSession::ok(&["SET>", "USER>"]);
        s.cancel_during = Some("slow".into());
        let mut io = FakeIo::new(vec![Some("slow".into()), Some("next".into()), None]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.reason, ReplExit::Eof);
        assert_eq!(r.commands, 2);
        assert_eq!(s.drained, 0, "interrupt path resyncs via prompt, no drain");
        assert!(
            io.notices.iter().any(|n| n.contains("<INTERRUPT>")),
            "user must see the break happened: {:?}",
            io.notices
        );
    }

    #[tokio::test]
    async fn raced_cancel_does_not_abort_next_command() {
        // flag set during command 1 (which completes Ok and unprompted —
        // the disarm race), command 2 must still run clean
        let mut s = FakeSession::ok(&["SET>", "USER>"]);
        s.cancel_during = Some("first".into());
        let mut io = FakeIo::new(vec![Some("first".into()), Some("second".into()), None]);
        let r = run_repl(&mut s, &mut io).await.unwrap();
        assert_eq!(r.commands, 2);
        assert_eq!(r.reason, ReplExit::Eof);
        // command 1 came back interrupted; command 2 must NOT be falsely
        // aborted by the stale flag: it reaches the server and streams
        assert_eq!(s.lines_seen, vec!["first", "second"]);
        assert_eq!(
            io.notices
                .iter()
                .filter(|n| n.contains("<INTERRUPT>"))
                .count(),
            1,
            "exactly one real interrupt: {:?}",
            io.notices
        );
    }

    #[tokio::test]
    async fn fatal_on_input_error() {
        struct BrokenIo;
        impl ReplIo for BrokenIo {
            fn next_line(&mut self, _: &str) -> Result<Option<String>> {
                Err(Error::Terminal("tty gone".into()))
            }
            fn notice(&mut self, _: &str) {}
            fn emit(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
            fn cancelled(&self) -> bool {
                false
            }
            fn reset_cancel(&mut self) {}
        }
        let mut s = FakeSession::ok(&[]);
        let r = run_repl(&mut s, &mut BrokenIo).await.unwrap();
        assert_eq!(r.reason, ReplExit::Fatal);
    }
}
