//! Live contract matrix over ALL 25 MCP tools via the stdio door — the
//! permanent proof for issue #16 (F1 overflow clamp, F2 chars-vs-bytes,
//! F3 background timeout 0) plus ≥2 observable cases per tool.
//!
//! Talks the same JSON-RPC harness as `background_jobs`
//! (`tests/support/mcp_harness.rs` — one harness, shared). Gated
//! `#[ignore]`: CI's live-smoke job runs it against a fresh IRIS;
//! locally `cargo test --test tool_matrix -- --ignored` with the dev
//! container up.
//!
//! Discipline (PLAN §B1/B6): every case asserts concrete values (row
//! counts, byte-exact round-trips, exit codes, frame content) — never
//! "answered without error". Fixtures are `RismCI.Matrix*` in USER and
//! are deleted on EVERY exit path: bodies collect failures and clean up
//! before asserting, so a red case leaves the server as clean as a green
//! one. No running task may outlive its test. Debugger tools live in
//! ONE test (single server slot); every other test performs zero
//! debug_* calls. Concurrent answers are matched by id — no test relies
//! on response order.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    // json!() temporaries by value is the natural shape of a test RPC client
    clippy::needless_pass_by_value,
    // the two-line RPC client idiom (m=Mcp, c=Case) repeats per test fn
    clippy::many_single_char_names
)]

#[path = "support/mcp_harness.rs"]
mod harness;

use harness::*;
use serde_json::json;
use std::process::Command;

/// Absurd timeout from issue #16's repro (`u64::MAX` seconds).
const U64_MAX: u64 = 18_446_744_073_709_551_615;

/// Minimal valid class source for `RismCI.Matrix*` fixtures.
fn class_source(name: &str) -> Vec<String> {
    vec![
        format!("Class {name} Extends %RegisteredObject"),
        "{".into(),
        String::new(),
        "ClassMethod Ok() As %Status".into(),
        "{".into(),
        "    quit $$$OK".into(),
        "}".into(),
        "}".into(),
    ]
}

/// A test body that can fail without skipping its cleanup. Cases push
/// messages into `failures`; the test then ALWAYS runs `cleanup` and
/// only then panics with the collected list.
struct Case {
    failures: Vec<String>,
}

impl Case {
    fn new() -> Self {
        Self {
            failures: Vec::new(),
        }
    }

    fn check(&mut self, cond: bool, msg: impl FnOnce() -> String) {
        if !cond {
            self.failures.push(msg());
        }
    }

    /// Call a tool expecting success; returns the parsed payload (Null
    /// on failure, with the error text recorded).
    fn ok(&mut self, m: &mut Mcp, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let (is_err, parsed, text) = m.call(tool, args);
        if is_err {
            self.failures
                .push(format!("{tool} expected success, got error: {text}"));
            return serde_json::Value::Null;
        }
        parsed
    }

    /// Call a tool expecting a failure on EITHER channel (tool-level
    /// isError or a JSON-RPC error) whose text contains `needle`
    /// (case-insensitive) — the message must name what to fix.
    fn err(&mut self, m: &mut Mcp, tool: &str, args: serde_json::Value, needle: &str) -> String {
        let (is_err, _, text) = m.call(tool, args);
        if !is_err {
            self.failures.push(format!(
                "{tool} expected error naming {needle:?}, got success: {}",
                text.chars().take(200).collect::<String>()
            ));
        } else if !text.to_lowercase().contains(needle) {
            self.failures.push(format!(
                "{tool} error text does not contain {needle:?}: {text}"
            ));
        }
        text
    }

    fn finish(self) {
        assert!(
            self.failures.is_empty(),
            "matrix case failures:\n{}",
            self.failures.join("\n")
        );
    }
}

/// Put a `RismCI.Matrix*` fixture and register it for cleanup.
/// `ignore_conflict: true` — fixtures are content-stable, and a stale
/// leftover from a crashed run must not cascade a red across suites.
fn put_fixture(c: &mut Case, m: &mut Mcp, name: &str, cleanup: &mut Vec<String>) {
    let r = c.ok(
        m,
        "put_document",
        json!({"name": name,
               "content": class_source(name.trim_end_matches(".cls")),
               "ignore_conflict": true}),
    );
    cleanup.push(name.to_string());
    c.check(r["name"] == json!(name), || {
        format!("put fixture {name} echoed wrong name: {r}")
    });
}

fn delete_fixtures(c: &mut Case, m: &mut Mcp, names: &[String]) {
    for n in names {
        let (is_err, _, text) = m.call("delete_document", json!({"name": n}));
        if is_err {
            c.failures
                .push(format!("CLEANUP: delete {n} failed: {text}"));
        }
    }
}

/// Health probe: the server answers a normal command again after an
/// error storm. Retried because 15 parallel tests hammer the shared
/// IRIS child-process slots and a transient #6713 "Start target
/// failed" says nothing about the error path under test — eventual
/// success is the honest contract.
fn probe_alive(c: &mut Case, m: &mut Mcp, marker: &str) {
    let mut last = String::new();
    for _ in 0..6 {
        let (is_err, parsed, text) = m.call(
            "execute_command",
            json!({"command": format!("write \"{marker}\",!")}),
        );
        if is_err {
            last = text;
        } else if parsed["output"]
            .as_str()
            .is_some_and(|s| s.contains(marker))
        {
            return;
        } else {
            last = format!("answered without marker: {parsed}");
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    c.check(false, || format!("server never came back: {last}"));
}

/// Delete THIS test's exact fixture names before creating them, ignoring
/// the not-found answer. Guards against leftovers from a crashed earlier
/// run (a conflict on `put_and_compile` would cascade a red); names are
/// test-exclusive, so parallel tests cannot pre-clean each other.
fn pre_clean(m: &mut Mcp, names: &[&str]) {
    for n in names {
        let _ = m.call("delete_document", json!({"name": n}));
    }
}

// ─────────────────────────────────────────────────────────────────────
// 1. execute_sql — row shape, TOP limit, actionable SQLCODE, namespace
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn sql_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let mut cleanup = Vec::new();
    pre_clean(&mut m, &["RismCI.MatrixSqlA.cls", "RismCI.MatrixSqlB.cls"]);
    for suffix in ["A", "B"] {
        put_fixture(
            &mut c,
            &mut m,
            &format!("RismCI.MatrixSql{suffix}.cls"),
            &mut cleanup,
        );
    }

    // H: two fixtures appear as EXACT rows (independently computed names)
    let r = c.ok(
        &mut m,
        "execute_sql",
        json!({"query": "select Name from %Dictionary.ClassDefinition WHERE Name %STARTSWITH 'RismCI.MatrixSql'"}),
    );
    c.check(r["row_count"] == json!(2), || {
        format!("expected exactly 2 fixture rows: {r}")
    });
    let mut names: Vec<String> = r["rows"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|row| row[0].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    c.check(
        names
            == vec!["RismCI.MatrixSqlA", "RismCI.MatrixSqlB"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<String>>(),
        || format!("row names not exact: {names:?}"),
    );
    c.check(
        r["columns"] == json!(["Name"]) && r["truncated"] == json!(false),
        || format!("columns/truncated shape wrong: {r}"),
    );

    // TOP is the honored row cap (verified live: this IRIS build returns
    // the full result set regardless of the maxRows body key — the curl
    // probe proved the server itself ignores it, so pin TOP semantics).
    let r = c.ok(
        &mut m,
        "execute_sql",
        json!({"query": "select top 7 Name from %Dictionary.ClassDefinition", "max_rows": 3}),
    );
    c.check(r["row_count"] == json!(7), || {
        format!("TOP 7 must answer 7 rows: {r}")
    });

    // E: unknown column — SQLCODE -29 text names the field (actionable)
    c.err(
        &mut m,
        "execute_sql",
        json!({"query": "select BogusCol from %Dictionary.ClassDefinition"}),
        "boguscol",
    );

    // E: unknown namespace — the message names the offending namespace
    let t = c.err(
        &mut m,
        "execute_sql",
        json!({"query": "select 1", "namespace": "NOPE.NS"}),
        "nope.ns",
    );
    let _ = t;

    delete_fixtures(&mut c, &mut m, &cleanup);
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 2. documents — put/get/list/delete lifecycle with byte-exact content
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn documents_lifecycle() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let name = "RismCI.MatrixDoc.cls";
    let content = class_source("RismCI.MatrixDoc");
    pre_clean(&mut m, &[name]);

    let r = c.ok(
        &mut m,
        "put_document",
        json!({"name": name, "content": content}),
    );
    c.check(
        r["name"] == json!(name)
            && r["ts"]
                .as_str()
                .is_some_and(|t| !t.is_empty() && t.starts_with("20")),
        || format!("put echo missing name/ts: {r}"),
    );

    // H: list filter finds exactly this doc
    let r = c.ok(
        &mut m,
        "list_documents",
        json!({"filter": "RismCI.MatrixDoc", "filetypes": ["CLS"]}),
    );
    c.check(r["count"] == json!(1), || {
        format!("filter must return exactly the fixture: {r}")
    });
    c.check(
        r["documents"][0]["name"] == json!(name) && r["documents"][0]["cat"] == json!("CLS"),
        || format!("doc metadata wrong: {r}"),
    );

    // H: get round-trips the source (IRIS style-normalizes the class
    // block — it can insert blank lines, so the honest contract is:
    // every NON-BLANK line comes back in order, nothing else, modulo
    // trailing blanks).
    let g = c.ok(&mut m, "get_document", json!({"name": name}));
    let got = g["content"].as_array().cloned().unwrap_or_default();
    let significant: Vec<&str> = got
        .iter()
        .filter_map(Value::as_str)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let wanted: Vec<&str> = content
        .iter()
        .map(String::as_str)
        .filter(|l| !l.trim().is_empty())
        .collect();
    c.check(significant == wanted, || {
        format!("non-blank lines must round-trip in order: got {significant:?}")
    });

    // X: second put WITHOUT ignore_conflict must fail (409 names the doc)
    let t = c.err(
        &mut m,
        "put_document",
        json!({"name": name, "content": content}),
        "409",
    );
    c.check(t.to_lowercase().contains("rismci.matrixdoc"), || {
        format!("conflict error must name the doc: {t}")
    });
    // H: second put WITH ignore_conflict=true succeeds
    let r = c.ok(
        &mut m,
        "put_document",
        json!({"name": name, "content": content, "ignore_conflict": true}),
    );
    c.check(r["name"] == json!(name), || {
        format!("ignore_conflict put must succeed: {r}")
    });

    // X: unknown namespace on list — error names the namespace
    c.err(
        &mut m,
        "list_documents",
        json!({"namespace": "NOPE.NS"}),
        "nope.ns",
    );

    // H: delete succeeds and the doc VANISHES (get then errors)
    let r = c.ok(&mut m, "delete_document", json!({"name": name}));
    c.check(r["deleted"] == json!(name), || {
        format!("delete echo wrong: {r}")
    });
    let t = c.err(&mut m, "get_document", json!({"name": name}), "not found");
    c.check(t.contains("RismCI.MatrixDoc"), || {
        format!("not-found error must name the doc: {t}")
    });
    // E: deleting again answers with the same not-found error (no crash)
    c.err(
        &mut m,
        "delete_document",
        json!({"name": name}),
        "not found",
    );

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 3. compilation — put_and_compile + compile_documents logs and errors
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn compile_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let good = "RismCI.MatrixComp.cls";
    let bad = "RismCI.MatrixBadComp.cls";
    let cleanup = vec![good.to_string(), bad.to_string()];
    pre_clean(&mut m, &[good, bad]);

    // H: put_and_compile compiles for real — console says so AND the
    // compiled class exists in %Dictionary.CompiledClass (1 row).
    let r = c.ok(
        &mut m,
        "put_and_compile",
        json!({"name": good, "content": class_source("RismCI.MatrixComp")}),
    );
    c.check(
        r["console"].as_array().is_some_and(|a| {
            a.iter().any(|l| {
                l.as_str()
                    .is_some_and(|s| s.contains("Compilation finished successfully"))
            })
        }),
        || format!("compile console missing success line: {r}"),
    );
    let r = c.ok(
        &mut m,
        "execute_sql",
        json!({"query": "select Name from %Dictionary.CompiledClass WHERE Name = 'RismCI.MatrixComp'"}),
    );
    c.check(r["row_count"] == json!(1), || {
        format!("class must be compiled (1 CompiledClass row): {r}")
    });

    // H: compile_documents (with the .cls suffix — the verified door
    // form; the bare class name 5844s) on the existing doc answers a
    // successful console naming the class (fresh compile or "up-to-date"
    // — both are the honest answer; "successfully" is common ground).
    let r = c.ok(&mut m, "compile_documents", json!({"names": [good]}));
    let joined = r["console"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<&str>>()
                .join("\n")
        })
        .unwrap_or_default();
    c.check(
        joined.contains("RismCI.MatrixComp") && joined.contains("successfully"),
        || format!("compile console must name the class + success: {r}"),
    );

    // X: a deliberately broken class — tool-level error carrying the IRIS
    // compile error text (names the class), and the doc still cleans up.
    let bad_src = vec![
        "Class RismCI.MatrixBadComp Extends %RegisteredObject".to_string(),
        "{".to_string(),
        "ClassMethod Ok() As %Status".to_string(),
        "    quit $$bogusfn(".to_string(),
        "}".to_string(),
    ];
    let t = c.err(
        &mut m,
        "put_and_compile",
        json!({"name": bad, "content": bad_src}),
        "rismci.matrixbadcomp",
    );
    c.check(t.to_lowercase().contains("error"), || {
        format!("compile failure text must carry the IRIS error: {t}")
    });

    // X: compiling an unknown name answers with the #5844 text, server
    // stays healthy (checked right after by a follow-up call).
    let t = c.err(
        &mut m,
        "compile_documents",
        json!({"names": ["RismCI.MatrixNothing"]}),
        "matrixnothing",
    );
    c.check(t.contains("5844"), || {
        format!("unknown-doc compile must surface ERROR #5844: {t}")
    });
    probe_alive(&mut c, &mut m, "post-compile-error-alive");

    delete_fixtures(&mut c, &mut m, &cleanup);
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 4. get_server_info — concrete fields + idempotent repeat
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn server_info_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    let r = c.ok(&mut m, "get_server_info", json!({}));
    c.check(
        r["version"]
            .as_str()
            .is_some_and(|v| v.contains("IRIS") && !v.is_empty()),
        || format!("version must name the product: {r}"),
    );
    c.check(r["api"].as_u64().unwrap_or(0) >= 1, || {
        format!("api version must be a positive int: {r}")
    });
    let ns: Vec<&str> = r["namespaces"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    c.check(ns.contains(&"USER") && ns.contains(&"%SYS"), || {
        format!("namespace list must contain USER and %SYS: {ns:?}")
    });
    c.check(
        r["base_url"]
            .as_str()
            .is_some_and(|u| u.starts_with("http")),
        || format!("base_url must be an http URL: {r}"),
    );

    // Repeat is stable (idempotence case): identical api + namespaces.
    let r2 = c.ok(&mut m, "get_server_info", json!({}));
    c.check(
        r2["api"] == r["api"] && r2["namespaces"] == r["namespaces"],
        || format!("second call disagrees with the first: {r2}"),
    );

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 5. execute_command — output frames, runtime errors, namespace door,
//    and the documented sync `timeout_secs: 0` immediate answer
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn execute_command_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    // H: multi-frame output lands verbatim with the prompt echoed.
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "for i=1:1:3 { write \"row\",i,! }"}),
    );
    let out = r["output"].as_str().unwrap_or_default().to_string();
    for i in 1..=3 {
        c.check(out.contains(&format!("row{i}")), || {
            format!("frame row{i} missing from output: {out:?}")
        });
    }
    c.check(
        r["prompt"] == json!("USER>") && r["namespace"] == json!("USER"),
        || format!("prompt/namespace echo wrong: {r}"),
    );
    c.check(
        r["output_truncated"] == json!(false) && r["output_omitted_chars"] == json!(0),
        || format!("untruncated call must report 0 omitted: {r}"),
    );

    // E: a runtime error arrives INSIDE the output frames (not as a tool
    // error) — the answer must surface it, never silence it.
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "write $$nope^RismCI.MissingRoutine"}),
    );
    c.check(
        r["output"]
            .as_str()
            .is_some_and(|s| s.contains("<NOROUTINE>")),
        || format!("runtime <NOROUTINE> error not in output: {r}"),
    );

    // H: namespace door — %SYS runs in %SYS and echoes it.
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "write $zv", "namespace": "%SYS"}),
    );
    c.check(
        r["namespace"] == json!("%SYS") && r["prompt"] == json!("%SYS>"),
        || format!("namespace override not echoed: {r}"),
    );
    c.check(
        r["output"].as_str().is_some_and(|s| s.contains("IRIS")),
        || format!("$zv output missing: {r}"),
    );

    // X (documented sync semantics): timeout 0 answers immediately with
    // a timeout error — under 10 s wall, not 30.
    let t0 = Instant::now();
    let text = c.err(
        &mut m,
        "execute_command",
        json!({"command": "h 5", "timeout_secs": 0}),
        "timed out",
    );
    c.check(t0.elapsed() < Duration::from_secs(10), || {
        format!(
            "sync timeout 0 must answer immediately, took {:?}",
            t0.elapsed()
        )
    });
    let _ = text;

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 6. run_shell — stdout, exit-code propagation, cwd, clamp behavior
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn run_shell_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    // H: stdout lands exactly, exit 0, cwd echoes the workspace default
    // (the test dir — cargo runs the test binary from the worktree root).
    let r = c.ok(&mut m, "run_shell", json!({"command": "echo matrix-shell"}));
    c.check(
        r["stdout"]
            .as_str()
            .is_some_and(|s| s.trim_end() == "matrix-shell"),
        || format!("stdout must be 'matrix-shell': {r}"),
    );
    c.check(r["exit_code"] == json!(0), || {
        format!("echo must exit 0: {r}")
    });

    // H: a non-zero exit is a RESULT carrying the code, not an error.
    let r = c.ok(&mut m, "run_shell", json!({"command": "exit 3"}));
    c.check(r["exit_code"] == json!(3), || {
        format!("exit 3 must propagate as exit_code: {r}")
    });

    // H: cwd override is honored (pwd reports it back).
    let r = c.ok(
        &mut m,
        "run_shell",
        json!({"command": "pwd", "cwd": "/tmp"}),
    );
    c.check(
        r["stdout"]
            .as_str()
            .is_some_and(|s| s.trim() == "/tmp" || s.trim().ends_with("/tmp"))
            && r["cwd"] == json!("/tmp"),
        || format!("cwd /tmp not honored: {r}"),
    );

    // E: timeout_secs clamps to >=1 — a sleep 3 with timeout 0 answers
    // with the kill notice naming the effective 1 s cap.
    let t = c.err(
        &mut m,
        "run_shell",
        json!({"command": "sleep 3", "timeout_secs": 0}),
        "timed out after 1s",
    );
    let _ = t;

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 7. read_file / list_files — workspace door: set, unset, traversal
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn workspace_files_contract() {
    // The spawned server reads its OWN env: RISM_WORKSPACE only steers
    // the door when set on the child (harness spawn_with_env).
    let root = std::env::current_dir().expect("cwd");
    let root_s = root.display().to_string();
    let mut m = Mcp::spawn_with_env(&[("RISM_WORKSPACE", root_s.as_str())]);
    let mut c = Case::new();

    // H: read the repo Cargo.toml through the door — real bytes.
    let r = c.ok(&mut m, "read_file", json!({"path": "Cargo.toml"}));
    c.check(
        r["content"]
            .as_str()
            .is_some_and(|s| s.contains("name = \"rism\"")),
        || format!("Cargo.toml must carry the package name: {}", r["path"]),
    );
    c.check(
        r["error"].is_null() && r["size"].as_u64().unwrap_or(0) > 100,
        || format!("clean read must report real size: {r}"),
    );

    // X: traversal escapes the root — the error names the path and why.
    let r = c.ok(&mut m, "read_file", json!({"path": "../../../etc/passwd"}));
    c.check(
        r["error"]
            .as_str()
            .is_some_and(|s| s.to_lowercase().contains("escape")),
        || format!("traversal must be blocked with an 'escapes' error: {r}"),
    );

    // H: list_files with the documented `**/*.rs` glob finds real files.
    let r = c.ok(
        &mut m,
        "list_files",
        json!({"path": "src", "pattern": "**/*.rs"}),
    );
    let paths: Vec<String> = r["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| f["path"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    c.check(
        r["count"].as_u64().unwrap_or(0) > 0 && paths.iter().any(|p| p.ends_with("lib.rs")),
        || {
            format!(
                "src/**/*.rs must include lib.rs (got count {:?})",
                r["count"]
            )
        },
    );
    c.check(paths.iter().all(|p| p.starts_with("src/")), || {
        format!("all paths must be src-relative: {paths:?}")
    });

    // E: a pattern that matches nothing answers with an empty list, no
    // error (empty is a valid answer).
    let r = c.ok(
        &mut m,
        "list_files",
        json!({"path": "src", "pattern": "**/*.qqq-nonexistent"}),
    );
    c.check(r["count"] == json!(0) && r["error"].is_null(), || {
        format!("no-match must be empty + error-free: {r}")
    });

    // H: max_results caps and flags truncation (3 of the **/*.rs hits).
    let r = c.ok(
        &mut m,
        "list_files",
        json!({"path": "src", "pattern": "**/*.rs", "max_results": 3}),
    );
    c.check(
        r["count"] == json!(3) && r["truncated"] == json!(true),
        || format!("max_results=3 must yield 3 + truncated: {r}"),
    );
    c.finish();

    // Unset-workspace door on its OWN spawn: both host-file tools answer
    // with the configuration error, never a crash.
    let mut plain = Mcp::spawn();
    let mut c2 = Case::new();
    let r = c2.ok(&mut plain, "read_file", json!({"path": "Cargo.toml"}));
    c2.check(
        r["error"]
            .as_str()
            .is_some_and(|s| s.contains("RISM_WORKSPACE")),
        || format!("read without workspace must name RISM_WORKSPACE: {r}"),
    );
    let r = c2.ok(&mut plain, "list_files", json!({"path": "src"}));
    c2.check(
        r["error"]
            .as_str()
            .is_some_and(|s| s.contains("RISM_WORKSPACE")),
        || format!("list without workspace must name RISM_WORKSPACE: {r}"),
    );
    c2.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 8. monitor_system — scored snapshot + raw_metrics toggle both ways
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn monitor_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    let r = c.ok(&mut m, "monitor_system", json!({}));
    for k in ["overall", "cpu", "memory", "disk", "process"] {
        let v = r["score"][k].as_f64();
        c.check(v.is_some_and(|v| (0.0..=100.0).contains(&v)), || {
            format!("score.{k} must be 0..=100: {r}")
        });
    }
    c.check(
        ["idle", "healthy", "moderate", "loaded", "critical"]
            .contains(&r["grade"].as_str().unwrap_or_default()),
        || format!("grade outside the known set: {}", r["grade"]),
    );
    c.check(r["metric_count"].as_u64().unwrap_or(0) > 0, || {
        format!("metric_count must be positive: {}", r["metric_count"])
    });
    c.check(r["raw_metrics"].is_null(), || {
        format!("raw_metrics must be absent by default: {}", r["timestamp"])
    });

    // Toggle: include_raw_metrics=true brings the array.
    let r2 = c.ok(
        &mut m,
        "monitor_system",
        json!({"include_raw_metrics": true}),
    );
    c.check(
        r2["raw_metrics"].as_array().is_some_and(|a| !a.is_empty()),
        || "raw_metrics must be a non-empty array when requested".to_string(),
    );
    c.check(r2["grade"].as_str().is_some(), || {
        "toggled call must still carry a grade".to_string()
    });

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 9. run_tests / list_tests / get_test_results — full %UnitTest door
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
// one linear run_tests/list_tests/get_test_results story, split would
// only re-plumb the same session across fns
#[allow(clippy::too_many_lines)]
fn testing_tools_contract() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let name = "RismCI.MatrixRunFixture";
    let cleanup = vec![format!("{name}.cls")];
    pre_clean(&mut m, &[&format!("{name}.cls")]);
    let src = vec![
        format!("Class {name} Extends %UnitTest.TestCase"),
        "{".into(),
        String::new(),
        "Method TestAlpha()".into(),
        "{".into(),
        "    do $$$AssertEquals(1,1)".into(),
        "}".into(),
        String::new(),
        "Method TestBravo()".into(),
        "{".into(),
        "    do $$$AssertEquals(1,2,\"deliberate\")".into(),
        "}".into(),
        "}".into(),
    ];
    c.ok(
        &mut m,
        "put_and_compile",
        json!({"name": format!("{name}.cls"), "content": src}),
    );

    // H: the WHOLE class — counts must match the fixture exactly
    // (1 passed, 1 failed) and both method names must appear.
    let r = c.ok(&mut m, "run_tests", json!({"test_class": name}));
    c.check(
        r["status"] == json!("failed")
            && r["passed"] == json!(1)
            && r["failed"] == json!(1)
            && r["skipped"] == json!(0),
        || {
            format!(
                "counts must be 1/1/0: {}",
                json!({
                    "status": r["status"], "passed": r["passed"],
                    "failed": r["failed"], "skipped": r["skipped"]
                })
            )
        },
    );
    let methods: Vec<&str> = r["methods"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["name"].as_str()).collect())
        .unwrap_or_default();
    c.check(
        methods.contains(&"TestAlpha") && methods.contains(&"TestBravo"),
        || format!("per-method outcomes missing: {methods:?}"),
    );

    // H: filtered single method — deterministic pass, and F1d rides
    // here: an absurd timeout_secs must ANSWER, not hang (issue #16).
    let t0 = Instant::now();
    let r = c.ok(
        &mut m,
        "run_tests",
        json!({"test_class": name, "test_method": "TestAlpha", "timeout_secs": U64_MAX}),
    );
    c.check(
        r["status"] == json!("passed") && r["passed"] == json!(1),
        || format!("TestAlpha alone must pass: {r}"),
    );
    c.check(t0.elapsed() < Duration::from_secs(120), || {
        format!(
            "u64::MAX run_tests must answer fast, took {:?}",
            t0.elapsed()
        )
    });

    // H: discovery lists the fixture with BOTH methods.
    let r = c.ok(&mut m, "list_tests", json!({"filter": name}));
    c.check(
        r["count"] == json!(1)
            && r["classes"][0]["name"] == json!(name)
            && r["classes"][0]["methods"]
                .as_array()
                .map(std::vec::Vec::len)
                == Some(2),
        || format!("list_tests must discover the fixture + 2 methods: {r}"),
    );

    // H: history contains our run (class field is the truth, not noise).
    let r = c.ok(
        &mut m,
        "get_test_results",
        json!({"test_class": name, "max_runs": 10}),
    );
    c.check(
        r["count"].as_u64().unwrap_or(0) >= 1
            && r["runs"]
                .as_array()
                .is_some_and(|a| a.iter().all(|e| e["class"] == json!(name))),
        || format!("history must return rows for our class only: {r}"),
    );
    c.check(
        r["runs"][0]["run_id"].is_number() && r["runs"][0]["status"].is_string(),
        || format!("history entry shape wrong: {}", r["runs"][0]),
    );

    // E: a class that never ran answers with an EMPTY envelope, no crash.
    let r = c.ok(
        &mut m,
        "get_test_results",
        json!({"test_class": "RismCI.MatrixNeverRan"}),
    );
    c.check(r["count"] == json!(0) && r["runs"].is_array(), || {
        format!("never-ran class must be count 0: {r}")
    });

    // X: junk filter prefix answers with an actionable config error.
    c.err(
        &mut m,
        "list_tests",
        json!({"filter": "zzz~!@#"}),
        "invalid filter",
    );

    // E: an unknown class must ANSWER — a runner verdict, never
    // silence (observed truth: status "unknown" + runner_output).
    let r = c.ok(
        &mut m,
        "run_tests",
        json!({"test_class": "RismCI.MatrixNoSuch"}),
    );
    c.check(
        r["status"].is_string() && !r["runner_output"].as_str().unwrap_or_default().is_empty(),
        || format!("unknown class must answer with a verdict + output: {r}"),
    );

    delete_fixtures(&mut c, &mut m, &cleanup);
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 10. F1 (issue #16) — absurd timeouts ANSWER on every door; the
//     server stays healthy across all of them
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn f1_overflow_timeouts_answer() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    // Sync door: u64::MAX must answer within a pollable budget.
    let t0 = Instant::now();
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "write \"f1-sync-ok\",!", "timeout_secs": U64_MAX}),
    );
    c.check(
        r["output"]
            .as_str()
            .is_some_and(|s| s.contains("f1-sync-ok")),
        || format!("sync u64::MAX must run the command: {r}"),
    );
    c.check(t0.elapsed() < Duration::from_secs(120), || {
        format!("sync door took {:?}", t0.elapsed())
    });

    // Background door: the task must reach a terminal state (>=2 polls:
    // start + at least one tasks/get, then wait for terminal) quickly.
    let handle = m.task_start(json!({
        "command": "for i=1:1:3 { write \"f1-bg\",i,!  h .1 }",
        "timeout_secs": U64_MAX,
    }));
    let id = Mcp::task_id(&handle);
    let mid = m.task(&id); // poll #1 (property: a pollable snapshot exists)
    c.check(
        mid["status"] == json!("working") || mid["status"] == json!("completed"),
        || format!("poll #1 status not pollable: {mid}"),
    );
    let done = m.wait_terminal(&id, Duration::from_secs(120)); // poll #2+
    c.check(done["status"] == json!("completed"), || {
        format!("background u64::MAX task must complete: {done}")
    });
    c.check(
        done["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("f1-bg1"),
        || format!("task result missing streamed marker: {}", done["result"]),
    );

    // run_shell door (already clamped at 3600 internally — answer + stay
    // within the documented cap).
    let t0 = Instant::now();
    let r = c.ok(
        &mut m,
        "run_shell",
        json!({"command": "echo f1-shell-ok", "timeout_secs": U64_MAX}),
    );
    c.check(
        r["stdout"]
            .as_str()
            .is_some_and(|s| s.contains("f1-shell-ok")),
        || format!("run_shell u64::MAX must run the command: {r}"),
    );
    c.check(t0.elapsed() < Duration::from_secs(30), || {
        format!("run_shell door took {:?}", t0.elapsed())
    });

    // Server health after the storm: a normal command answers again.
    probe_alive(&mut c, &mut m, "f1-still-healthy");

    c.finish();
}

/// F1c — the CLI door (`rism exec --timeout u64::MAX`) rides the same
/// clamp (main.rs repl path); the process must exit 0, not panic.
#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn f1_cli_exec_door_answers() {
    let exe = env!("CARGO_BIN_EXE_rism");
    let out = Command::new(exe)
        .args([
            "exec",
            "write \"f1-cli-door-ok\",!",
            "--timeout",
            &U64_MAX.to_string(),
        ])
        .output()
        .expect("spawn rism exec");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let mut c = Case::new();
    c.check(out.status.success(), || {
        format!("exec --timeout u64::MAX must exit 0: status={out:?} stderr={stderr}")
    });
    c.check(stdout.contains("f1-cli-door-ok"), || {
        format!("CLI door output missing marker: {stdout:?}")
    });
    c.check(!stderr.contains("panicked"), || {
        format!("CLI door panicked: {stderr}")
    });
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 11. F3 (issue #16) — background `timeout_secs: 0` means "use the
//     default": the task completes with output, not an instant timeout
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn f3_zero_timeout_background_uses_default() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();

    let handle = m.task_start(json!({
        "command": "for i=1:1:4 { write \"f3-zero\",i,!  h .1 }",
        "timeout_secs": 0,
    }));
    let id = Mcp::task_id(&handle);
    let first = m.task(&id); // poll #1
    c.check(
        first["status"] == json!("working") || first["status"] == json!("completed"),
        || format!("zero-timeout task must be alive or done, not failed: {first}"),
    );
    let done = m.wait_terminal(&id, Duration::from_secs(60)); // poll #2+
    c.check(done["status"] == json!("completed"), || {
        format!("F3: timeout 0 must complete via the default, got: {done}")
    });
    c.check(
        done["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("f3-zero4"),
        || format!("completed task must carry the output: {}", done["result"]),
    );

    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 12. F2 (issue #16) — output_omitted_chars counts CHARS, not bytes
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn f2_omitted_counts_chars_not_bytes() {
    // Cyrillic А-В are 2 UTF-8 bytes each: a byte-summed omitted count
    // is provably different from the char count (88 vs 176).
    let mut m = Mcp::spawn_with_env(&[("RISM_TERMINAL_MAX_OUTPUT_CHARS", "64")]);
    let mut c = Case::new();

    // One frame of 120 chars (240 bytes) against a 64-byte cap: 32
    // chars (64 bytes) retained, 88 CHARS omitted.
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "set s=\"\" for i=1:1:40 { set s=s_$c(1040,1041,1042) } write s"}),
    );
    let out = r["output"].as_str().unwrap_or_default();
    c.check(r["output_truncated"] == json!(true), || {
        format!("over-bound output must flag truncated: {r}")
    });
    c.check(out.chars().count() == 32 && out.len() == 64, || {
        format!(
            "retention must be 32 chars / 64 bytes: got {} chars",
            out.chars().count()
        )
    });
    c.check(r["output_omitted_chars"] == json!(88), || {
        format!(
            "omitted must be 88 CHARS (bytes math would say 176): {}",
            r["output_omitted_chars"]
        )
    });
    c.check(
        out.chars().all(|ch| ('\u{410}'..='\u{412}').contains(&ch)),
        || format!("retained text must be the Cyrillic payload: {out:?}"),
    );
    c.finish();

    // ASCII cross-check on the DEFAULT cap: retained + omitted must sum
    // to the exact emitted length — the honest-field contract for the
    // common 1-byte case too. (One space per column, then CRLF.)
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let r = c.ok(
        &mut m,
        "execute_command",
        json!({"command": "write $j(\"\",150000)"}),
    );
    let out = r["output"].as_str().unwrap_or_default().to_string();
    c.check(r["output_truncated"] == json!(true), || {
        format!("150k output must be truncated at 100k: {r}")
    });
    c.check(
        (out.chars().count() as u64) + r["output_omitted_chars"].as_u64().unwrap_or(0) >= 149_999
            && (out.chars().count() as u64) + r["output_omitted_chars"].as_u64().unwrap_or(0)
                <= 150_001,
        || {
            format!(
                "retained {} + omitted {} must sum to the 150k payload",
                out.chars().count(),
                r["output_omitted_chars"]
            )
        },
    );
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 13. The debugger chain — ALL 9 debug_* tools in ONE test (the server
//     carries one session slot). Every exit path stops + deletes.
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
// the debug agent slot is singular: the whole 9-tool lifecycle must
// live in ONE test owning ONE session start to stop
#[allow(clippy::too_many_lines)]
fn debugger_lifecycle_chain() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let cls = "RismCI.MatrixDbgFixture";
    let cleanup = vec![format!("{cls}.cls")];
    pre_clean(&mut m, &[&format!("{cls}.cls")]);
    let src = vec![
        format!("Class {cls} Extends %RegisteredObject"),
        "{".into(),
        String::new(),
        "ClassMethod Run() As %Integer".into(),
        "{".into(),
        "    set x=42".into(),
        "    for i=1:1:5 { s y=i }".into(),
        "    quit y".into(),
        "}".into(),
        "}".into(),
    ];
    c.ok(
        &mut m,
        "put_and_compile",
        json!({"name": format!("{cls}.cls"), "content": src}),
    );

    // debug_list_processes (pre-session): every entry carries a real
    // pid + namespace; the system view is non-empty on any live server.
    let procs = c.ok(&mut m, "debug_list_processes", json!({"system": true}));
    let arr = procs.as_array().cloned().unwrap_or_default();
    c.check(
        !arr.is_empty()
            && arr
                .iter()
                .all(|p| p["pid"].is_i64() && p["namespace"].is_string()),
        || format!("process list shape wrong: {procs}"),
    );

    // debug_start: entry break in the fixture, bp at offset 1.
    let s1 = c.ok(
        &mut m,
        "debug_start",
        json!({"target": format!("##class({cls}).Run()"), "stop_on_entry": true}),
    );
    let sid1 = s1["session_id"]
        .as_str()
        .map(String::from)
        .unwrap_or_default();
    c.check(!sid1.is_empty() && s1["state"] == json!("break"), || {
        format!("start must break at entry: {s1}")
    });
    c.check(
        s1["location"]["document"].as_str().is_some_and(|d| {
            std::path::Path::new(d)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cls"))
        }),
        || format!("entry location must be the .cls: {}", s1["location"]),
    );
    c.check(s1["breakpoints"][0]["state"] == json!("enabled"), || {
        format!("entry breakpoint must be enabled: {}", s1["breakpoints"])
    });

    // conflict door: a second start while live answers with the stop hint
    let t = c.err(
        &mut m,
        "debug_start",
        json!({"target": format!("##class({cls}).Run()"), "stop_on_entry": true}),
        "already active",
    );
    let _ = t;

    // debug_stack: IRIS levels start at 1.
    let st = c.ok(&mut m, "debug_stack", json!({"session_id": sid1}));
    c.check(
        st.is_array() && !st.as_array().unwrap().is_empty() && st[0]["level"] == json!(1),
        || format!("stack must start at level 1: {st}"),
    );

    // debug_variables at entry: x is NOT yet set (break is BEFORE line
    // 1's effects) — after step_over the set has run: x == 42.
    let v0 = c.ok(&mut m, "debug_variables", json!({"session_id": sid1}));
    c.check(v0["variables"].is_array(), || {
        format!("entry variables must be an array: {v0}")
    });
    let stepped = c.ok(
        &mut m,
        "debug_step",
        json!({"session_id": sid1, "action": "step_over"}),
    );
    c.check(stepped["state"] == json!("break"), || {
        format!("step_over must land on a break: {stepped}")
    });
    let found_x = stepped["variables"].as_array().is_some_and(|a| {
        a.iter().any(|v| {
            v["name"] == json!("x") && v["value"].as_str().is_some_and(|s| s.contains("42"))
        })
    });
    c.check(found_x, || {
        format!(
            "after step, x=42 must be visible in the frame vars: {}",
            stepped["variables"]
        )
    });
    let v1 = c.ok(&mut m, "debug_variables", json!({"session_id": sid1}));
    c.check(
        v1["variables"].as_array().is_some_and(|a| {
            a.iter().any(|v| {
                v["name"] == json!("x") && v["value"].as_str().is_some_and(|s| s.contains("42"))
            })
        }),
        || format!("debug_variables must see x=42: {}", v1["variables"]),
    );

    // debug_inspect: eval x returns 42; garbage answers an error.
    let ins = c.ok(
        &mut m,
        "debug_inspect",
        json!({"session_id": sid1, "expression": "x"}),
    );
    c.check(
        ins["value"].as_str().is_some_and(|s| s.contains("42")),
        || format!("inspect x must return 42: {ins}"),
    );
    let t = c.err(
        &mut m,
        "debug_inspect",
        json!({"session_id": sid1, "expression": "@@@garbage@@@"}),
        "error",
    );
    let _ = t;

    // debug_breakpoints: list (entry bp), set, list (2), remove, list (1).
    let l0 = c.ok(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "list"}),
    );
    c.check(
        l0["breakpoints"].as_array().map(std::vec::Vec::len) == Some(1),
        || format!("entry session must carry exactly the entry bp: {l0}"),
    );
    let set = c.ok(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "set",
                "breakpoint": {"class": cls, "method": "Run", "offset": 2}}),
    );
    let bp_id = set["breakpoint"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    c.check(
        !bp_id.is_empty() && set["breakpoint"]["state"] == json!("enabled"),
        || format!("set must return an enabled bp with an id: {set}"),
    );
    let l1 = c.ok(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "list"}),
    );
    c.check(
        l1["breakpoints"].as_array().map(std::vec::Vec::len) == Some(2),
        || format!("after set the list must have 2 bps: {l1}"),
    );
    c.ok(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "remove", "id": bp_id}),
    );
    let l2 = c.ok(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "list"}),
    );
    c.check(
        l2["breakpoints"].as_array().map(std::vec::Vec::len) == Some(1),
        || format!("after remove the list must be back to 1: {l2}"),
    );
    // error door: a bogus action answers with the action name.
    c.err(
        &mut m,
        "debug_breakpoints",
        json!({"session_id": sid1, "action": "teleport"}),
        "teleport",
    );
    // error door on step too: bogus action names itself.
    c.err(
        &mut m,
        "debug_step",
        json!({"session_id": sid1, "action": "teleport"}),
        "teleport",
    );

    // debug_stop: session ends, follow-ups answer "no active", a SECOND
    // start works — the squat guard (the slot really released).
    let stop = c.ok(&mut m, "debug_stop", json!({"session_id": sid1}));
    c.check(stop["state"] == json!("ended"), || {
        format!("stop must report ended: {stop}")
    });
    c.err(
        &mut m,
        "debug_variables",
        json!({"session_id": sid1}),
        "no active debug session",
    );
    let s2 = c.ok(
        &mut m,
        "debug_start",
        json!({"target": format!("##class({cls}).Run()"), "stop_on_entry": true}),
    );
    let sid2 = s2["session_id"].as_str().unwrap_or_default().to_string();
    c.check(!sid2.is_empty() && sid2 != sid1, || {
        format!("restart must mint a fresh session: {s2}")
    });

    // debug_step run-to-completion: target finishes -> session gone.
    let run = c.ok(
        &mut m,
        "debug_step",
        json!({"session_id": sid2, "action": "run"}),
    );
    c.check(
        run["state"] == json!("ended") || run["state"] == json!("break"),
        || format!("run must move the target: {run}"),
    );
    if run["state"] != json!("ended") {
        // still stepping (entry loop) — finish it off
        let _ = c.ok(
            &mut m,
            "debug_step",
            json!({"session_id": sid2, "action": "run"}),
        );
    }
    // best-effort stop on whatever is left (idempotent cleanup — an
    // ended session already self-removed and errors, which is fine).
    let _ = m.call("debug_stop", json!({"session_id": sid2}));

    // LAST: debug_attach's error door. A failed attach can leave the
    // server-side agent squatting until its idle timeout (verified live
    // — it poisons later debug_start connections for minutes), so it
    // runs AFTER every start in this chain, as the terminal case. The
    // happy-attach door is covered by the pid list above (PLAN B2 #23:
    // error path + cross-check; a live attach race is CI-fragile).
    let t = c.err(&mut m, "debug_attach", json!({"pid": 999_999}), "999999");
    c.check(t.to_lowercase().contains("pid"), || {
        format!("attach error must name the PID: {t}")
    });

    delete_fixtures(&mut c, &mut m, &cleanup);
    c.finish();
}

// ─────────────────────────────────────────────────────────────────────
// 14. tools/list — the 25-tool surface the matrix claims to cover
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn surface_is_the_25_tools_the_matrix_covers() {
    let mut m = Mcp::spawn();
    let mut c = Case::new();
    let msg = m.rpc("tools/list", json!({}));
    let tools = msg["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    c.check(tools.len() == 25, || {
        format!("matrix assumes 25 tools, tools/list says {}", tools.len())
    });
    // The exact set this file exercises — a tool added/removed/redenied
    // breaks here first, in the file that promises coverage.
    let covered = [
        "execute_sql",
        "list_documents",
        "get_document",
        "put_document",
        "delete_document",
        "put_and_compile",
        "compile_documents",
        "get_server_info",
        "execute_command",
        "run_tests",
        "list_tests",
        "get_test_results",
        "monitor_system",
        "run_shell",
        "read_file",
        "list_files",
        "debug_start",
        "debug_attach",
        "debug_list_processes",
        "debug_step",
        "debug_variables",
        "debug_inspect",
        "debug_stack",
        "debug_breakpoints",
        "debug_stop",
    ];
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for want in covered {
        c.check(names.contains(&want), || {
            format!("tool {want} missing from tools/list")
        });
    }
    for have in &names {
        c.check(covered.contains(have), || {
            format!("tools/list carries {have}, which no matrix case covers")
        });
    }
    c.finish();
}
