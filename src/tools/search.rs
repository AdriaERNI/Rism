//! `search_code` tool: thin adapter over [`crate::iris::search`] with the
//! §2 normalization. Framework-free (no rmcp/clap types).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::iris::IrisClient;
use crate::iris::search as api;
use crate::tools::names::valid_namespace;

/// Default document masks when the caller does not choose any (issue #17:
/// classes, macros, routines).
pub const DEFAULT_DOCUMENTS: [&str; 3] = ["*.cls", "*.mac", "*.int"];

/// Arguments for [`search_code`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCodeArgs {
    /// Text or regex to find (regex ON unless regex=false; URL-encoded)
    pub query: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Document masks (default: *.cls, *.mac, *.int)
    pub documents: Option<Vec<String>>,
    /// Regex mode; omit to use the server default (on)
    pub regex: Option<bool>,
    /// Include system documents (default false)
    pub include_system: Option<bool>,
    /// Include generated documents (default false)
    pub include_generated: Option<bool>,
    /// Case-sensitive match (only sent when set; server default applies otherwise)
    pub case_sensitive: Option<bool>,
    /// Whole-word match (only sent when set; server default applies otherwise)
    pub whole_word: Option<bool>,
    /// Max matches (default: settings `search_max_results` = 200; newer
    /// servers honour higher values — `truncated` flags a capped scan)
    pub max_results: Option<u32>,
}

/// One line-level hit inside a document.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchMatch {
    /// Member name for class-file hits; null for routine-line hits.
    pub member: Option<String>,
    /// Member attribute (e.g. "Description") when the server reports one.
    pub attr: Option<String>,
    /// 1-based line when it parses as a number (routine lines, member lines).
    pub line: Option<u32>,
    /// Raw non-numeric line token (e.g. "Description+32") when line is null.
    pub label: Option<String>,
    /// Matched text.
    pub text: String,
}

/// Hits grouped by document.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchDocMatches {
    /// Document name (e.g. "My.Cls.cls").
    pub doc: String,
    /// Matches inside it.
    pub matches: Vec<SearchMatch>,
}

/// Result of [`search_code`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    /// Namespace searched.
    pub namespace: String,
    /// Echo of the query.
    pub query: String,
    /// Document masks used.
    pub documents: Vec<String>,
    /// Server tally line ("Found N occurrence/s in M file/s." / "Not found.").
    pub server_tally: Option<String>,
    /// True when the server hit its search limit (console marker or the
    /// match count reached the requested max).
    pub truncated: bool,
    /// Documents with at least one match (client-side count).
    pub doc_count: usize,
    /// Total matches (client-side count).
    pub match_count: usize,
    /// Per-document hits.
    pub results: Vec<SearchDocMatches>,
}

/// Search source documents — behind `rism search` and MCP `search_code`.
///
/// # Errors
/// Transport/envelope errors; 404 on a server without `action/search` is
/// [`crate::Error::HttpStatus`] (not remapped).
pub async fn search_code(client: &IrisClient, args: &SearchCodeArgs) -> Result<SearchResult> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace.clone());
    // F-09: the namespace interpolates raw into the URL path segment —
    // refuse unsafe values before any HTTP request, same policy as the
    // graph tools' class-name guard.
    if !valid_namespace(&ns) {
        return Err(Error::Config(format!(
            "invalid namespace (URL path guard): {ns}"
        )));
    }
    let documents: Vec<String> = args
        .documents
        .clone()
        .map(|d| d.into_iter().filter(|s| !s.is_empty()).collect())
        .filter(|d: &Vec<String>| !d.is_empty())
        .unwrap_or_else(|| DEFAULT_DOCUMENTS.iter().map(|s| (*s).to_string()).collect());
    let max = args
        .max_results
        .unwrap_or(client.settings().search_max_results);
    let params = api::SearchParams {
        query: args.query.clone(),
        documents: documents.clone(),
        regex: args.regex,
        include_system: Some(args.include_system.unwrap_or(false)),
        include_generated: Some(args.include_generated.unwrap_or(false)),
        case_sensitive: args.case_sensitive,
        whole_word: args.whole_word,
        max: Some(max),
    };
    let env = api::search(client, &ns, &params).await?;
    let mut res = api::parse_search_result(&env, max)?;
    res.namespace = ns;
    res.query.clone_from(&args.query);
    res.documents = documents;
    Ok(res)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
mod tests {
    use super::*;
    use serde_json::json;

    /// U1 — serde contract: query required, unknown fields refused.
    #[test]
    fn search_args_roundtrip_denies_unknown() {
        let ok: SearchCodeArgs = serde_json::from_value(json!({"query": "Foo"})).unwrap();
        assert_eq!(ok.query, "Foo");
        assert!(ok.documents.is_none());
        assert!(ok.max_results.is_none());
        // missing query → error
        assert!(serde_json::from_value::<SearchCodeArgs>(json!({})).is_err());
        // extra field → error
        assert!(
            serde_json::from_value::<SearchCodeArgs>(json!({"query":"x","wild":true})).is_err()
        );
        // documents round-trips as a Vec
        let v: SearchCodeArgs =
            serde_json::from_value(json!({"query":"x","documents":["*.cls"]})).unwrap();
        assert_eq!(v.documents, Some(vec!["*.cls".to_string()]));
    }

    /// U6 — line normalization: int, numeric string, label token; never
    /// drops a match.
    #[test]
    fn search_line_int_string_label_normalize() {
        let env = |result: serde_json::Value| crate::iris::http::Envelope {
            status: crate::iris::http::EnvelopeStatus::default(),
            console: vec![],
            result,
        };
        // int line
        let r = crate::iris::search::parse_search_result(
            &env(json!([{"doc":"A.cls","matches":[{"line":5,"text":"x"}]}])),
            200,
        )
        .unwrap();
        assert_eq!(r.results[0].matches[0].line, Some(5));
        assert_eq!(r.results[0].matches[0].label, None);
        // numeric string line (routine)
        let r = crate::iris::search::parse_search_result(
            &env(json!([{"doc":"A.int","matches":[{"line":"9","text":"x"}]}])),
            200,
        )
        .unwrap();
        assert_eq!(r.results[0].matches[0].line, Some(9));
        // non-numeric string → label, line null, match kept
        let r = crate::iris::search::parse_search_result(
            &env(json!([{"doc":"A.cls","matches":[{"line":"Description+32","text":"x"}]}])),
            200,
        )
        .unwrap();
        assert_eq!(r.results[0].matches[0].line, None);
        assert_eq!(
            r.results[0].matches[0].label.as_deref(),
            Some("Description+32")
        );
        assert_eq!(r.match_count, 1, "match never dropped");
        // member hit with no line at all
        let r = crate::iris::search::parse_search_result(
            &env(json!([{"doc":"A.cls","matches":[{"member":"Set","text":"Set"}]}])),
            200,
        )
        .unwrap();
        assert_eq!(r.results[0].matches[0].member.as_deref(), Some("Set"));
        assert_eq!(r.results[0].matches[0].line, None);
    }
}
