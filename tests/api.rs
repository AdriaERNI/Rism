//! Wiremock integration tests: lock the Atelier wire quirks documented in
//! documentation/iris-atelier.md §10 so a future refactor cannot silently
//! change what goes on the wire.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
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
