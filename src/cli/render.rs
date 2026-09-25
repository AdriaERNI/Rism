//! Terminal rendering for shared-tool results. The ONLY module allowed to
//! print to stdout in CLI mode.

use crate::tools::command::CommandResult;
use crate::tools::compile::CompileResult;
use crate::tools::documents::{DocumentOut, ListDocumentsResult, PutDocumentResult};
use crate::tools::host::{ListFilesResult, ReadFileResult, ShellResult};
use crate::tools::monitor::MonitorResult;
use crate::tools::serverinfo::ServerInfoOut;
use crate::tools::sql::SqlResult;
use crate::tools::testing::{ListTestsResult, RunTestsResult, TestHistoryResult};

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

/// Render a test run.
pub fn render_run_tests(res: &RunTestsResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            for m in &res.methods {
                let flag = match m.status.as_str() {
                    "passed" => "PASS",
                    "failed" => "FAIL",
                    "skipped" => "SKIP",
                    _ => "??  ",
                };
                println!("[{flag}] {} ({}s)", m.name, m.duration);
                if let Some(e) = &m.error {
                    println!("       {e}");
                }
                for a in &m.assertions {
                    let st = a.get("Status").and_then(serde_json::Value::as_i64);
                    println!(
                        "       assert: {} — {} [{:?}]",
                        a.get("Action")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default(),
                        a.get("Description")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default(),
                        st
                    );
                }
            }
            println!(
                "\n{}: {} passed, {} failed, {} skipped",
                res.class, res.passed, res.failed, res.skipped
            );
        },
    );
}

/// Render test discovery.
pub fn render_list_tests(res: &ListTestsResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            for c in &res.classes {
                println!("{} ({} methods)", c.name, c.methods.len());
                for m in &c.methods {
                    println!("  - {m}");
                }
            }
            println!("\n{} class(es)", res.count);
        },
    );
}

/// Render results history.
pub fn render_results(res: &TestHistoryResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            for r in &res.runs {
                println!(
                    "run {} @ {}  {} [{}]",
                    r.run_id,
                    r.run_time,
                    r.class.as_str().unwrap_or_default(),
                    r.status
                );
            }
            println!("\n{} row(s)", res.count);
        },
    );
}

/// Render a monitor snapshot.
pub fn render_monitor(res: &MonitorResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            let s = &res.snapshot;
            println!(
                "grade: {:<9} overall {:>5.1}  (cpu {:.1}  mem {:.1}  disk {:.1}  proc {:.1})",
                s.grade,
                s.score.overall,
                s.score.cpu,
                s.score.memory,
                s.score.disk,
                s.score.process
            );
            println!(
                "alerts: {}   samples: {}   db: {:.2} GB total, {:.2} GB max, {} dbs, {:.1} ms avg latency",
                s.alerts_count,
                s.metric_count,
                s.aggregated.db_total_size_gb,
                s.aggregated.db_total_max_gb,
                s.aggregated.db_count,
                s.aggregated.db_avg_latency_ms
            );
            println!(
                "smh: {:.2} GB   csp conns: {:.0} ({:.0} in use)",
                s.aggregated.smh_total_gb,
                s.aggregated.csp_total_connections,
                s.aggregated.csp_in_use_connections
            );
            let cpu: Vec<String> = s
                .aggregated
                .cpu_by_type
                .iter()
                .map(|(k, v)| format!("{k}={v:.0}"))
                .collect();
            println!("cpu by type: {}", cpu.join("  "));
            if !s.aggregated.top_processes.is_empty() {
                println!("top processes (by commands):");
                for p in &s.aggregated.top_processes {
                    println!(
                        "  pid {:<6} {:>8} cmds  {:<12} {:<12} {}",
                        p.pid, p.commands, p.jobtype, p.namespace, p.routine
                    );
                }
            }
        },
    );
}

/// Render a shell result.
pub fn render_shell(res: &ShellResult, format: OutputFormat) {
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            print!("{}", res.stdout);
            if !res.stderr.is_empty() {
                eprint!("{}", res.stderr);
            }
            if res.exit_code != 0 {
                std::process::exit(res.exit_code);
            }
        },
    );
}

/// Render a file read.
pub fn render_read_file(res: &ReadFileResult, format: OutputFormat) {
    if let Some(err) = &res.error {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            print!("{}", res.content);
            if let Some(msg) = &res.truncation_message {
                eprintln!("{msg}");
            }
        },
    );
}

/// Render a workspace listing.
pub fn render_list_files(res: &ListFilesResult, format: OutputFormat) {
    if let Some(err) = &res.error {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
    render_json(
        &serde_json::to_value(res).unwrap_or_default(),
        format,
        || {
            for f in &res.files {
                let kind = if f.is_dir { "d" } else { "-" };
                println!(
                    "{kind} {:>10}  {}",
                    if f.is_dir {
                        String::new()
                    } else {
                        f.size.to_string()
                    },
                    f.path
                );
            }
            println!(
                "\n{} entr(y/ies){}
",
                res.count,
                if res.truncated { " (truncated)" } else { "" }
            );
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
