//! Wiremock integration tests: lock the Atelier wire quirks documented in
//! documentation/iris-atelier.md §10 so a future refactor cannot silently
//! change what goes on the wire.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rism::iris::IrisClient;
use rism::settings::Settings;

fn settings_for(base: &str) -> Settings {
    Settings {
        iris_base_url: base.to_string(),
        ..Settings::default()
    }
}

fn client(mock: &MockServer) -> IrisClient {
    IrisClient::new(settings_for(mock.uri().as_str())).expect("client")
}

/// Same builder under a name a local `let client = client(..)` cannot
/// shadow — needed by tests that build several clients (search/graph).
fn client_of(mock: &MockServer) -> IrisClient {
    client(mock)
}

// §1 — %SYS must leave as %25SYS on the wire.
#[tokio::test]
async fn percent_namespace_is_encoded_in_path() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/%25SYS/action/query"))
        .and(body_json(json!({"query": "SELECT 1", "maxRows": 1000})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""},
            "console": [],
            "result": {"content": [{"x": 1}]}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let out = rism::iris::sql::query(&client, "%SYS", "SELECT 1", 1000)
        .await
        .expect("query ok");
    assert!(!out.has_more);
    mock.verify().await;
}

// §5 — Basic auth header must be present on every call.
#[tokio::test]
async fn basic_auth_is_sent() {
    let mock = MockServer::start().await;
    // base64("_SYSTEM:SYS")
    let expected = format!(
        "Basic {}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"_SYSTEM:SYS")
    );
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/doc/A.B.cls"))
        .and(header("authorization", expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"name": "A.B.cls", "cat": "CLS", "ts": "t", "content": ["x"]}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let doc = rism::iris::documents::get_doc(&client, "USER", "A.B.cls")
        .await
        .expect("authed get");
    assert_eq!(doc.content, vec!["x"]);
}

// §10 — compile body is a BARE ARRAY and flags ride the query string.
#[tokio::test]
async fn compile_uses_array_body_and_flags_qs() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/compile"))
        .and(query_param("flags", "cuk"))
        .and(body_json(json!(["A.B.cls"])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""},
            "console": ["Compilation finished successfully."],
            "result": {"content": [{"name": "A.B.cls", "status": ""}]}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let out = rism::iris::compile::compile(&client, "USER", &["A.B.cls".into()], "cuk")
        .await
        .expect("compile ok");
    assert_eq!(out.statuses.len(), 1);
    assert!(out.console[0].contains("successfully"));
}

// §10 — compile FAILURE arrives in envelope status.errors while HTTP is 200.
#[tokio::test]
async fn compile_failure_surfaces_iris_error() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/compile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [{"error": "ERROR #5475: boom", "code": 5475}], "summary": "ERROR #5475: boom"},
            "console": ["Detected 1 errors during compilation."],
            "result": {"content": []}
        })))
        .mount(&mock)
        .await;

    let client = client(&mock);
    let err = rism::iris::compile::compile(&client, "USER", &["A.B.cls".into()], "cuk")
        .await
        .expect_err("must fail");
    assert!(matches!(err, rism::Error::Iris { code: 5475, .. }), "{err}");
}

// §4 — PUT body is {enc:false, content:[lines]} and ts comes back in result.
#[tokio::test]
async fn put_doc_sends_line_array() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/atelier/v8/USER/doc/A.B.cls"))
        .and(body_json(
            json!({"enc": false, "content": ["line1", "line2"]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"name": "A.B.cls", "ts": "2026-09-25 09:00:00.000"}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let ts = rism::iris::documents::put_doc(
        &client,
        "USER",
        "A.B.cls",
        &["line1".into(), "line2".into()],
        false,
    )
    .await
    .expect("put ok");
    assert_eq!(ts, "2026-09-25 09:00:00.000");
}

// §10 — 404 on doc routes maps to DocNotFound.
#[tokio::test]
async fn delete_404_maps_to_doc_not_found() {
    let mock = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/atelier/v8/USER/doc/A.B.cls"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [], "result": {}
        })))
        .mount(&mock)
        .await;

    let client = client(&mock);
    let err = rism::iris::documents::delete_doc(&client, "USER", "A.B.cls")
        .await
        .expect_err("404");
    assert!(matches!(err, rism::Error::DocNotFound { .. }), "{err}");
}

// §5 — SQL errors can ride in result.status while HTTP 200 (nested shape).
#[tokio::test]
async fn sql_nested_error_surfaces() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"status": {"errors": [{"error": "SQL table not found", "code": -1}], "summary": ""}, "content": []}
        })))
        .mount(&mock)
        .await;

    let client = client(&mock);
    let err = rism::iris::sql::query(&client, "USER", "SELECT * FROM Nope", 10)
        .await
        .expect_err("nested sql error");
    assert!(
        matches!(err, rism::Error::Sql(_) | rism::Error::Iris { .. }),
        "{err}"
    );
}

// §2 — negotiate_version() must repoint later calls at the server's API.
#[tokio::test]
async fn version_negotiation_repoints_calls() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": {"version": "IRIS 2024.1", "api": 7}}
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v7/USER/action/query")) // v7, not the v8 default!
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": []}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    client.negotiate_version().await;
    rism::iris::sql::query(&client, "USER", "SELECT 1", 10)
        .await
        .expect("v7 path used after negotiation");
}

// Testing trio: discovery and history must go through action/query as pure
// SQL — never via an uploaded helper class (Rism's no-upload design rule).
#[tokio::test]
async fn list_tests_is_sql_only() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": [
                {"class_name": "My.Tests", "method_name": "TestA"},
                {"class_name": "My.Tests", "method_name": "TestB"}
            ]}
        })))
        .mount(&mock)
        .await;
    // Guard: if any code path tries PUT/doc routes (an upload), the mock
    // server has no route for them and they would 404 — assert none happen.
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(418))
        .expect(0)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let res = rism::tools::testing::list_tests(
        &client,
        &rism::tools::testing::ListTestsArgs {
            filter: Some("My.Tests".into()),
            namespace: None,
        },
    )
    .await
    .expect("list");
    assert_eq!(res.count, 1);
    assert_eq!(res.classes[0].methods, vec!["TestA", "TestB"]);
    mock.verify().await;
}

#[tokio::test]
async fn run_tests_rejects_unsafe_names_before_any_call() {
    let mock = MockServer::start().await;
    let client = client(&mock);
    let err = rism::tools::testing::run_tests(
        &client,
        &rism::tools::testing::RunTestsArgs {
            test_class: "Bobby'; DROP TABLE x;--".into(),
            test_method: None,
            namespace: None,
            timeout_secs: None,
        },
    )
    .await
    .expect_err("injection must be refused");
    assert!(matches!(err, rism::Error::Config(_)), "{err}");
}

// Monitor: /api/monitor/* is NOT envelope JSON — plain text endpoints.
#[tokio::test]
async fn monitor_metrics_parses_plain_text() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/monitor/metrics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "# HELP iris_cpu_usage CPU\niris_cpu_usage 40\niris_phys_mem_percent_used{id=\"x\"} 50\n",
        ))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/monitor/alerts"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&mock)
        .await;

    let client = client(&mock);
    let res = rism::tools::monitor::monitor_system(
        &client,
        &rism::tools::monitor::MonitorArgs {
            include_raw_metrics: true,
        },
    )
    .await
    .expect("snapshot");
    assert_eq!(res.snapshot.metric_count, 2);
    assert!((res.snapshot.metrics["iris_cpu_usage"] - 40.0).abs() < f64::EPSILON);
    assert_eq!(res.snapshot.alerts_count, 0);
    assert!(res.raw_metrics.is_some());
    // cpu: only iris_cpu_usage present — iris_cpu_pct contributes only when
    // the metric exists (Prism-parity guard; Prism scores this fixture at 40)
    assert!(
        (res.snapshot.score.cpu - 40.0).abs() < f64::EPSILON,
        "cpu {}",
        res.snapshot.score.cpu
    );
}

// §4 — docnames passes filter/filetypes/count as query params.
#[tokio::test]
async fn docnames_query_params() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/docnames"))
        .and(query_param("filter", "Rism%"))
        .and(query_param("filetypes", "CLS,RTN"))
        .and(query_param("count", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": [{"name": "Rism.X.cls", "cat": "CLS", "ts": "t", "db": "USER"}]}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let docs = rism::iris::documents::list_docs(
        &client,
        "USER",
        Some("Rism%"),
        Some(&["CLS", "RTN"]),
        Some(5),
    )
    .await
    .expect("list");
    assert_eq!(docs.len(), 1);
}

// §debug — debug_list_processes maps the %SYS/jobs envelope (Prism parity).
#[tokio::test]
async fn debug_list_processes_maps_jobs() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/api/atelier/v\d/%25SYS/jobs"))
        .and(query_param("system", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": [
                {"pid": 4711, "namespace": "USER", "routine": "%ZPT", "state": "CMD", "device": "|TCP|1"},
                {"pid": 9, "namespace": "%SYS", "routine": "X", "state": "HANG", "device": "/dev/null"}
            ]}
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let args = rism::tools::debugger::DebugListProcessesArgs {
        namespace: Some("USER".into()),
        system: Some(false),
    };
    let procs = rism::tools::debugger::debug_list_processes(&client, &args)
        .await
        .expect("jobs");
    assert_eq!(procs.len(), 1, "namespace filter client-side");
    assert_eq!(procs[0].pid, 4711);
    assert_eq!(procs[0].state, "CMD");
}

// ═════════════════════════════════════════════════════════════════════
// search_code (E_SPEC §2) — U3–U5, U7: wire params + envelope parsing
// ═════════════════════════════════════════════════════════════════════

// U3 — the query and the documents mask go on the wire percent-encoded.
#[tokio::test]
async fn search_url_encodes_query_and_masks() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        // query "a$b.*c?" — wiremock decodes, so pin the decoded value and
        // prove urlencoding ran via the RAW url (%24 must be on the wire).
        .and(query_param("query", "a$b.*c?"))
        .and(move |req: &wiremock::Request| {
            let raw = req.url.query().unwrap_or_default();
            raw.contains("%24") && raw.contains("%2A") && raw.contains("%3F")
        })
        // non-default masks join with "," then encode (%2C on the wire)
        .and(query_param("documents", "*.cls,*.mac"))
        .and(move |req: &wiremock::Request| {
            req.url
                .query()
                .unwrap_or_default()
                .contains("%2A.cls%2C%2A.mac")
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": ["Not found."],
            "result": []
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client(&mock);
    let res = rism::tools::search::search_code(
        &client,
        &rism::tools::search::SearchCodeArgs {
            query: "a$b.*c?".into(),
            namespace: None,
            documents: Some(vec!["*.cls".into(), "*.mac".into()]),
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect("search");
    assert_eq!(res.match_count, 0);
    mock.verify().await;
}

// U4 — param presence contract: case/word omitted when None; sys/gen
// explicit 0/0 with the tools defaults; regex omitted when None.
#[tokio::test]
async fn search_bools_omitted_or_zero_one() {
    let mock = MockServer::start().await;
    // None case/word: those params must NOT exist at all.
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .and(query_param("sys", "0"))
        .and(query_param("gen", "0"))
        .and(query_param("max", "200"))
        .and(move |req: &wiremock::Request| {
            let q = req
                .url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            !q.contains_key("case") && !q.contains_key("word") && !q.contains_key("regex")
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [], "result": []
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    rism::tools::search::search_code(
        &client,
        &rism::tools::search::SearchCodeArgs {
            query: "x".into(),
            namespace: None,
            documents: None,
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect("defaults");
    mock.verify().await;

    // Some(true): case=1&word=1&regex=1 must ride the wire.
    let mock2 = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .and(query_param("case", "1"))
        .and(query_param("word", "1"))
        .and(query_param("regex", "1"))
        .and(query_param("sys", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [], "result": []
        })))
        .expect(1)
        .mount(&mock2)
        .await;
    let cli2 = client_of(&mock2);
    rism::tools::search::search_code(
        &cli2,
        &rism::tools::search::SearchCodeArgs {
            query: "x".into(),
            namespace: None,
            documents: None,
            regex: Some(true),
            include_system: Some(true),
            include_generated: Some(false),
            case_sensitive: Some(true),
            whole_word: Some(true),
            max_results: Some(50),
        },
    )
    .await
    .expect("flags");
    mock2.verify().await;
}

// U5 — console tally extraction + truncation flag from the server marker.
#[tokio::test]
async fn search_parses_result_array_tally_truncation() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""},
            "console": ["",
                        "Searching for 'Class' in '*.cls'",
                        "A.B.cls(Class): Class",
                        "Found 2 occurrence/s in 2 file/s. Maximum search limit reached."],
            "result": [
                {"doc": "A.B.cls", "matches": [{"member": "Class", "text": "Class"}]},
                {"doc": "C.D.cls", "matches": [{"line": 1, "text": "Class C.D"}]}
            ]
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    let res = rism::tools::search::search_code(
        &client,
        &rism::tools::search::SearchCodeArgs {
            query: "Class".into(),
            namespace: None,
            documents: None,
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect("tally");
    assert_eq!(
        res.server_tally.as_deref(),
        Some("Found 2 occurrence/s in 2 file/s. Maximum search limit reached.")
    );
    assert!(res.truncated, "console marker sets truncated");
    assert_eq!(res.doc_count, 2);
    assert_eq!(res.match_count, 2);
    mock.verify().await;

    // No marker, few matches → truncated false.
    let mock2 = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""},
            "console": ["Found 1 occurrence/s in 1 file/s."],
            "result": [{"doc": "A.B.cls", "matches": [{"member": "Set", "text": "Set"}]}]
        })))
        .mount(&mock2)
        .await;
    let cli2 = client_of(&mock2);
    let res2 = rism::tools::search::search_code(
        &cli2,
        &rism::tools::search::SearchCodeArgs {
            query: "Set".into(),
            namespace: None,
            documents: None,
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect("no truncation");
    assert!(!res2.truncated);
    assert_eq!(
        res2.server_tally.as_deref(),
        Some("Found 1 occurrence/s in 1 file/s.")
    );
}

// U7 — result shapes: [] , {} (old-API empty), absent → 0 results;
// any other type → InvalidEnvelope.
#[tokio::test]
async fn search_empty_result_object_or_array() {
    for body in [
        json!([]),               // searched, nothing
        json!({}),               // old-API empty (probe)
        serde_json::Value::Null, // absent
    ] {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/atelier/v8/USER/action/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": {"errors": [], "summary": ""}, "console": [], "result": body
            })))
            .mount(&mock)
            .await;
        let cli = client_of(&mock);
        let res = rism::tools::search::search_code(
            &cli,
            &rism::tools::search::SearchCodeArgs {
                query: "x".into(),
                namespace: None,
                documents: None,
                regex: None,
                include_system: None,
                include_generated: None,
                case_sensitive: None,
                whole_word: None,
                max_results: None,
            },
        )
        .await
        .expect("empty must not error");
        assert_eq!(res.match_count, 0);
        assert!(res.results.is_empty());
    }
    // string payload → InvalidEnvelope
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [], "result": "wat"
        })))
        .mount(&mock)
        .await;
    let cli = client_of(&mock);
    let err = rism::tools::search::search_code(
        &cli,
        &rism::tools::search::SearchCodeArgs {
            query: "x".into(),
            namespace: None,
            documents: None,
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect_err("non-array result must fail");
    assert!(matches!(err, rism::Error::InvalidEnvelope(_)), "{err}");
}

// ═════════════════════════════════════════════════════════════════════
// graph tools (E_SPEC §3) — U8–U12, U14: guard-before-HTTP + SQL shapes
// ═════════════════════════════════════════════════════════════════════

/// Exact `query` body matcher.
fn body_query(sql: String) -> impl Fn(&wiremock::Request) -> bool + Send + Sync {
    let needle = sql;
    move |req: &wiremock::Request| {
        let v: serde_json::Value =
            serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
        v.get("query").and_then(|q| q.as_str()) == Some(needle.as_str())
    }
}

/// Helper: a POST body whose `query` key contains `needle`.
fn body_query_contains(needle: &'static str) -> impl Fn(&wiremock::Request) -> bool + Send + Sync {
    move |req: &wiremock::Request| {
        let v: serde_json::Value =
            serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
        v.get("query")
            .and_then(|q| q.as_str())
            .is_some_and(|q| q.contains(needle))
    }
}

fn query_envelope(content: &[serde_json::Value]) -> serde_json::Value {
    json!({
        "status": {"errors": [], "summary": ""}, "console": [],
        "result": {"content": content, "more": false, "id": ""}
    })
}

// U8 — refusal happens BEFORE any HTTP request reaches the server.
#[tokio::test]
async fn subclasses_rejects_unsafe_class_before_http() {
    let mock = MockServer::start().await;
    let client = client(&mock);
    let err = rism::tools::graph::find_subclasses(
        &client,
        &rism::tools::graph::FindSubclassesArgs {
            class: "Bobby'; DROP TABLE x;--".into(),
            namespace: None,
            depth: None,
        },
    )
    .await
    .expect_err("injection must be refused");
    assert!(matches!(err, rism::Error::Config(_)), "{err}");
    // Zero requests: no mock mounted at all, so wiremock counts everything.
    assert_eq!(mock.received_requests().await.unwrap().len(), 0);
}

// U9 — same gate for class_members.
#[tokio::test]
async fn members_rejects_unsafe_class_before_http() {
    let mock = MockServer::start().await;
    let client = client(&mock);
    let err = rism::tools::graph::class_members(
        &client,
        &rism::tools::graph::ClassMembersArgs {
            class: "Bad'Name".into(),
            namespace: None,
            kind: None,
            include_inherited: None,
        },
    )
    .await
    .expect_err("guard must refuse");
    assert!(matches!(err, rism::Error::Config(_)), "{err}");
    assert_eq!(mock.received_requests().await.unwrap().len(), 0);
}

// U10 — same gate for class_dependencies.
#[tokio::test]
async fn deps_rejects_unsafe_class_before_http() {
    let mock = MockServer::start().await;
    let client = client(&mock);
    let err = rism::tools::graph::class_dependencies(
        &client,
        &rism::tools::graph::ClassDependenciesArgs {
            class: "Drop'; --".into(),
            namespace: None,
            direction: None,
            depth: None,
        },
    )
    .await
    .expect_err("guard must refuse");
    assert!(matches!(err, rism::Error::Config(_)), "{err}");
    assert_eq!(mock.received_requests().await.unwrap().len(), 0);
}

// U11 — the first-level SQL is exactly the frozen contains-walk, and a
// multi-super row is kept only when the frontier name is an EXACT element
// (substring over-match guard).
#[tokio::test]
async fn subclasses_sql_shape_contains_walk() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query(
            "SELECT Name, Super FROM %Dictionary.ClassDefinition WHERE Super [ 'X' ORDER BY Name"
                .to_string(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[
            // exact child of X
            json!({"Name": "Child1", "Super": "X"}),
            // multi-super: exact element X → kept
            json!({"Name": "Child2", "Super": "X,Y"}),
            // substring over-match (X is a prefix of Xyz) → dropped
            json!({"Name": "OverMatch", "Super": "Xyz"}),
        ])))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    let res = rism::tools::graph::find_subclasses(
        &client,
        &rism::tools::graph::FindSubclassesArgs {
            class: "X".into(),
            namespace: None,
            depth: Some(1),
        },
    )
    .await
    .expect("walk");
    assert_eq!(res.count, 2);
    assert_eq!(res.subclasses[0].name, "Child1");
    assert_eq!(res.subclasses[0].level, 1);
    assert_eq!(res.subclasses[1].name, "Child2");
    assert_eq!(res.subclasses[1].parent, "X,Y");
    // depth=1 with non-empty frontier at loop end → truncated
    assert!(res.truncated);
    mock.verify().await;
}

// U12 — Origin filter present/absent per include_inherited; XData has none.
// F-03 adds an existence cross-check query ONLY when all member tables come
// back empty; these mocks return rows for one table so the walk is not
// re-triggered — the empty-read branch is pinned separately (U19). The
// census tables (Query/Trigger/Index) ride the same Origin filter rule.
#[tokio::test]
#[allow(clippy::too_many_lines)] // one linear mock pin list: table -> SQL shape
async fn members_sql_shape_origin_filter() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query_contains("CompiledMethod"))
        .and(body_query_contains("AND Origin = 'X'"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(query_envelope(&[json!({"Name": "M1"})])),
        )
        .expect(1)
        .mount(&mock)
        .await;
    for tbl in [
        "CompiledProperty",
        "CompiledParameter",
        "CompiledQuery",
        "CompiledTrigger",
        "CompiledIndex",
    ] {
        let t = tbl.to_string();
        Mock::given(method("POST"))
            .and(path("/api/atelier/v8/USER/action/query"))
            .and(move |req: &wiremock::Request| {
                let v: serde_json::Value =
                    serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
                v["query"]
                    .as_str()
                    .unwrap_or("")
                    .contains(&format!("%Dictionary.{t} "))
                    && v["query"]
                        .as_str()
                        .unwrap_or("")
                        .contains("AND Origin = 'X'")
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
            .expect(1)
            .mount(&mock)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query_contains("CompiledXData"))
        .and(move |req: &wiremock::Request| {
            let v: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            !v["query"].as_str().unwrap_or("").contains(" AND Origin = ")
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client_of(&mock);
    rism::tools::graph::class_members(
        &client,
        &rism::tools::graph::ClassMembersArgs {
            class: "X".into(),
            namespace: None,
            kind: None,
            include_inherited: None,
        },
    )
    .await
    .expect("members (own)");
    mock.verify().await;

    // include_inherited=true → no Origin filter anywhere.
    let mock2 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(move |req: &wiremock::Request| {
            let v: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            !v["query"].as_str().unwrap_or("").contains(" AND Origin = ")
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
        .with_priority(10)
        .expect(7)
        .mount(&mock2)
        .await;
    // F-03: all seven tables empty triggers the existence cross-check; the
    // class exists → zeros are reported honestly (no error). Default
    // priority 0 beats the catch-all above.
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query(
            "SELECT Name FROM %Dictionary.ClassDefinition WHERE Name = 'X'".to_string(),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(query_envelope(&[json!({"Name": "X"})])),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&mock2)
        .await;
    let client2 = client_of(&mock2);
    rism::tools::graph::class_members(
        &client2,
        &rism::tools::graph::ClassMembersArgs {
            class: "X".into(),
            namespace: None,
            kind: None,
            include_inherited: Some(true),
        },
    )
    .await
    .expect("members (inherited)");
    // 7 table reads + 1 existence cross-check (all-zero result branch).
    assert_eq!(mock2.received_requests().await.unwrap().len(), 8);
}

// U14 — the impact query shape + substring drop on a DependsOn hit.
#[tokio::test]
async fn used_by_sql_shape_super_merge() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query(
            "SELECT Name, DependsOn, Super FROM %Dictionary.ClassDefinition WHERE DependsOn [ 'X' OR Super [ 'X' ORDER BY Name".to_string(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[
            json!({"Name": "User1", "DependsOn": "X", "Super": "%RegisteredObject"}),
            json!({"Name": "OverMatch", "DependsOn": "Xyz.Other", "Super": ""}),
            json!({"Name": "User2", "DependsOn": "", "Super": "X,P"}),
        ])))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    let res = rism::tools::graph::class_dependencies(
        &client,
        &rism::tools::graph::ClassDependenciesArgs {
            class: "X".into(),
            namespace: None,
            direction: Some(rism::tools::graph::DependencyDirection::UsedBy),
            depth: Some(1),
        },
    )
    .await
    .expect("used_by");
    assert_eq!(res.used_by.len(), 2, "substring hit Xyz dropped");
    assert_eq!(res.used_by[0].name, "User1");
    assert_eq!(res.used_by[1].name, "User2");
    assert!(
        res.depends_on.is_empty(),
        "used_by-only leaves the field []"
    );
    assert!(res.truncated, "depth-1 cap with live frontier");
    mock.verify().await;
}

// ═════════════════════════════════════════════════════════════════════
// F-fix round (PR #22 review) — U19..U21
// ═════════════════════════════════════════════════════════════════════

// U19 — F-03 regression: a name with no %Dictionary row (alias like
// %RegisteredObject, or a typo) must answer an honest not-found, NOT
// misleading all-zero members. All four Compiled* reads empty + no
// existence row → Error::Config.
#[tokio::test]
async fn members_unknown_class_is_honest_not_found() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
        .mount(&mock)
        .await;
    let client = client(&mock);
    let err = rism::tools::graph::class_members(
        &client,
        &rism::tools::graph::ClassMembersArgs {
            class: "Not.A.Class".into(),
            namespace: None,
            kind: None,
            include_inherited: None,
        },
    )
    .await
    .expect_err("zeros with no dictionary row must be an honest error");
    assert!(
        matches!(&err, rism::Error::Config(m) if m.contains("not found") && m.contains("alias")),
        "{err}"
    );
    // 8th request is the existence cross-check.
    let seen = mock.received_requests().await.unwrap();
    assert_eq!(seen.len(), 8, "7 tables + existence check");
}

// U20 — F-09 regression: the namespace rides the URL path unencoded, so an
// unsafe namespace is refused client-side with ZERO HTTP requests, on every
// new surface (search + all three graph tools).
#[tokio::test]
async fn hostile_namespace_refused_before_http() {
    for ns in ["A' OR 1=1--", "../admin", "USER&x=1", "", "%2e%2e"] {
        let mock = MockServer::start().await;
        let client = client(&mock);
        let e1 = rism::tools::search::search_code(
            &client,
            &rism::tools::search::SearchCodeArgs {
                query: "x".into(),
                namespace: Some(ns.to_string()),
                documents: None,
                regex: None,
                include_system: None,
                include_generated: None,
                case_sensitive: None,
                whole_word: None,
                max_results: None,
            },
        )
        .await
        .expect_err("ns guard must refuse");
        assert!(
            matches!(&e1, rism::Error::Config(m) if m.contains("invalid namespace")),
            "{e1}"
        );
        let e2 = rism::tools::graph::find_subclasses(
            &client,
            &rism::tools::graph::FindSubclassesArgs {
                class: "X".into(),
                namespace: Some(ns.to_string()),
                depth: Some(1),
            },
        )
        .await
        .expect_err("graph ns guard");
        assert!(matches!(e2, rism::Error::Config(_)), "{e2}");
        assert_eq!(
            mock.received_requests().await.unwrap().len(),
            0,
            "zero HTTP for ns {ns:?}"
        );
    }
    // %SYS stays legal (regression against over-tightening the predicate).
    let mock = MockServer::start().await;
    let client = client(&mock);
    let _ = rism::tools::graph::find_subclasses(
        &client,
        &rism::tools::graph::FindSubclassesArgs {
            class: "X".into(),
            namespace: Some("%SYS".into()),
            depth: Some(1),
        },
    )
    .await;
    // may fail later on the mock body, but must NOT be a Config guard error
    // and must have made ≥1 request.
    assert!(
        !mock.received_requests().await.unwrap().is_empty(),
        "%SYS must pass the guard"
    );
}

// U21 — F-04 regression: `--documents ''` (CLI empty string) falls back to
// the default masks instead of sending documents= (server HTTP 400).
#[tokio::test]
async fn empty_document_string_falls_back_to_defaults() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .and(query_param("documents", "*.cls,*.mac,*.int,*.inc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [], "result": []
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    let res = rism::tools::search::search_code(
        &client,
        &rism::tools::search::SearchCodeArgs {
            query: "x".into(),
            namespace: None,
            documents: Some(vec![String::new()]),
            regex: None,
            include_system: None,
            include_generated: None,
            case_sensitive: None,
            whole_word: None,
            max_results: None,
        },
    )
    .await
    .expect("empty mask falls back");
    assert_eq!(res.documents, vec!["*.cls", "*.mac", "*.int", "*.inc"]);
    mock.verify().await;
}

// ═════════════════════════════════════════════════════════════════════
// find_references + class_members census — U22..U27: guard-before-HTTP,
// SQL shapes, end-to-end scanner classification on fetched docs.
// ═════════════════════════════════════════════════════════════════════

fn refs_args(target: &str) -> rism::tools::refs::FindReferencesArgs {
    rism::tools::refs::FindReferencesArgs {
        target: target.into(),
        namespace: None,
        include_system: None,
        include_generated: None,
        max_results: None,
    }
}

fn search_envelope(docs: &[(&str, &str)]) -> serde_json::Value {
    // docs: (doc, matched text) — one line-level match per doc.
    let result: Vec<serde_json::Value> = docs
        .iter()
        .map(|(d, t)| json!({"doc": d, "matches": [{"line": 1, "text": t}]}))
        .collect();
    json!({"status": {"errors": [], "summary": ""}, "console": [], "result": result})
}

fn doc_envelope(lines: &[&str]) -> serde_json::Value {
    json!({"status": {"errors": [], "summary": ""}, "console": [],
           "result": {"name": "x", "cat": "CLS", "ts": "t", "content": lines}})
}

// U22 — unsafe target refuses with ZERO HTTP (grammar guard runs first).
#[tokio::test]
async fn refs_rejects_unsafe_target_before_http() {
    for bad in ["Robert'; DROP TABLE x", "^Bad';Name", "", "a b", "x^y"] {
        let mock = MockServer::start().await;
        let client = client(&mock);
        let err = rism::tools::refs::find_references(&client, &refs_args(bad))
            .await
            .expect_err("guard must refuse");
        assert!(matches!(err, rism::Error::Config(_)), "{bad:?}: {err}");
        assert_eq!(mock.received_requests().await.unwrap().len(), 0, "{bad:?}");
    }
}

// U23 — unsafe namespace refuses with ZERO HTTP on this surface too.
#[tokio::test]
async fn refs_rejects_hostile_namespace_before_http() {
    let mock = MockServer::start().await;
    let client = client(&mock);
    let a = rism::tools::refs::FindReferencesArgs {
        target: "FProbeG.Target.TgtMethod".into(),
        namespace: Some("A' OR 1=1--".into()),
        include_system: None,
        include_generated: None,
        max_results: None,
    };
    let err = rism::tools::refs::find_references(&client, &a)
        .await
        .expect_err("namespace guard must refuse");
    assert!(matches!(err, rism::Error::Config(_)), "{err}");
    assert_eq!(mock.received_requests().await.unwrap().len(), 0);
}

// U24 — SQL shapes: existence check, the two embedded-code narrowing
// queries, and the Super walk for a class target. The needle must ride
// inside the quoted literal with NO escaping damage and NO injection.
#[tokio::test]
async fn refs_sql_shapes_member_and_class_targets() {
    let mock = MockServer::start().await;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    {
        let seen = seen.clone();
        Mock::given(method("POST"))
            .and(path("/api/atelier/v8/USER/action/query"))
            .and(move |req: &wiremock::Request| {
                let v: serde_json::Value =
                    serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
                if let Some(q) = v["query"].as_str() {
                    seen.lock().unwrap().push(q.to_string());
                }
                true
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/atelier/v8/USER/action/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(search_envelope(&[])))
            .mount(&mock)
            .await;
    }
    // Member target: existence (2 rows: Class + re-read as class fails ->
    // error). Give the class a row so the walk runs.
    let client = client(&mock);
    let a = rism::tools::refs::FindReferencesArgs {
        target: "FProbeG.Target.TgtMethod".into(),
        namespace: None,
        include_system: None,
        include_generated: None,
        max_results: None,
    };
    // The existence mock returns [] for everything -> honest not-found.
    let err = rism::tools::refs::find_references(&client, &a)
        .await
        .expect_err("no dictionary row -> not found");
    assert!(
        matches!(&err, rism::Error::Config(m) if m.contains("not found")),
        "{err}"
    );
    let seen = seen.lock().unwrap().clone();
    // existence on the class part, then the whole-string re-read
    assert!(
        seen.iter()
            .any(|q| q
                == "SELECT Name FROM %Dictionary.ClassDefinition WHERE Name = 'FProbeG.Target'")
    );
    assert!(seen.iter().any(|q| q
        == "SELECT Name FROM %Dictionary.ClassDefinition WHERE Name = 'FProbeG.Target.TgtMethod'"));
}

// U25 — the full pass: literal-search candidates + SQL narrowing, fetch
// once, classify. Fixture doc shapes come back through the doc endpoint
// and the edge buckets must match the frozen semantics.
#[tokio::test]
#[allow(clippy::too_many_lines)] // one linear end-to-end fixture: mock -> classify -> assert buckets
async fn refs_end_to_end_classification() {
    let mock = MockServer::start().await;
    // existence: the class exists
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(body_query_contains(
            "ClassDefinition WHERE Name = 'FProbeG.Target'",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(query_envelope(&[json!({"Name": "FProbeG.Target"})])),
        )
        .up_to_n_times(1)
        .mount(&mock)
        .await;
    // narrowing: the two embedded-code SQL rows point at the target class
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .and(move |req: &wiremock::Request| {
            let v: serde_json::Value =
                serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
            let q = v["query"].as_str().unwrap_or("");
            q.contains("CompiledProperty WHERE SqlComputeCode [ 'TgtMethod'")
                || q.contains("CompiledTrigger WHERE Code [ 'TgtMethod'")
        })
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(query_envelope(&[json!({"parent": "FProbeG.Target"})])),
        )
        .mount(&mock)
        .await;
    // Super-of-owner queries: default empty (no rows).
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
        .with_priority(5)
        .mount(&mock)
        .await;
    // literal search returns the caller doc.
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .and(query_param("query", "TgtMethod"))
        .and(query_param("regex", "0"))
        .and(query_param("documents", "*.cls,*.mac,*.int,*.inc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_envelope(&[(
            "FProbeG.Caller.cls",
            "set t = ##class(FProbeG.Target).TgtMethod(1)",
        )])))
        .mount(&mock)
        .await;
    // doc fetches (once per candidate).
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/doc/FProbeG.Caller.cls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(doc_envelope(&[
            "Class FProbeG.Caller",
            "{",
            "ClassMethod Edge1() As %Status",
            "{",
            "    set t = ##class(FProbeG.Target).TgtMethod(1)",
            "    set r = $zobjmethod(obj, \"TgtMethod\", 4)",
            "    quit",
            "}",
            "}",
        ])))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("/api/atelier/v8/USER/doc/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(doc_envelope(&[
            "Class FProbeG.Target",
            "{",
            "ClassMethod TgtMethod(p1) As %Integer { Quit p1+1 }",
            "Method MInst() As %Integer { Quit ..TgtMethod(6) }",
            "}",
        ])))
        .mount(&mock)
        .await;

    let client = client(&mock);
    let a = rism::tools::refs::FindReferencesArgs {
        target: "FProbeG.Target.TgtMethod".into(),
        namespace: None,
        include_system: None,
        include_generated: None,
        max_results: None,
    };
    let res = rism::tools::refs::find_references(&client, &a)
        .await
        .expect("refs run");
    assert_eq!(res.kind, "class_member");
    assert_eq!(res.target, "FProbeG.Target.TgtMethod");
    // Caller ##class edge resolved; the $zobjmethod line unresolved with a
    // dynamic_string hint; the owner doc's ..-self edge resolved.
    let caller_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.doc == "FProbeG.Caller.cls")
        .collect();
    assert_eq!(caller_edges.len(), 1, "{:?}", res.edges);
    assert_eq!(caller_edges[0].via, "class_call");
    assert_eq!(caller_edges[0].member.as_deref(), Some("Edge1"));
    assert_eq!(
        res.unresolved
            .iter()
            .filter(|u| u.doc == "FProbeG.Caller.cls" && u.hint == "dynamic_string")
            .count(),
        1,
        "{:?}",
        res.unresolved
    );
    let self_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.doc == "FProbeG.Target.cls" && e.via == "self_call")
        .collect();
    assert_eq!(self_edges.len(), 1, "{:?}", res.edges);
    assert!(res.docs_scanned >= 2);
}

// U26 — class-not-found is an honest error, never an empty success
// (routine targets skip the existence walk entirely).
#[tokio::test]
async fn refs_class_not_found_is_error_routine_skips_dict() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/atelier/v8/USER/action/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/action/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_envelope(&[])))
        .mount(&mock)
        .await;
    let client = client(&mock);
    let err = rism::tools::refs::find_references(&client, &refs_args("No.Such.Class"))
        .await
        .expect_err("not-found");
    assert!(
        matches!(&err, rism::Error::Config(m) if m.contains("not found")),
        "{err}"
    );
    // Now a routine target: zero ClassDefinition queries for it.
    let before = mock.received_requests().await.unwrap().len();
    let res = rism::tools::refs::find_references(&client, &refs_args("^MyRoutine"))
        .await
        .expect("routine scan");
    assert_eq!(res.kind, "routine");
    let after = mock.received_requests().await.unwrap();
    for r in &after[before..] {
        let body = String::from_utf8_lossy(&r.body);
        assert!(
            !(body.contains("action/query")
                || (body.contains("\"query\"") && body.contains("ClassDefinition"))),
            "routine path must not touch ClassDefinition"
        );
    }
}

// U27 — class_members census SQL shapes: queries/triggers/indices tables,
// the `_Time` column (Time is reserved — a bare `Time` select must NEVER
// ride the wire), procedures = CompiledMethod WHERE SqlProc = 1.
#[tokio::test]
async fn members_census_sql_shapes() {
    let mock = MockServer::start().await;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    {
        let seen = seen.clone();
        Mock::given(method("POST"))
            .and(path("/api/atelier/v8/USER/action/query"))
            .and(move |req: &wiremock::Request| {
                let v: serde_json::Value =
                    serde_json::from_slice(&req.body).unwrap_or(serde_json::Value::Null);
                if let Some(q) = v["query"].as_str() {
                    seen.lock().unwrap().push(q.to_string());
                }
                true
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(query_envelope(&[])))
            .mount(&mock)
            .await;
    }
    let client = client(&mock);
    for kind in ["queries", "triggers", "indices", "procedures"] {
        let m: rism::tools::graph::ClassMembersArgs =
            serde_json::from_value(json!({"class": "FProbeG.Target", "kind": kind})).unwrap();
        // empty census + existing class -> zeros reported honestly, so the
        // existence row must answer.
        let _ = rism::tools::graph::class_members(&client, &m).await;
    }
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|q| q.contains("%Dictionary.CompiledQuery WHERE parent = 'FProbeG.Target'"))
    );
    assert!(seen.iter().any(|q| {
        q.contains("%Dictionary.CompiledTrigger WHERE parent = 'FProbeG.Target'")
            && q.contains("_Time")
    }));
    assert!(
        seen.iter()
            .any(|q| q.contains("%Dictionary.CompiledIndex WHERE parent = 'FProbeG.Target'"))
    );
    assert!(
        seen.iter()
            .any(|q| q.contains("CompiledMethod WHERE parent = 'FProbeG.Target' AND SqlProc = 1"))
    );
    // The reserved-word trap: NO query may select a bare `Time` column.
    for q in seen.iter().filter(|q| q.contains("CompiledTrigger")) {
        let sel = q.split(" FROM ").next().unwrap_or("");
        assert!(
            sel.split(',').all(|c| c.trim() != "Time"),
            "bare reserved column in SELECT: {q}"
        );
    }
}

// F-09 completion: the SQL, document, compile, and terminal doors interpolate
// the namespace into the URL path too, so they must refuse hostile values
// client-side with ZERO HTTP — same policy, same text, as search/graph/refs.
async fn hostile_ns_calls(
    client: &IrisClient,
    ns: &str,
) -> Vec<(&'static str, rism::error::Result<()>)> {
    vec![
        (
            "execute_sql",
            rism::tools::sql::execute_sql(
                client,
                &rism::tools::sql::ExecuteSqlArgs {
                    query: "select 1".into(),
                    namespace: Some(ns.into()),
                    max_rows: None,
                },
            )
            .await
            .map(|_| ()),
        ),
        (
            "list_documents",
            rism::tools::documents::list_documents(
                client,
                &rism::tools::documents::ListDocumentsArgs {
                    filter: None,
                    filetypes: None,
                    count: None,
                    namespace: Some(ns.into()),
                },
            )
            .await
            .map(|_| ()),
        ),
        (
            "get_document",
            rism::tools::documents::get_document(
                client,
                &rism::tools::documents::GetDocumentArgs {
                    name: "%Library.RegisteredObject.cls".into(),
                    namespace: Some(ns.into()),
                },
            )
            .await
            .map(|_| ()),
        ),
        (
            "compile_documents",
            rism::tools::compile::compile_documents(
                client,
                &rism::tools::compile::CompileDocumentsArgs {
                    names: vec!["No.Such.cls".into()],
                    flags: None,
                    namespace: Some(ns.into()),
                },
            )
            .await
            .map(|_| ()),
        ),
        (
            "execute_command",
            rism::tools::command::execute_command(
                client,
                &rism::tools::command::ExecuteCommandArgs {
                    command: "write 1".into(),
                    namespace: Some(ns.into()),
                    timeout_secs: None,
                    background: None,
                },
            )
            .await
            .map(|_| ()),
        ),
    ]
}

#[tokio::test]
async fn sql_documents_compile_command_refuse_hostile_namespace_before_http() {
    for ns in ["US;ER", "A' OR 1=1--", "../admin", "USER&x=1", ""] {
        let mock = MockServer::start().await;
        let client = client(&mock);
        for (tool, res) in hostile_ns_calls(&client, ns).await {
            match res {
                Err(rism::Error::Config(m)) => assert!(
                    m.contains("invalid namespace (URL path guard)"),
                    "{tool}/{ns:?}: wrong guard text: {m}"
                ),
                other => panic!("{tool} with ns {ns:?} must be a Config refusal, got {other:?}"),
            }
            assert_eq!(
                mock.received_requests().await.unwrap().len(),
                0,
                "{tool}: zero HTTP for ns {ns:?}"
            );
        }
    }
    // %SYS must still pass the guard on the SQL door (no over-tightening).
    let mock = MockServer::start().await;
    let client = client(&mock);
    let _ = rism::tools::sql::execute_sql(
        &client,
        &rism::tools::sql::ExecuteSqlArgs {
            query: "select 1".into(),
            namespace: Some("%SYS".into()),
            max_rows: Some(1),
        },
    )
    .await;
    assert!(
        !mock.received_requests().await.unwrap().is_empty(),
        "%SYS must reach the wire on execute_sql"
    );
}

// list_documents contract: the docnames endpoint ignores `filetypes` and
// `count` on this IRIS build (verified live), so the tool enforces both
// client-side. The wire still carries the params (back-compat with servers
// that DO honor them); the returned list must respect them either way.
#[tokio::test]
async fn list_documents_enforces_filetypes_and_count_client_side() {
    let rows = |n: usize, cat: &str| -> Vec<serde_json::Value> {
        (0..n)
            .map(|i| {
                json!({"name": format!("Pack.Doc{i}.cls"), "cat": cat,
                       "ts": "t", "db": "USER"})
            })
            .collect()
    };
    let mut content = rows(4, "CLS");
    content.push(json!({"name":"Pack.R1.mac","cat":"MAC","ts":"t","db":"USER"}));
    content.push(json!({"name":"Pack.R2.mac","cat":"MAC","ts":"t","db":"USER"}));
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/docnames"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": {"errors": [], "summary": ""}, "console": [],
            "result": {"content": content}
        })))
        .expect(3)
        .mount(&mock)
        .await;
    let client = client(&mock);
    // filetypes selects by category, case-insensitively
    let r = rism::tools::documents::list_documents(
        &client,
        &rism::tools::documents::ListDocumentsArgs {
            filter: None,
            filetypes: Some(vec!["cls".into()]),
            count: None,
            namespace: None,
        },
    )
    .await
    .expect("list");
    assert_eq!(r.count, 4, "only CLS rows: {r:?}");
    // count caps AFTER filetype filtering
    let r = rism::tools::documents::list_documents(
        &client,
        &rism::tools::documents::ListDocumentsArgs {
            filter: None,
            filetypes: Some(vec!["MAC".into()]),
            count: Some(1),
            namespace: None,
        },
    )
    .await
    .expect("list");
    assert_eq!((r.count, r.documents.len()), (1, 1), "count cap: {r:?}");
    assert_eq!(r.documents[0].cat, "MAC");
    // a filetype matching nothing is an empty SUCCESS, not an error
    let r = rism::tools::documents::list_documents(
        &client,
        &rism::tools::documents::ListDocumentsArgs {
            filter: None,
            filetypes: Some(vec!["qqq".into()]),
            count: None,
            namespace: None,
        },
    )
    .await
    .expect("empty filetype success");
    assert_eq!(r.count, 0);
}

// R3 guard: document names that would escape the namespace doc root (`..`
// segment) or address the collection endpoint (empty) must refuse with ZERO
// HTTP — the server error text they used to produce leaked the raw URL path.
#[tokio::test]
async fn unsafe_document_names_refused_before_http() {
    for name in ["", "../../etc/passwd", "../%SYS/doc/x.cls", "a/..\\b.cls"] {
        let mock = MockServer::start().await;
        let client = client(&mock);
        let res = rism::tools::documents::get_document(
            &client,
            &rism::tools::documents::GetDocumentArgs {
                name: name.into(),
                namespace: None,
            },
        )
        .await;
        match res {
            Err(rism::Error::Config(m)) => assert!(
                m.contains("invalid document name"),
                "guard text for {name:?}: {m}"
            ),
            other => panic!("{name:?} must refuse, got {other:?}"),
        }
        assert_eq!(
            mock.received_requests().await.unwrap().len(),
            0,
            "zero HTTP for doc {name:?}"
        );
    }
    // A CSP-style path name stays legal (slashes alone are not traversal).
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/atelier/v8/USER/doc//csp/user/menu.csp"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    let _ = rism::tools::documents::get_document(
        &client,
        &rism::tools::documents::GetDocumentArgs {
            name: "/csp/user/menu.csp".into(),
            namespace: None,
        },
    )
    .await;
}
