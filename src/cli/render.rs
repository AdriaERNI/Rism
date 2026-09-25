//! Terminal rendering for shared-tool results. The ONLY module allowed to
//! print to stdout in CLI mode.

use crate::tools::command::CommandResult;
use crate::tools::compile::CompileResult;
use crate::tools::documents::{DocumentOut, ListDocumentsResult, PutDocumentResult};
use crate::tools::serverinfo::ServerInfoOut;
use crate::tools::sql::SqlResult;

use super::OutputFormat;

/// Render any serializable result as JSON (table fallback is the `plain` thunk).
pub fn render_json(v: &serde_json::Value, format: OutputFormat, plain: impl FnOnce()) {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        }
        OutputFormat::Table => plain(),
    }
}

/// Render a document list.
pub fn render_doc_list(res: &ListDocumentsResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            let widths = res
                .documents
                .iter()
                .map(|d| (d.name.len(), d.cat.len(), d.ts.len()))
                .fold((24usize, 3, 19), |(a, b, c), (n, t, s)| {
                    (a.max(n), b.max(t), c.max(s))
                });
            println!(
                "{:<w0$}  {:<w1$}  {:<w2$}",
                "NAME",
                "CAT",
                "TS",
                w0 = widths.0,
                w1 = widths.1,
                w2 = widths.2
            );
            println!("{}", "-".repeat(widths.0 + widths.1 + widths.2 + 4));
            for d in &res.documents {
                println!(
                    "{:<w0$}  {:<w1$}  {:<w2$}",
                    d.name,
                    d.cat,
                    d.ts,
                    w0 = widths.0,
                    w1 = widths.1,
                    w2 = widths.2
                );
            }
            println!("\n{} document(s)", res.count);
        },
    );
}

/// Render a document fetch (source in `--format json`, lines otherwise).
pub fn render_doc_get(doc: &DocumentOut, format: OutputFormat) {
    render_json(
        &serde_json::to_value(doc).unwrap_or_default(),
        format,
        || {
            for line in &doc.content {
                println!("{line}");
            }
        },
    );
}

/// Render a put (+ optional compile) result.
pub fn render_doc_put(res: &PutDocumentResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            println!("{} — saved (ts {})", res.name, res.ts);
            for line in &res.console {
                println!("{line}");
            }
        },
    );
}

/// Render a terminal command result.
pub fn render_command(res: &CommandResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            if !res.output.is_empty() {
                println!("{}", res.output.trim_end_matches('\n'));
            }
            let more = if res.output_truncated {
                format!(" (truncated, {} chars omitted)", res.output_omitted_chars)
            } else {
                String::new()
            };
            eprintln!("prompt: {}{more}", res.prompt.trim());
        },
    );
}

/// Render a standalone compile result.
pub fn render_compile(res: &CompileResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            for line in &res.console {
                println!("{line}");
            }
            for st in &res.statuses {
                if !st.status.is_empty() {
                    println!("{}: {}", st.name, st.status);
                }
            }
        },
    );
}

/// Render server info.
pub fn render_info(info: &ServerInfoOut, format: OutputFormat) {
    render_json(
        &serde_json::to_value(info).unwrap_or_default(),
        format,
        || {
            println!("base:      {}", info.base_url);
            println!("version:   {}", info.version);
            println!("atelier:   v{}", info.api);
            println!("namespace(s): {}", info.namespaces.join(", "));
        },
    );
}

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
