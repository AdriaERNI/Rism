//! Prometheus metrics from `/api/monitor/*` — a sibling API of atelier/
//! (documentation/iris-atelier.md "Monitor"). Both endpoints are plain text
//! (exposition format / JSON), NOT Atelier envelopes, so they go through
//! [`IrisClient::get_text`].

use std::collections::BTreeMap;

use crate::error::Result;
use crate::iris::http::IrisClient;

/// One parsed Prometheus sample.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct Sample {
    /// Metric name.
    pub name: String,
    /// Labels from the `{...}` block (sorted).
    pub labels: BTreeMap<String, String>,
    /// Value; may be NaN/Inf — IRIS emits NaN for unavailable counters.
    pub value: f64,
}

/// `GET /api/monitor/metrics` → parsed samples.
///
/// # Errors
/// Transport or non-2xx status.
pub async fn metrics(client: &IrisClient) -> Result<Vec<Sample>> {
    let url = format!(
        "{}/api/monitor/metrics",
        client.settings().iris_base_url.trim_end_matches('/')
    );
    Ok(parse_prometheus(&client.get_text(&url).await?))
}

/// `GET /api/monitor/alerts` raw text. Alerts are CLEARED by each read
/// (same semantics as Prism's scrape) — count = alerts since last scrape.
///
/// # Errors
/// Transport or non-2xx status.
pub async fn alerts_raw(client: &IrisClient) -> Result<String> {
    let url = format!(
        "{}/api/monitor/alerts",
        client.settings().iris_base_url.trim_end_matches('/')
    );
    client.get_text(&url).await
}

/// Parse Prometheus exposition text: skip `#` comments/blank lines, split
/// `name{labels} value [timestamp]`.
#[must_use]
pub fn parse_prometheus(text: &str) -> Vec<Sample> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, rest)) = split_head_value(line) else {
            continue;
        };
        let Some(value) = parse_value(rest) else {
            continue;
        };
        let (name, labels) = match head.split_once('{') {
            Some((n, l)) => (
                n.to_string(),
                parse_labels(l.strip_suffix('}').unwrap_or(l)),
            ),
            None => (head.to_string(), BTreeMap::new()),
        };
        if !valid_metric_name(&name) {
            continue;
        }
        out.push(Sample {
            name,
            labels,
            value,
        });
    }
    out
}

/// Split at the first whitespace outside the label block.
fn split_head_value(line: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut in_quote = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '{' if !in_quote => depth += 1,
            '}' if !in_quote => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 && !in_quote => {
                let head = line[..i].trim_end();
                let rest = line[i..].trim_start();
                return Some((head, rest)).filter(|(h, _)| !h.is_empty());
            }
            _ => {}
        }
    }
    None
}

fn valid_metric_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// Parse a sample value: `NaN`/`±Inf` map to the f64 specials; a trailing
/// timestamp token is ignored.
fn parse_value(rest: &str) -> Option<f64> {
    let token = rest.split_whitespace().next()?;
    if token.eq_ignore_ascii_case("nan") {
        return Some(f64::NAN);
    }
    let body = token.strip_prefix(['+', '-']).unwrap_or(token);
    if body.eq_ignore_ascii_case("inf") || body.eq_ignore_ascii_case("infinity") {
        return Some(token.parse::<f64>().unwrap_or(f64::NAN));
    }
    token.parse::<f64>().ok()
}

/// Parse `key="value"` pairs with Prometheus escapes (`\\`, `\"`, `\n`).
fn parse_labels(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // key
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | ':'))
        {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect();
        // = "
        while i < chars.len() && matches!(chars[i], ' ' | '=') {
            i += 1;
        }
        if i < chars.len() && chars[i] == '"' {
            i += 1;
            let mut val = String::new();
            while i < chars.len() {
                match chars[i] {
                    '\\' if i + 1 < chars.len() => {
                        match chars[i + 1] {
                            'n' => val.push('\n'),
                            other => val.push(other),
                        }
                        i += 2;
                    }
                    '"' => {
                        i += 1;
                        break;
                    }
                    c => {
                        val.push(c);
                        i += 1;
                    }
                }
            }
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
        while i < chars.len() && matches!(chars[i], ',' | ' ') {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn parses_verified_exposition_shape() {
        // live capture (rism-iris /api/monitor/metrics): labels, leading-dot
        // value, NaN value, comments
        let text = "# HELP iris_cpu_pct CPU usage (percentage) by process type\n\
                    # TYPE iris_cpu_pct gauge\n\
                    iris_cpu_pct{id=\"AUXWD\"} 0\n\
                    iris_cpu_usage 0\n\
                    iris_csp_gateway_latency{id=\"127.0.0.1:52773\"} .81\n\
                    iris_db_free_space{id=\"MGR\",path=\"/usr/irissys/mgr/\"} NaN\n\
                    broken line here\n";
        let s = parse_prometheus(text);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].name, "iris_cpu_pct");
        assert_eq!(s[0].labels["id"], "AUXWD");
        assert!((s[2].value - 0.81).abs() < f64::EPSILON);
        assert!(s[3].value.is_nan());
    }

    #[test]
    fn parses_labels_with_escapes_and_multi_pair() {
        let s = parse_prometheus("x{a=\"1\",b=\"he\\\"llo\"} 5\n");
        assert_eq!(s[0].labels["a"], "1");
        assert_eq!(s[0].labels["b"], "he\"llo");
    }

    #[test]
    fn ignores_timestamp_suffix() {
        let s = parse_prometheus("m 7 1600000000000\n");
        assert!((s[0].value - 7.0).abs() < f64::EPSILON);
    }
}
