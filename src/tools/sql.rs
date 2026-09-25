//! SQL tools. [`ExecuteSqlArgs`] is the canonical shape both front doors
//! convert into; field docs feed the MCP JSON schema AND `clap --help`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::iris::IrisClient;
use crate::iris::sql as api;

/// Arguments for [`execute_sql`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteSqlArgs {
    /// SQL statement to execute
    pub query: String,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Max rows to return (default 1000)
    pub max_rows: Option<u32>,
}

/// Rows plus paging metadata, shared by CLI table output and MCP
/// `structuredContent`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SqlResult {
    /// Column names (union of keys, server order).
    pub columns: Vec<String>,
    /// Row values aligned to `columns`.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Number of rows returned.
    pub row_count: usize,
    /// True when more rows exist server-side.
    pub truncated: bool,
}

/// Execute SQL — the single implementation behind `rism sql` and
/// MCP `execute_sql`.
///
/// # Errors
/// Propagates [`crate::error::Error`] from the Atelier layer.
pub async fn execute_sql(client: &IrisClient, args: &ExecuteSqlArgs) -> Result<SqlResult> {
    let ns = args
        .namespace
        .clone()
        .unwrap_or_else(|| client.settings().iris_namespace.clone());
    let max_rows = args.max_rows.unwrap_or(client.settings().sql_max_rows);

    let outcome = api::query(client, &ns, &args.query, max_rows).await?;
    Ok(outcome.into())
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

    #[test]
    fn args_roundtrip_denies_unknown() {
        let ok: ExecuteSqlArgs = serde_json::from_value(json!({"query": "SELECT 1"})).unwrap();
        assert_eq!(ok.query, "SELECT 1");
        assert!(ok.namespace.is_none());
        let bad = serde_json::from_value::<ExecuteSqlArgs>(json!({"query":"x","nope":1}));
        assert!(bad.is_err());
    }

    #[test]
    fn sql_result_schema_shape() {
        // schemars must produce an object schema with our 4 fields — this is
        // the MCP outputSchema contract.
        let s = schemars::schema_for!(SqlResult);
        let json = serde_json::to_value(&s).unwrap();
        let props = json.get("properties").and_then(|p| p.as_object());
        assert_eq!(props.map(serde_json::Map::len), Some(4));
    }
}
