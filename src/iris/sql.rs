//! SQL endpoint: `POST /{ns}/action/query` (verified shape: body key is
//! `query`, NOT `SQL` — iris-atelier.md §5/§10).

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::iris::http::{Envelope, IrisClient};

/// One SQL result page.
#[derive(Debug, Clone)]
pub struct QueryOutcome {
    /// Row objects in server order.
    pub rows: Vec<Value>,
    /// True when the server indicated more rows exist.
    pub has_more: bool,
    /// Console lines, if any.
    pub console: Vec<String>,
}

/// Execute a query in `namespace`, asking for at most `max_rows`.
///
/// # Errors
/// Propagates [`Error`] from transport, envelope, and nested SQL errors.
pub async fn query(
    client: &IrisClient,
    namespace: &str,
    sql: &str,
    max_rows: u32,
) -> Result<QueryOutcome> {
    let url = format!("{}/action/query", client.api_url(namespace));
    let env = client
        .post_json(&url, &json!({ "query": sql, "maxRows": max_rows }))
        .await?;

    // SQL failures can ride nested in `result` even under a clean envelope.
    if let Some(err) = sql_error_from_envelope(&env) {
        return Err(err);
    }

    let content = env
        .result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // `id` + more rows -> hasMore paging marker (iris-atelier.md §5).
    let has_more = env
        .result
        .get("hasMore")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            env.result.get("id").is_some()
                && content.len() >= usize::try_from(max_rows).unwrap_or(usize::MAX)
        });

    Ok(QueryOutcome {
        rows: content,
        has_more,
        console: env.console,
    })
}

/// Turn a row array into (columns, rows-of-values) using first-row keys.
/// Rows missing a key from the union get `Value::Null`.
#[must_use]
pub fn rows_to_table(rows: &[Value]) -> (Vec<String>, Vec<Vec<Value>>) {
    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for k in obj.keys() {
                if !columns.contains(k) {
                    columns.push(k.clone());
                }
            }
        }
    }
    let out_rows = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| row.get(c).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    (columns, out_rows)
}

impl From<QueryOutcome> for crate::tools::sql::SqlResult {
    fn from(o: QueryOutcome) -> Self {
        let (columns, rows) = rows_to_table(&o.rows);
        let row_count = rows.len();
        Self {
            columns,
            rows,
            row_count,
            truncated: o.has_more,
        }
    }
}

/// Extract SQL error text from an in-result `status.errors` carrier.
/// `absorb` handles the envelope-level ones; this covers failures nested
/// inside `result` (IRIS occasionally puts SQL errors there).
pub(crate) fn sql_error_from_envelope(env: &Envelope) -> Option<Error> {
    if let Some(m) = env
        .result
        .pointer("/status/errors/0/error")
        .and_then(Value::as_str)
    {
        return Some(Error::Sql(m.to_string()));
    }
    // Some builds return {"result": {"status": "ERROR #..."}}
    env.result
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| s.contains("ERROR"))
        .map(|m| Error::Sql(m.to_string()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
mod tests {
    use super::*;

    #[test]
    fn rows_to_table_unions_keys() {
        let rows = vec![json!({"A": 1, "B": "x"}), json!({"A": 2, "C": true})];
        let (cols, vals) = rows_to_table(&rows);
        assert_eq!(cols, ["A".to_string(), "B".to_string(), "C".to_string()]);
        assert_eq!(vals[0][2], Value::Null);
        assert_eq!(vals[1][1], Value::Null);
    }
}
