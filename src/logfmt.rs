//! Prism-parity request/response log formatting (Prism `iris/sdk/log.py`).
//!
//! Output shape mirrors Prism's `logged_tool` banners, e.g.
//! `── put_document ── REQUEST ────────` followed by pretty JSON. Timestamps
//! are NOT embedded: the tracing subscriber's own prefix carries them
//! (idiomatic Rust; documented deviation from Prism's in-message `%H:%M:%S`).
//!
//! Two truncations keep logs readable (ported 1:1):
//! - `content` arrays longer than 10 lines → first 3 + `... (N lines total)`
//!   marker + last 3 (large document uploads).
//! - `output` strings longer than 4000 chars → head 2000 + omitted-chars
//!   marker + tail 1000 (terminal command results).

use serde_json::{Value, json};

const WIDTH: usize = 55;
const MAX_LOG_OUTPUT_CHARS: usize = 4_000;

/// Banner line for a REQUEST (Prism `_WIDTH - len(tool) - 14`).
#[must_use]
pub fn request_banner(tool: &str) -> String {
    banner(tool, "REQUEST", 14)
}

/// Banner line for a RESPONSE (Prism `_WIDTH - len(tool) - 15`).
#[must_use]
pub fn response_banner(tool: &str) -> String {
    banner(tool, "RESPONSE", 15)
}

fn banner(tool: &str, kind: &str, pad: usize) -> String {
    let sep_len = WIDTH.saturating_sub(tool.len() + kind.len() + pad);
    format!("── {tool} ── {kind} {}", "─".repeat(sep_len))
}

/// Pretty JSON (Prism `_pretty`: indent 2, non-ASCII preserved).
#[must_use]
pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// Truncate an `arguments` map for logging (Prism `_truncate_content`).
#[must_use]
pub fn truncate_params(params: &Value) -> Value {
    let Some(map) = params.as_object() else {
        return params.clone();
    };
    let Some(Value::Array(lines)) = map.get("content") else {
        return params.clone();
    };
    if lines.len() <= 10 {
        return params.clone();
    }
    let mut truncated: Vec<Value> = lines[..3].to_vec();
    truncated.push(json!(format!("  ... ({} lines total) ...", lines.len())));
    truncated.extend_from_slice(&lines[lines.len() - 3..]);
    let mut out = map.clone();
    out.insert("content".to_string(), Value::Array(truncated));
    Value::Object(out)
}

/// Truncate a result payload for logging (Prism `_truncate_result`).
/// Summarizes an oversized top-level `output` string.
#[must_use]
pub fn truncate_result(result: &Value) -> Value {
    let Some(map) = result.as_object() else {
        return result.clone();
    };
    let Some(Value::String(output)) = map.get("output") else {
        return result.clone();
    };
    if output.len() <= MAX_LOG_OUTPUT_CHARS {
        return result.clone();
    }
    let head: String = output.chars().take(2000).collect();
    let tail: String = output.chars().skip(output.chars().count() - 1000).collect();
    let omitted = output.chars().count() - 3000;
    let mut out = map.clone();
    // Prism's exact shape: head + "\n... (N chars omitted) ...\n" + tail.
    out.insert(
        "output".to_string(),
        json!(format!("{head}\n... ({omitted} chars omitted) ...\n{tail}")),
    );
    Value::Object(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn banners_match_prism_widths() {
        // Format: "── tool ── KIND " + sep* where sep = 55 - len(tool) - 14/-15
        // (Prism log.py); dash count = 4 border + sep.
        let r = request_banner("put_document");
        assert_eq!(
            r.chars().filter(|c| *c == '─').count(),
            4 + (55 - 12 - "REQUEST".len() - 14)
        );
        assert!(r.starts_with("── put_document ── REQUEST"));
        assert!(response_banner("run_shell").contains("RESPONSE"));
        // Pathological long name must not underflow.
        let long = request_banner(&"x".repeat(60));
        assert!(long.ends_with("REQUEST "));
    }

    #[test]
    fn params_content_truncation() {
        let lines: Vec<Value> = (0..25).map(|i| json!(format!("line{i}"))).collect();
        let p = json!({"name": "X.cls", "content": lines});
        let t = truncate_params(&p);
        let out = t.get("content").unwrap().as_array().unwrap();
        assert_eq!(out.len(), 7);
        assert_eq!(out[0], json!("line0"));
        assert_eq!(out[3], json!("  ... (25 lines total) ..."));
        assert_eq!(out[6], json!("line24"));
        // <=10 untouched
        let small = json!({"content": ["a", "b"]});
        assert_eq!(truncate_params(&small), small);
        // no content untouched
        let other = json!({"command": "write 1"});
        assert_eq!(truncate_params(&other), other);
    }

    #[test]
    fn result_output_truncation() {
        let big = "é".repeat(5_000) + &"x".repeat(3_000);
        let r = json!({"output": big, "prompt": "USER>"});
        let t = truncate_result(&r);
        let out = t.get("output").unwrap().as_str().unwrap();
        assert!(out.contains("\n... (5000 chars omitted) ...\n"));
        assert_eq!(out.chars().count(), 3030); // head 2000 + marker 30 + tail 1000
        assert_eq!(t.get("prompt").unwrap(), "USER>");
        // short output untouched
        let small = json!({"output": "hello"});
        assert_eq!(truncate_result(&small), small);
    }
}
