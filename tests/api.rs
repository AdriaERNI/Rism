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
