//! Host-side tools: [`run_shell`], [`read_file`], [`list_files`].
//! These act on the machine Rism runs on — NOT the IRIS server (for that,
//! [`execute_command`] / documents. Semantics mirror Prism's `shell.py`/`files.py`
//! (output caps, binary detection, traversal guard, truncation notices).
//! Host-local by design: these never touch IRIS.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{Error, Result};
use crate::iris::IrisClient;

const MAX_OUTPUT_CHARS: usize = 10_000;
const MAX_FILE_CHARS: usize = 100_000;
const DEFAULT_SHELL_TIMEOUT_SECS: u64 = 30;
const MAX_SHELL_TIMEOUT_SECS: u64 = 3600;

/// Arguments for [`run_shell`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunShellArgs {
    /// Shell command (`PowerShell` on Windows, Bash elsewhere)
    pub command: String,
    /// Timeout seconds (default 30, max 3600)
    pub timeout_secs: Option<u64>,
    /// Working directory (default: workspace root or cwd)
    pub cwd: Option<String>,
}

/// Result of [`run_shell`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ShellResult {
    /// Captured stdout (truncated at 10k chars with notice).
    pub stdout: String,
    /// Captured stderr (truncated likewise).
    pub stderr: String,
    /// Process exit code; -1 when killed by timeout.
    pub exit_code: i32,
    /// Working directory actually used.
    pub cwd: String,
}

/// Run a shell command on the LOCAL host (never the IRIS server).
///
/// # Errors
/// [`Error::Io`] on spawn failure (a non-zero exit is a RESULT, not an error).
pub async fn run_shell(client: &IrisClient, args: &RunShellArgs) -> Result<ShellResult> {
    let timeout = args
        .timeout_secs
        .unwrap_or(DEFAULT_SHELL_TIMEOUT_SECS)
        .clamp(1, MAX_SHELL_TIMEOUT_SECS);

    let cwd = match &args.cwd {
        Some(c) => PathBuf::from(c),
        None => default_cwd(client),
    };

    #[cfg(windows)]
    let (prog, prefix): (&str, &[&str]) =
        ("powershell.exe", &["-NoProfile", "-NoLogo", "-Command"]);
    #[cfg(not(windows))]
    let (prog, prefix): (&str, &[&str]) = ("/bin/bash", &["-c"]);

    let child = Command::new(prog)
        .args(prefix)
        .arg(&args.command)
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Io(std::io::Error::new(e.kind(), format!("spawn {prog}: {e}"))))?;

    let outcome = tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output())
        .await
        .map_err(|_| {
            // wait_for + kill semantics (Prism parity): timed-out commands
            // ARE killed; nothing else can cap wall-clock.
            Error::Io(std::io::Error::other(format!(
                "Command timed out after {timeout}s and was killed"
            )))
        })?;

    let out = outcome?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok(ShellResult {
        stdout: truncate_at(MAX_OUTPUT_CHARS, stdout, "stdout"),
        stderr: truncate_at(MAX_OUTPUT_CHARS, stderr, "stderr"),
        exit_code: out.status.code().unwrap_or(-1),
        cwd: cwd.display().to_string(),
    })
}

fn default_cwd(client: &IrisClient) -> PathBuf {
    workspace_root(client)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Arguments for [`read_file`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFileArgs {
    /// Path relative to the workspace root (traversal blocked)
    pub path: String,
}

/// Result of [`read_file`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReadFileResult {
    /// File content (truncated at 100k chars on a line boundary).
    pub content: String,
    /// Workspace-relative path echoed.
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// True when content was cut.
    pub truncated: bool,
    /// Truncation notice.
    pub truncation_message: Option<String>,
    /// Set instead of hard-failing (matches Prism's tool-level errors).
    pub error: Option<String>,
}

/// Read a text file from the local workspace.
///
/// # Errors
/// [`Error::Config`] when no workspace root is configured.
pub async fn read_file(client: &IrisClient, args: &ReadFileArgs) -> Result<ReadFileResult> {
    let fail = |msg: String| {
        Ok(ReadFileResult {
            content: String::new(),
            path: args.path.clone(),
            size: 0,
            truncated: false,
            truncation_message: None,
            error: Some(msg),
        })
    };
    let Some(root) = workspace_root(client) else {
        return fail(
            "RISM_WORKSPACE is not configured. Set it to enable file reading.".to_string(),
        );
    };
    let resolved = match resolve_safe(&root, &args.path) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    if !resolved.is_file() {
        return fail(format!("File not found in workspace: {}", args.path));
    }
    let bytes = tokio::fs::read(&resolved).await?;
    if is_binary(&bytes) {
        return fail(format!(
            "Binary file not supported: {} (use terminal/execute_command or read on the IRIS server via get_document for .cls docs)",
            args.path
        ));
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let (content, truncated, message) = truncate_file(text);
    Ok(ReadFileResult {
        content,
        path: args.path.clone(),
        size: bytes.len() as u64,
        truncated,
        truncation_message: message,
        error: None,
    })
}

/// Arguments for [`list_files`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListFilesArgs {
    /// Directory within the workspace (default root)
    pub path: Option<String>,
    /// Glob filter (e.g. `*.cls`, `**/*.json`)
    pub pattern: Option<String>,
    /// Max results (default 200, max 1000)
    pub max_results: Option<u32>,
}

/// One directory entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FileEntry {
    /// Base name.
    pub name: String,
    /// Workspace-relative path (forward slashes).
    pub path: String,
    /// Directory flag.
    pub is_dir: bool,
    /// Size in bytes (0 for dirs).
    pub size: u64,
}

/// Result of [`list_files`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListFilesResult {
    /// Entries.
    pub files: Vec<FileEntry>,
    /// Path listed.
    pub path: String,
    /// Count.
    pub count: usize,
    /// True when `max_results` cut the list.
    pub truncated: bool,
    /// Tool-level error text.
    pub error: Option<String>,
}

/// List files in the local workspace.
///
/// # Errors
/// Never (errors come back in `.error`, Prism parity).
pub async fn list_files(client: &IrisClient, args: &ListFilesArgs) -> Result<ListFilesResult> {
    let empty = |msg: String| ListFilesResult {
        files: Vec::new(),
        path: args.path.clone().unwrap_or_default(),
        count: 0,
        truncated: false,
        error: Some(msg),
    };
    let Some(root) = workspace_root(client) else {
        return Ok(empty("RISM_WORKSPACE is not configured.".to_string()));
    };
    let target = match &args.path {
        Some(p) => match resolve_safe(&root, p) {
            Ok(t) => t,
            Err(e) => return Ok(empty(e)),
        },
        None => root.clone(),
    };
    if !target.exists() {
        return Ok(empty(format!(
            "Path not found: {}",
            args.path.as_deref().unwrap_or_default()
        )));
    }
    if !target.is_dir() {
        return Ok(empty(format!(
            "Not a directory: {}",
            args.path.as_deref().unwrap_or_default()
        )));
    }

    let max = args.max_results.unwrap_or(200).clamp(1, 1000) as usize;
    let mut files: Vec<FileEntry> = Vec::new();
    let mut truncated = false;

    if let Some(pattern) = &args.pattern {
        let full = if pattern.contains("**") {
            pattern.clone()
        } else {
            format!("{}/**/{pattern}", strip_absolute(pattern))
        };
        let mut paths: Vec<PathBuf> = glob::glob(&target.join(full).display().to_string())
            .map_err(|e| Error::Config(format!("bad glob pattern: {e}")))?
            .filter_map(std::result::Result::ok)
            .collect();
        paths.sort();
        for p in paths {
            if files.len() >= max {
                truncated = true;
                break;
            }
            if let Some(entry) = file_entry(&root, &p) {
                files.push(entry);
            }
        }
    } else {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&target)?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for p in dirs {
            if files.len() >= max {
                truncated = true;
                break;
            }
            if let Some(entry) = file_entry(&root, &p) {
                files.push(entry);
            }
        }
    }

    let count = files.len();
    Ok(ListFilesResult {
        files,
        path: args.path.clone().unwrap_or_else(|| ".".to_string()),
        count,
        truncated,
        error: None,
    })
}

fn strip_absolute(pattern: &str) -> String {
    pattern.trim_start_matches('/').to_string()
}

fn file_entry(root: &Path, p: &Path) -> Option<FileEntry> {
    let rel = p.strip_prefix(root).ok()?;
    let meta = std::fs::metadata(p).ok()?;
    Some(FileEntry {
        name: p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        path: rel.to_string_lossy().replace('\\', "/"),
        is_dir: meta.is_dir(),
        size: if meta.is_file() { meta.len() } else { 0 },
    })
}

/// Workspace root from settings (None when unset).
fn workspace_root(client: &IrisClient) -> Option<PathBuf> {
    let raw = client.settings().workspace_root;
    if raw.trim().is_empty() {
        return None;
    }
    canonical_lenient(Path::new(raw.trim()))
}

/// Canonicalize, falling back to normalising away `.`/`..` components when
/// the path does not exist yet or is uncanonicalizable.
fn canonical_lenient(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Some(c);
    }
    // lexical: collapse . and .. against a rooted path
    if !p.is_absolute() {
        return std::env::current_dir()
            .ok()
            .map(|cwd| collapse(&cwd.join(p)));
    }
    Some(collapse(p))
}

fn collapse(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Join + resolve inside root, blocking traversal (Prism `resolve_safe`).
fn resolve_safe(root: &Path, relative: &str) -> std::result::Result<PathBuf, String> {
    let resolved = canonical_lenient(&root.join(relative))
        .ok_or_else(|| "path resolution failed".to_string())?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "Path escapes workspace: {relative:?} resolves to {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn is_binary(bytes: &[u8]) -> bool {
    let chunk = &bytes[..bytes.len().min(8192)];
    if chunk.contains(&0) {
        return true;
    }
    if chunk.is_empty() {
        return false;
    }
    let control = chunk
        .iter()
        .filter(|b| **b < 32 && !matches!(**b, 9 | 10 | 13))
        .count();
    // Sample is <= 8192 bytes by construction: casts are exact.
    #[allow(clippy::cast_precision_loss)]
    let ratio = control as f64 / chunk.len() as f64;
    ratio > 0.20
}

fn truncate_file(text: String) -> (String, bool, Option<String>) {
    if text.len() <= MAX_FILE_CHARS {
        return (text, false, None);
    }
    let cut = text[..MAX_FILE_CHARS]
        .rfind('\n')
        .filter(|c| *c > MAX_FILE_CHARS / 2)
        .unwrap_or(MAX_FILE_CHARS);
    let msg = format!("[... file truncated, {} more chars]", text.len() - cut);
    (text[..cut].to_string(), true, Some(msg))
}

fn truncate_at(limit: usize, text: String, what: &str) -> String {
    if text.len() <= limit {
        return text;
    }
    format!(
        "{}\n[... {what} truncated, {} more chars]",
        &text[..limit],
        text.len() - limit
    )
}

/// Write text to a local path (helper for `rism doc get --save`).
///
/// # Errors
/// Io errors on write.
pub async fn save_local(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let mut f = tokio::fs::File::create(path).await?;
    f.write_all(text.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn traversal_guard() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(resolve_safe(&root, "normal/file.txt").is_ok());
        assert!(resolve_safe(&root, "../../etc/passwd").is_err());
    }

    #[test]
    fn binary_detection() {
        assert!(is_binary(b"abc\x00def"));
        assert!(!is_binary(b"normal text \xc3\xa9 with utf8"));
        let junk: Vec<u8> = (0..100u8).map(|i| i % 8).collect(); // mostly control
        assert!(is_binary(&junk));
    }

    #[test]
    fn file_truncation_line_boundary() {
        let text = "x".repeat(MAX_FILE_CHARS + 100) + "\ny";
        let (cut, truncated, msg) = truncate_file(text);
        assert!(truncated && msg.unwrap().contains("more chars") && cut.len() <= MAX_FILE_CHARS);
    }
}
