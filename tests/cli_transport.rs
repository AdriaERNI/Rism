//! Transport-door CLI contract (SPEC T3/T4): flag parsing, error paths,
//! and the stdio door's byte-identical default — none of these need IRIS.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]

use std::io::Write as _;

use assert_cmd::Command;
use predicates::prelude::*;

fn rism() -> Command {
    Command::cargo_bin("rism").expect("binary built by cargo")
}

#[test]
fn transport_bogus_is_a_clean_clap_error_exit_2() {
    rism()
        .args(["mcp", "--transport", "bogus"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid value 'bogus'"));
}

#[test]
fn transport_env_bad_value_also_errors() {
    // clap env= tier shares the value_parser: garbage in env is the same
    // deterministic exit-2 error, never a panic or a silent stdio fallback.
    rism()
        .arg("mcp")
        .env("RISM_MCP_TRANSPORT", "carrier-pigeon")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn aliases_parse_to_http_without_binding() {
    // --help short-circuits serving: aliases still resolve through clap.
    for alias in ["http", "HTTP", "streamable-http", "streamable_http"] {
        rism()
            .args(["mcp", "--transport", alias, "--port", "0", "--help"])
            .assert()
            .success();
    }
}

#[test]
fn stdio_door_with_net_flags_warns_and_serves_stdio() {
    // Warning on stderr, stdio door underneath: one initialize line must
    // still get a JSON-RPC answer on stdout, and NO port was opened.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rism"))
        .args(["mcp", "--transport", "stdio", "--port", "39999"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"clientInfo\":{\"name\":\"t\",\"version\":\"0\"},\"capabilities\":{}}}\n",
        )
        .expect("write");
    let out = child.wait_with_output().expect("reaped");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--port/--host are ignored with the stdio transport"),
        "warning present: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"jsonrpc\"") && stdout.contains("protocolVersion"),
        "stdio answered: {stdout}"
    );
    // Nothing ever listened on 39999.
    assert!(
        std::net::TcpStream::connect_timeout(
            &"127.0.0.1:39999".parse().expect("addr"),
            std::time::Duration::from_millis(300)
        )
        .is_err(),
        "stdio transport must not open the --port"
    );
}

#[test]
fn bare_mcp_flagless_stays_stdio() {
    // Row C: no flags = shipped stdio behavior, no HTTP, exit via stdin EOF.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rism"))
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("reaped");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("listening on http"),
        "no HTTP door: {stdout}"
    );
}

#[test]
fn non_loopback_bind_without_optout_refuses() {
    // SPEC T4: refuse at startup, clean stderr message, non-zero exit —
    // tested against a routable-looking placeholder (never actually bound).
    rism()
        .args([
            "mcp",
            "--transport",
            "http",
            "--host",
            "203.0.113.7",
            "--port",
            "0",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("--allow-all-interfaces")
                .and(predicate::str::contains("NO authentication")),
        );
}
