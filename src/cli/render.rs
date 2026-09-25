//! Terminal rendering for shared-tool results. The ONLY module allowed to
//! print to stdout in CLI mode.

use crate::tools::sql::SqlResult;

use super::OutputFormat;

/// Render a SQL result in the requested format.
pub fn render_sql(res: &SqlResult, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let v = serde_json::to_value(res).unwrap_or(serde_json::Value::Null);
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        OutputFormat::Table => {
            let widths: Vec<usize> = res
                .columns
                .iter()
                .map(|c| {
                    c.len().max(
                        res.rows
                            .iter()
                            .map(|r| {
                                res.columns
                                    .iter()
                                    .position(|x| x == c)
                                    .and_then(|i| r.get(i))
                                    .map_or(0, cell_width)
                            })
                            .max()
                            .unwrap_or(0),
                    )
                })
                .collect();

            let header = render_row(&res.columns.clone(), &widths);
            println!("{header}");
            println!("{}", "-".repeat(header.chars().count()));
            for row in &res.rows {
                let cells: Vec<String> = row
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Null => "NULL".to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                println!("{}", render_row(&cells, &widths));
            }
            let more = if res.truncated { " (truncated)" } else { "" };
            println!("\n{} row(s){more}", res.row_count);
        }
    }
}

fn cell_width(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Null => 4,
        serde_json::Value::String(s) => s.len(),
        other => other.to_string().len(),
    }
}

fn render_row(cells: &[String], widths: &[usize]) -> String {
    cells
        .iter()
        .zip(widths)
        .map(|(c, w)| format!("{c:<w$}"))
        .collect::<Vec<_>>()
        .join("  ")
        .trim_end()
        .to_string()
}
