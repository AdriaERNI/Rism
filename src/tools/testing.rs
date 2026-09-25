//! Unit-testing tools: discovery, execution, and results — 100% Atelier API.
//!
//! Design note (differs from Prism deliberately): Prism auto-deploys a
//! `MCP.TestRunner` `SqlProc` class to the server. Rism must NEVER upload
//! code, so the trio works like this:
//! - discovery/results: plain SQL over `%Dictionary` / `%UnitTest_Result`
//!   via `action/query` (v1 endpoint).
//! - one terminal WebSocket session running
//!   `%UnitTest.Manager.DebugRunTestCase` directly — plus the required
//!   temp-namespace-equivalent setup (`^UnitTestRoot` pointing at a real dir:
//!   without it every suite silently fails with ERROR #5007, verified live).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::iris::IrisClient;
use crate::iris::sql as sqlapi;
use crate::iris::terminal;

/// Class-name allowlist (package.Class form, optional leading %).
/// The query endpoint has no bind parameters, so interpolation must be
/// validated — same rule Prism enforces.
fn valid_class_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '%')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn ns_of(client: &IrisClient, ns: Option<&String>) -> String {
    ns.cloned()
        .unwrap_or_else(|| client.settings().iris_namespace.clone())
}

/// Arguments for [`run_tests`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunTestsArgs {
    /// Test class extending %UnitTest.TestCase (must be compiled on server)
    pub test_class: String,
    /// Single Test* method to run (default: all)
    pub test_method: Option<String>,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
    /// Timeout seconds for the whole run (default: configured timeout)
    pub timeout_secs: Option<u64>,
}

/// One method's outcome inside a run.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MethodOutcome {
    /// Method name.
    pub name: String,
    /// passed | failed | skipped | unknown.
    pub status: String,
    /// Duration in seconds.
    pub duration: serde_json::Value,
    /// Failure description when failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Failing assertion actions when failed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<serde_json::Value>,
}

/// Result of [`run_tests`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RunTestsResult {
    /// The class that ran.
    pub class: String,
    /// passed | failed | unknown.
    pub status: String,
    /// Method counts.
    pub passed: usize,
    /// Method counts.
    pub failed: usize,
    /// Method counts.
    pub skipped: usize,
    /// Per-method outcomes.
    pub methods: Vec<MethodOutcome>,
    /// Runner console text (URL + summary lines).
    pub runner_output: String,
}

/// Run `%UnitTest` tests on the server — behind `rism test run` and MCP
/// `run_tests`.
///
/// # Errors
/// [`Error::Config`] for invalid names; [`Error::Terminal`] if the runner
/// call fails; SQL errors on the result query.
pub async fn run_tests(client: &IrisClient, args: &RunTestsArgs) -> Result<RunTestsResult> {
    if !valid_class_name(&args.test_class) {
        return Err(Error::Config(format!(
            "invalid class name: {}",
            args.test_class
        )));
    }
    if let Some(m) = &args.test_method {
        if !valid_identifier(m) {
            return Err(Error::Config(format!("invalid method name: {m}")));
        }
    }
    let ns = ns_of(client, args.namespace.as_ref());
    let timeout = Duration::from_secs(
        args.timeout_secs
            .unwrap_or(client.settings().timeout_secs.max(120)),
    );
    let method = args.test_method.clone().unwrap_or_default();

    // One WS session: prep ^UnitTestRoot (prerequisite verified live —
    // missing dir makes every suite fail with #5007) then run the manager.
    let runner = format!(
        "s root=$System.Util.ManagerDirectory()_\"Temp/UnitTest/\" d ##class(%File).CreateDirectoryChain(root) s ^UnitTestRoot=root w ##class(%UnitTest.Manager).DebugRunTestCase(\"\", \"{}\", \"/display=none\", \"{}\"),!",
        args.test_class, method
    );
    let out = terminal::execute(client, &ns, &runner, timeout).await?;

    // Read the structured results of the run we just triggered. The runner
    // prints its portal Index; take it to query that exact run.
    let index = out
        .output
        .split("Index=")
        .nth(1)
        .map(|s| {
            s.split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    if index.is_empty() {
        // No portal index: runner failed early — surface its text.
        return Ok(RunTestsResult {
            class: args.test_class.clone(),
            status: "unknown".to_string(),
            passed: 0,
            failed: 0,
            skipped: 0,
            methods: Vec::new(),
            runner_output: out.output,
        });
    }

    let query = format!(
        "SELECT tm.Name, tm.Status, tm.Duration, tm.ErrorAction, tm.ErrorDescription \
         FROM %UnitTest_Result.TestMethod tm \
         JOIN %UnitTest_Result.TestCase tc ON tm.TestCase = tc.ID \
         JOIN %UnitTest_Result.TestSuite ts ON tc.TestSuite = ts.ID \
         WHERE ts.TestInstance = {index} AND tc.Name = '{}' ORDER BY tm.Name",
        args.test_class
    );
    let res = sqlapi::query(client, &ns, &query, 1000).await?;

    let (methods, passed, failed, skipped) =
        collect_methods(client, &ns, &res.rows, &index, &args.test_class).await?;

    Ok(RunTestsResult {
        class: args.test_class.clone(),
        status: if failed > 0 {
            "failed"
        } else if passed > 0 {
            "passed"
        } else {
            "unknown"
        }
        .to_string(),
        passed,
        failed,
        skipped,
        methods,
        runner_output: out.output,
    })
}

/// Map result rows to per-method outcomes, pulling assertion detail for
/// failures (one extra query per failed method, like Prism).
async fn collect_methods(
    client: &IrisClient,
    ns: &str,
    rows: &[serde_json::Value],
    index: &str,
    test_class: &str,
) -> Result<(Vec<MethodOutcome>, usize, usize, usize)> {
    let mut methods = Vec::new();
    let (mut passed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    for row in rows {
        let name = row
            .get("Name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let code = row
            .get("Status")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1);
        let status = match code {
            1 => "passed",
            0 => "failed",
            2 => "skipped",
            _ => "unknown",
        };
        match status {
            "passed" => passed += 1,
            "failed" => failed += 1,
            "skipped" => skipped += 1,
            _ => {}
        }
        let mut outcome = MethodOutcome {
            name: name.clone(),
            status: status.to_string(),
            duration: row
                .get("Duration")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            error: row
                .get("ErrorDescription")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            assertions: Vec::new(),
        };
        if status == "failed" {
            let aq = format!(
                "SELECT ta.Action, ta.Description, ta.Status FROM %UnitTest_Result.TestAssert ta \
                 JOIN %UnitTest_Result.TestMethod tm ON ta.TestMethod = tm.ID \
                 JOIN %UnitTest_Result.TestCase tc ON tm.TestCase = tc.ID \
                 JOIN %UnitTest_Result.TestSuite ts ON tc.TestSuite = ts.ID \
                 WHERE ts.TestInstance = {index} AND tc.Name = '{test_class}' AND tm.Name = '{name}' ORDER BY ta.Counter",
            );
            if let Ok(ares) = sqlapi::query(client, ns, &aq, 1000).await {
                outcome.assertions = ares.rows;
            }
        }
        methods.push(outcome);
    }
    Ok((methods, passed, failed, skipped))
}

/// Arguments for [`list_tests`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTestsArgs {
    /// Class-name prefix filter (e.g. MyApp.Tests)
    pub filter: Option<String>,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// A discovered test class.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TestClassEntry {
    /// Class name.
    pub name: String,
    /// Test* methods.
    pub methods: Vec<String>,
}

/// Result of [`list_tests`].
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListTestsResult {
    /// Discovered classes.
    pub classes: Vec<TestClassEntry>,
    /// How many.
    pub count: usize,
}

/// Discover %UnitTest.TestCase subclasses and their Test* methods.
///
/// # Errors
/// SQL/transport errors.
pub async fn list_tests(client: &IrisClient, args: &ListTestsArgs) -> Result<ListTestsResult> {
    if let Some(f) = &args.filter {
        if !valid_class_name(f) {
            return Err(Error::Config(format!("invalid filter prefix: {f}")));
        }
    }
    let ns = ns_of(client, args.namespace.as_ref());
    let filter_clause = args
        .filter
        .as_ref()
        .map(|f| format!("AND cd.Name %STARTSWITH '{f}'"))
        .unwrap_or_default();
    let query = format!(
        "SELECT cd.Name AS class_name, md.Name AS method_name FROM %Dictionary.MethodDefinition md \
         JOIN %Dictionary.ClassDefinition cd ON md.parent = cd.Name \
         WHERE cd.Super [ '%UnitTest.TestCase' AND md.Name %STARTSWITH 'Test' {filter_clause} \
         ORDER BY cd.Name, md.Name"
    );
    let res = sqlapi::query(client, &ns, &query, 10_000).await?;
    let mut classes: Vec<TestClassEntry> = Vec::new();
    for row in &res.rows {
        let name = row
            .get("class_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let method = row
            .get("method_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if let Some(entry) = classes.iter_mut().find(|c| c.name == name) {
            entry.methods.push(method.to_string());
        } else {
            classes.push(TestClassEntry {
                name: name.to_string(),
                methods: vec![method.to_string()],
            });
        }
    }
    let count = classes.len();
    Ok(ListTestsResult { classes, count })
}

/// Arguments for [`get_test_results`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTestResultsArgs {
    /// Class name filter (default: all classes of the latest run)
    pub test_class: Option<String>,
    /// Max runs to list (history mode; default 1 = latest only)
    pub max_runs: Option<u32>,
    /// Target namespace (defaults to configured namespace)
    pub namespace: Option<String>,
}

/// One historical run with its per-class status.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RunHistoryEntry {
    /// `TestInstance` id (portal `Index`).
    pub run_id: serde_json::Value,
    /// Server timestamp.
    pub run_time: serde_json::Value,
    /// Class name.
    pub class: serde_json::Value,
    /// failed|passed|skipped (class-level).
    pub status: String,
}

/// Result of [`get_test_results`] (history view of stored runs).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TestHistoryResult {
    /// Rows across the requested runs.
    pub runs: Vec<RunHistoryEntry>,
    /// Rows returned.
    pub count: usize,
}

/// Read stored results of past runs (latest first).
///
/// # Errors
/// SQL/transport errors.
pub async fn get_test_results(
    client: &IrisClient,
    args: &GetTestResultsArgs,
) -> Result<TestHistoryResult> {
    if let Some(c) = &args.test_class {
        if !valid_class_name(c) {
            return Err(Error::Config(format!("invalid class name: {c}")));
        }
    }
    let ns = ns_of(client, args.namespace.as_ref());
    let limit = args.max_runs.unwrap_or(10).min(1000);
    let where_clause = args
        .test_class
        .as_ref()
        .map(|c| format!("WHERE tc.Name = '{c}'"))
        .unwrap_or_default();
    let query = format!(
        "SELECT TOP {limit} ti.ID AS run_id, ti.DateTime AS run_time, tc.Name AS class_name, tc.Status AS class_status \
         FROM %UnitTest_Result.TestCase tc \
         JOIN %UnitTest_Result.TestSuite ts ON tc.TestSuite = ts.ID \
         JOIN %UnitTest_Result.TestInstance ti ON ts.TestInstance = ti.ID \
         {where_clause} ORDER BY ti.ID DESC"
    );
    let res = sqlapi::query(client, &ns, &query, limit).await?;
    let runs: Vec<RunHistoryEntry> = res
        .rows
        .iter()
        .map(|row| RunHistoryEntry {
            run_id: row
                .get("run_id")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            run_time: row
                .get("run_time")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            class: row
                .get("class_name")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            status: match row.get("class_status").and_then(serde_json::Value::as_i64) {
                Some(1) => "passed",
                Some(0) => "failed",
                Some(2) => "skipped",
                _ => "unknown",
            }
            .to_string(),
        })
        .collect();
    let count = runs.len();
    Ok(TestHistoryResult { runs, count })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn class_name_allowlist() {
        assert!(valid_class_name("My.Tests.Calc"));
        assert!(valid_class_name("%UnitTest.SQLRegression"));
        assert!(!valid_class_name("Robert'; DROP TABLE"));
        assert!(!valid_class_name("Class.Name--x"));
        assert!(!valid_class_name(""));
        assert!(valid_identifier("TestAdd"));
        assert!(!valid_identifier("Test Add"));
        assert!(!valid_identifier("1Test"));
    }
}
