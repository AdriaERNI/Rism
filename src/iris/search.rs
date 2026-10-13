//! Code-search endpoint: `GET /{ns}/action/search` (Atelier text/regex
//! search; params + response shapes verified live per `E_SPEC` §2 — the
//! result payload is a TOP-LEVEL `result[]` array, with a console tally
//! line and `Maximum search limit reached` truncation notice).

use serde_json::Value;

use crate::error::{Error, Result};
use crate::iris::http::{Envelope, IrisClient};

/// Which wire knobs to send for one search. `None` means "omit the param"
/// so the server default applies (`E_SPEC` §1: `case`/`word`/`regex` are
/// only sent when the caller set them; `sys`/`gen` ride explicit defaults
/// from the tools layer).
#[derive(Debug, Clone)]
pub struct SearchParams {
    /// Raw query text (regex by server default) — user-controlled, always
    /// URL-encoded on the wire.
    pub query: String,
    /// Document masks, e.g. `["*.cls", "*.mac", "*.int"]`; joined with `,`.
    pub documents: Vec<String>,
    /// `regex` param; `None` omits it (server default is 1).
    pub regex: Option<bool>,
    /// `sys` param.
    pub include_system: Option<bool>,
    /// `gen` param.
    pub include_generated: Option<bool>,
    /// `case` param — only sent when `Some`.
    pub case_sensitive: Option<bool>,
    /// `word` param — only sent when `Some`.
    pub whole_word: Option<bool>,
    /// `max` param — values above 200 pass through unchanged; the server
    /// hard-caps (issue #17 fact).
    pub max: Option<u32>,
}

fn bool_flag(v: Option<bool>) -> Option<&'static str> {
    v.map(|b| if b { "1" } else { "0" })
}

/// Run a search in `namespace`; returns the raw envelope for the tools
/// layer to normalize ([`parse_search_result`]).
///
/// # Errors
/// Transport/envelope errors; HTTP 404 on a server without `action/search`
/// surfaces as [`Error::HttpStatus`] (deliberately NOT mapped to
/// `DocNotFound` — `E_SPEC` §2).
pub async fn search(client: &IrisClient, namespace: &str, p: &SearchParams) -> Result<Envelope> {
    let mut url = format!("{}/action/search", client.api_url(namespace));
    let mut qs: Vec<(String, String)> = vec![
        (
            "query".to_string(),
            urlencoding::encode(&p.query).into_owned(),
        ),
        (
            "documents".to_string(),
            urlencoding::encode(&p.documents.join(",")).into_owned(),
        ),
    ];
    if let Some(r) = bool_flag(p.regex) {
        qs.push(("regex".to_string(), r.to_string()));
    }
    if let Some(s) = bool_flag(p.include_system) {
        qs.push(("sys".to_string(), s.to_string()));
    }
    if let Some(g) = bool_flag(p.include_generated) {
        qs.push(("gen".to_string(), g.to_string()));
    }
    if let Some(c) = bool_flag(p.case_sensitive) {
        qs.push(("case".to_string(), c.to_string()));
    }
    if let Some(w) = bool_flag(p.whole_word) {
        qs.push(("word".to_string(), w.to_string()));
    }
    if let Some(m) = p.max {
        qs.push(("max".to_string(), m.to_string()));
    }
    let joined = qs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    url.push('?');
    url.push_str(&joined);

    client.get(&url).await
}

/// Console marker the server prints when it stops at the `max` cap
/// (probe: `"Found 2 occurrence/s in 2 file/s. Maximum search limit
/// reached."`).
pub const TRUNCATION_MARKER: &str = "Maximum search limit reached";

/// Normalize an `action/search` envelope into the tools-layer result
/// (`E_SPEC` §2 rules 1-6). Pure so the unit tests can pin it directly.
///
/// `env.result` shapes accepted: top-level array (normal), `[]`, `{}` and
/// absent/null (old-API-version empties, probe-verified) → zero results;
/// any other shape → [`Error::InvalidEnvelope`].
///
/// # Errors
/// [`Error::InvalidEnvelope`] on unparsable payloads.
pub fn parse_search_result(
    env: &Envelope,
    requested_max: u32,
) -> Result<crate::tools::search::SearchResult> {
    let entries: Vec<&Value> = match &env.result {
        Value::Array(a) => a.iter().collect(),
        Value::Null => Vec::new(),
        Value::Object(o) if o.is_empty() => Vec::new(),
        other => {
            return Err(Error::InvalidEnvelope(format!(
                "search result: expected array, got {}",
                match other {
                    Value::Object(_) => "object",
                    Value::String(_) => "string",
                    Value::Number(_) => "number",
                    Value::Bool(_) => "bool",
                    _ => "null",
                }
            )));
        }
    };

    let mut results: Vec<crate::tools::search::SearchDocMatches> = Vec::new();
    let mut match_count = 0usize;
    for entry in entries {
        let Some(doc) = entry.get("doc").and_then(Value::as_str) else {
            return Err(Error::InvalidEnvelope(
                "search result entry without string 'doc'".to_string(),
            ));
        };
        let raw_matches = entry
            .get("matches")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut matches = Vec::with_capacity(raw_matches.len());
        for m in &raw_matches {
            matches.push(normalize_match(m));
        }
        match_count += matches.len();
        results.push(crate::tools::search::SearchDocMatches {
            doc: doc.to_string(),
            matches,
        });
    }

    let truncated = env.console.iter().any(|l| l.contains(TRUNCATION_MARKER))
        || (match_count >= usize::try_from(requested_max).unwrap_or(usize::MAX)
            && !results.is_empty());

    // Tally = LAST console line starting with "Found " (probe shapes also
    // include "Not found." — kept when present and no Found line exists).
    let server_tally = env
        .console
        .iter()
        .rev()
        .find(|l| l.starts_with("Found "))
        .or_else(|| {
            env.console
                .iter()
                .rev()
                .find(|l| l.starts_with("Not found"))
        })
        .cloned();

    let doc_count = results.len();
    Ok(crate::tools::search::SearchResult {
        namespace: String::new(),
        query: String::new(),
        documents: Vec::new(),
        server_tally,
        truncated,
        doc_count,
        match_count,
        results,
    })
}

/// One `matches[]` entry: `line` is int (class members) OR string (routine
/// lines, probe); a non-numeric string moves to `label` with `line: null`
/// (`E_SPEC` §2 rule 3 — degrade, never drop).
fn normalize_match(m: &Value) -> crate::tools::search::SearchMatch {
    let (line, label) = match m.get("line") {
        None | Some(Value::Null) => (None, None),
        Some(Value::Number(n)) => (n.as_u64().and_then(|v| u32::try_from(v).ok()), None),
        Some(Value::String(s)) => match s.trim().parse::<u32>() {
            Ok(n) => (Some(n), None),
            Err(_) => (None, Some(s.clone())),
        },
        Some(other) => (None, Some(other.to_string())),
    };
    crate::tools::search::SearchMatch {
        member: m.get("member").and_then(Value::as_str).map(str::to_string),
        attr: m.get("attr").and_then(Value::as_str).map(str::to_string),
        line,
        label,
        text: m
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}
