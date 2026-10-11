//! Live MCP-surface contract for background terminal work via MCP Tasks
//! (SEP-2663).
//!
//! Talks JSON-RPC over stdio to the built `rism mcp` binary — the exact
//! contract a Tasks-capable MCP client sees: schemas + annotations,
//! task-mode start (non-blocking), streamed growth between polls via
//! `tasks/get`, real server-side cancel via `tasks/cancel` with a frozen
//! final state, the sync-result equivalence of completed tasks, the
//! Rail-A refusal for non-declaring clients, cap/unknown-id errors.
//! Gated `#[ignore]`: CI runs it in the live-smoke job against a fresh
//! IRIS; locally, `cargo test -- --ignored` with a dev container up.
//!
//! The RPC client lives in `tests/support/mcp_harness.rs` (shared with
//! `tool_matrix` — one harness, not two forks).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    // json!() temporaries by value is the natural shape of a test RPC client
    clippy::needless_pass_by_value
)]

#[path = "support/mcp_harness.rs"]
mod harness;

use harness::*;

/// ~20 s of server-side work: 400 ticks, 50 ms apart (braces keep `h`
/// INSIDE the loop body — without them it completes instantly).
const LONG: &str = "for i=1:1:400 { write \"bg:\",i,!  h .05 }";

#[test]
fn message_count_parser_handles_both_shapes() {
    assert_eq!(Mcp::message_count("streamed 82 bytes"), 82);
    assert_eq!(
        Mcp::message_count("cancelled (interrupted), streamed 153 bytes"),
        153,
        "cancelled shape parsed positionally would yield 0"
    );
    assert_eq!(Mcp::message_count("completed"), 0);
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn annotations_and_task_surface_on_the_wire() {
    let mut m = Mcp::spawn();
    let msg = m.rpc("tools/list", serde_json::json!({}));
    let tools = msg["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 25, "tool count contract (Tasks migration)");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // The three custom poll-tools are GONE (full removal, no fallbacks).
    for removed in [
        "execute_command_background",
        "command_status",
        "command_cancel",
    ] {
        assert!(!names.contains(&removed), "{removed} must be removed");
    }
    let find = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .clone()
    };
    let ec = find("execute_command");
    assert_eq!(
        ec["annotations"]["destructiveHint"],
        serde_json::json!(true),
        "ObjectScript can mutate data"
    );
    assert!(
        ec["annotations"]["title"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "execute_command needs a title"
    );
    assert!(
        ec["inputSchema"]["properties"]["background"].is_object(),
        "background param must be in the schema"
    );
    // (advertise check lives in server_advertises_tasks_extension)
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn server_advertises_tasks_extension() {
    // A fresh HANDSHAKE is the proof: the initialize response must carry
    // the extension + correct serverInfo. (The inline client never
    // initializes, so use the handshake builder here.)
    let m =
        Mcp::spawn_with(serde_json::json!({"extensions": {"io.modelcontextprotocol/tasks": {}}}));
    let caps = &m.init["result"]["capabilities"];
    assert!(
        caps["extensions"]["io.modelcontextprotocol/tasks"].is_object(),
        "server must advertise the Tasks extension: {caps}"
    );
    assert!(
        caps["tools"].is_object(),
        "tools capability survives the manual get_info(): {caps}"
    );
    assert_eq!(m.init["result"]["serverInfo"]["name"], "rism");
    // version = env!("CARGO_PKG_VERSION"), not the old hard-coded 0.1.0
    let want = env!("CARGO_PKG_VERSION");
    assert_eq!(
        m.init["result"]["serverInfo"]["version"].as_str(),
        Some(want),
        "serverInfo.version must track the crate version"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn task_lifecycle_start_stream_cancel_frozen() {
    // start is non-blocking even for a ~20 s workload. Under the suite-
    // wide launch storm IRIS can refuse the task's child session (#6713)
    // ~2-3 s AFTER the handle (the handle itself is always `working` —
    // jobs::start registers before api::open), which would break the
    // "still running" poll below. The handle-shape contract is pinned on
    // an attempt whose task actually gets going: restart the whole start
    // if the first polls land #6713 (probe_alive class, storm not the
    // contract).
    let mut m = Mcp::spawn();
    let mut res;
    let mut attempt = 0u32;
    loop {
        let t_start = Instant::now();
        res = m.task_start(serde_json::json!({"command": LONG, "timeout_secs": 120}));
        assert!(t_start.elapsed() < Duration::from_secs(2), "start blocked");
        let id = Mcp::task_id(&res);
        std::thread::sleep(Duration::from_millis(1200));
        let early = m.task(&id);
        if early["status"] != "failed" {
            break;
        }
        assert!(
            early["error"]["message"]
                .as_str()
                .is_some_and(|msg| msg.contains("#6713")),
            "lifecycle task {id} failed for a NON-storm reason: {early}"
        );
        attempt += 1;
        assert!(attempt < 6, "still #6713 after 6 lifecycle starts: {early}");
        std::thread::sleep(Duration::from_millis(1500));
    }
    assert_eq!(res["status"], "working");
    let id = Mcp::task_id(&res);
    assert_eq!(
        res["ttlMs"],
        serde_json::json!(1_800_000),
        "retention as ttlMs"
    );
    assert!(res["pollIntervalMs"].is_number(), "poll cadence advertised");
    assert!(is_iso8601(res["createdAt"].as_str().unwrap()));

    // streaming: strictly more output between two polls while working
    std::thread::sleep(Duration::from_secs(2));
    let a = m.task(&id);
    assert_eq!(a["status"], "working", "still running");
    let ca = Mcp::streamed_count(&a);
    assert!(ca > 0, "output did not stream: {a}");
    std::thread::sleep(Duration::from_secs(2));
    let b = m.task(&id);
    let cb = Mcp::streamed_count(&b);
    assert!(cb > ca, "output did not grow between polls: {ca} -> {cb}");

    // cancel: real server-side interrupt, well before natural end
    let t0 = Instant::now();
    let c = m.task_cancel(&id);
    assert!(c.get("error").is_none(), "tasks/cancel failed: {c}");
    // Item 5a: the ACK is a TaskAckResult — `resultType: "complete"` and
    // NOTHING else (no taskId/status: state changes are observed via the
    // next tasks/get). "no error" alone passed vacuously on `{}`.
    assert_eq!(
        c["result"]["resultType"],
        serde_json::json!("complete"),
        "cancel ack shape: {c}"
    );
    assert_eq!(
        c["result"]
            .as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
            .unwrap_or_default(),
        vec!["resultType"],
        "cancel ack must carry ONLY resultType: {c}"
    );
    let done = m.wait_terminal(&id, Duration::from_secs(15));
    assert!(t0.elapsed() < Duration::from_secs(10), "cancel too slow");
    assert_eq!(done["status"], "cancelled");
    assert!(
        done.get("result").is_none() && done.get("error").is_none(),
        "cancelled payload carries neither result nor error: {done}"
    );
    assert!(done["lastUpdatedAt"].as_str().is_some_and(is_iso8601));

    // frozen state: output stops growing (loop truly dead server-side)
    let chars = Mcp::streamed_count(&done);
    std::thread::sleep(Duration::from_secs(2));
    let later = m.task(&id);
    assert_eq!(
        Mcp::streamed_count(&later),
        chars,
        "server-side loop still ticking after cancel"
    );
    // cancel on a terminal task: safe no-op
    let c2 = m.task_cancel(&id);
    assert!(c2.get("error").is_none(), "re-cancel must not error: {c2}");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn quick_task_completes_with_the_sync_result() {
    let mut m = Mcp::spawn();
    // MULTI-frame on purpose: each `write !` inside the loop is a separate
    // terminal frame, and the sync path joins frames with \n while the
    // streaming tail concatenates them raw. A single-frame command would
    // pass both paths identically and mask the divergence (it did, once).
    let cmd = "for i=1:1:4 { write \"quick\",i,!  h .15 }";
    let id = m.task_start_live(serde_json::json!({"command": cmd}));
    let done = m.wait_terminal(&id, Duration::from_secs(15));
    assert_eq!(done["status"], "completed");
    assert_eq!(done["resultType"], "complete");
    // Spec MUST: the completed payload is the EXACT CallToolResult the
    // synchronous call would have returned — prove it by running the sync
    // command and diffing the two CallToolResults.
    let payload = &done["result"];
    assert_eq!(payload["isError"], serde_json::json!(false));
    let task_out: Value =
        serde_json::from_str(payload["content"][0]["text"].as_str().unwrap()).unwrap();
    let sync_out = m.call_survives_storm("execute_command", serde_json::json!({"command": cmd}));
    assert_eq!(
        task_out["output"], sync_out["output"],
        "task result must equal the sync result"
    );
    assert_eq!(task_out["namespace"], sync_out["namespace"]);
    assert!(task_out["output"].as_str().unwrap().contains("quick"));
    assert!(
        task_out["prompt"]
            .as_str()
            .unwrap_or_default()
            .contains('>')
    );
    // task bookkeeping survives completion
    assert!(done["createdAt"].as_str().is_some_and(is_iso8601));
    assert!(is_iso8601(done["lastUpdatedAt"].as_str().unwrap()));
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn rail_a_and_bad_task_ids_are_errors_not_crashes() {
    // Rail A: non-declaring client asking for background gets an honest
    // tool-level error (never a silent 60-s timeout).
    let mut plain = Mcp::spawn_plain();
    let (is_err, _, text) = plain.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true}),
    );
    assert!(is_err, "background must refuse for non-declaring clients");
    assert!(
        text.lowercase_contains("tasks extension"),
        "refusal must name the Tasks extension: {text}"
    );
    // Rail-A fires BEFORE schema validation: even a payload that would
    // fail deny_unknown_fields gets the guidance, not a bare -32602
    // (a declared client with the same broken payload still gets -32602
    // — the parse branch runs only inside the declared path).
    let (is_err, _, text) = plain.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true, "force": true}),
    );
    assert!(is_err);
    assert!(
        text.lowercase_contains("tasks extension"),
        "schema-invalid + non-declaring must still get Rail-A: {text}"
    );
    // tasks/* methods from a non-declaring client: protocol error (rmcp
    // validates the capability before the handler runs).
    let msg = plain.rpc("tasks/get", serde_json::json!({"taskId": "whatever"}));
    assert!(msg.get("error").is_some(), "must reject: {msg}");
    drop(plain);

    // Declared client: unknown ids get a clean JSON-RPC error with the
    // task-facing phrasing.
    let mut m = Mcp::spawn();
    let msg = m.rpc("tasks/get", serde_json::json!({"taskId": "bogus"}));
    assert!(msg.get("error").is_some());
    let get_err = msg["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        get_err.lowercase_contains("unknown or expired"),
        "hint missing: {msg}"
    );
    let msg = m.rpc("tasks/cancel", serde_json::json!({"taskId": "bogus"}));
    assert!(msg.get("error").is_some());
    // 7d: cancel must carry the SAME unknown/expired hint as get (shared
    // task_error code) — a bare "unknown job id" leak would pass silently.
    assert!(
        msg["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string()
            .lowercase_contains("unknown or expired"),
        "cancel hint missing: {msg}"
    );
    // tasks/update: never offers input — honest refusal, not a hang.
    // Params MUST use the spec shape (inputResponses); a stale-shape
    // payload would fail at deserialization and make this assert pass for
    // the wrong reason.
    let msg = m.rpc(
        "tasks/update",
        serde_json::json!({"taskId": "bogus", "inputResponses": {}}),
    );
    assert!(msg.get("error").is_some(), "update_task must refuse: {msg}");
    assert!(
        msg["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("never request input"),
        "refusal must be the honest handler answer, not a parse error: {msg}"
    );
    // unknown field rejected (deny_unknown_fields)
    let (is_err, _, _) = m.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "force": true}),
    );
    assert!(is_err, "deny_unknown_fields must reject");
    // missing required command rejected
    let (is_err, _, _) = m.call("execute_command", serde_json::json!({}));
    assert!(is_err);
    // server still healthy (storm-retried: a #6713 on this probe is the
    // shared-IRIS child-slot transient, not a door death — probe_alive class)
    let p = m.call_survives_storm(
        "execute_command",
        serde_json::json!({"command": "write \"healthy\",!"}),
    );
    assert!(p["output"].as_str().unwrap().contains("healthy"));
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn handshake_declared_extension_key_is_inert() {
    // SEP-2663 compat table: under protocol 2025-11-25 the
    // `io.modelcontextprotocol/tasks` extension is NOT defined — a
    // declaring client MUST be treated as non-declaring (that version has
    // a different, not-wire-compatible tasks spec). rmcp 3.4.1 validates
    // ONLY the capability (`supports_tasks()`), never the version, so the
    // version clause is Rism's own gate (`tasks_supported`).
    //
    // This is the DEFAULT handshake shape: rmcp 3.4.1 cannot negotiate
    // 2026-06-30 over initialize (its V_LATEST is 2025-11-25 — probed
    // live). A handshake client declaring the key therefore exercises the
    // inert path, and the modern path is covered by every inline test
    // (spawn() = SEP-2575 per-request `_meta`, 2026-07-28).
    let mut m =
        Mcp::spawn_with(serde_json::json!({"extensions": {"io.modelcontextprotocol/tasks": {}}}));
    // If rmcp ever learns to echo 2026-06-30+ here, this test's premise
    // (pre-extension session) dies — fail loudly instead of vacuously.
    assert!(
        m.version.as_str() < "2026-06-30",
        "negotiated {} is already extension-era: gate would be vacuous here — \
         move these assertions to the inline client",
        m.version
    );
    let (is_err, _, text) = m.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true}),
    );
    assert!(
        is_err && text.contains("Tasks extension"),
        "declared-but-legacy client must get the Rail-A refusal, not a task \
         handle (would leak a CreateTaskResult its version cannot define): \
         err={is_err} text={text}"
    );
    // The refusal must name the version requirement (honest guidance).
    assert!(
        text.contains("2026-06-30"),
        "Rail-A text must state the protocol floor: {text}"
    );
    // tasks/* likewise rejected — never a DetailedTask.
    let msg = m.rpc("tasks/get", serde_json::json!({"taskId": "nope"}));
    assert!(
        msg.get("error").is_some(),
        "legacy session must reject tasks/get: {msg}"
    );
    assert_eq!(
        msg["error"]["code"],
        serde_json::json!(-32021),
        "-32021 expected"
    );
    // sync door still fully healthy on the legacy session. Retried like
    // probe_alive in the matrix: 17 parallel tests hammer the shared IRIS
    // child-process slots and a transient #6713 "Start target failed"
    // says nothing about the Rail-A gate under test — eventual success
    // is the honest contract.
    let mut last = String::new();
    let mut healthy = false;
    for _ in 0..10 {
        let (is_err, p, text) = m.call(
            "execute_command",
            serde_json::json!({"command": "write 7", "timeout_secs": 30}),
        );
        healthy = !is_err && p["output"].as_str().is_some_and(|s| s.contains('7'));
        last = if is_err { text } else { p.to_string() };
        if healthy {
            break;
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    assert!(healthy, "legacy session sync door never came back: {last}");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn inline_task_ids_are_high_entropy() {
    // SEP-2663 Security MUST: ids unguessable. Two live tasks in one
    // session: suffixes differ and neither is a counter step (the old
    // SEQ bug: id N+1 was trivially predictable from id N).
    let mut m = Mcp::spawn();
    let a = m.task_start(serde_json::json!({"command": "write 1", "timeout_secs": 30}));
    let b = m.task_start(serde_json::json!({"command": "write 2", "timeout_secs": 30}));
    let (ia, ib) = (Mcp::task_id(&a), Mcp::task_id(&b));
    assert_ne!(ia, ib);
    let sa = ia.rsplit('-').next().unwrap();
    let sb = ib.rsplit('-').next().unwrap();
    assert_eq!(sa.len(), 16, "64-bit hex suffix: {ia}");
    assert_ne!(sa, sb);
    for id in [&ia, &ib] {
        let t = m.wait_terminal(id, Duration::from_secs(15));
        // The contract is id UNPREDICTABILITY (asserted above), which the
        // handles already prove. A transient #6713 launch failure (child-
        // slot storm from parallel tests — same class probe_alive tolerates)
        // still ends the task with an honest error, never wedges the pin.
        let st = t["status"].as_str().unwrap_or_default();
        assert!(
            st == "completed"
                || (st == "failed"
                    && t["error"]["message"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())),
            "task {id} must end completed (or failed with an honest error under the \
             launch storm): {t}"
        );
    }
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn every_call_tool_path_is_logged_prism_parity() {
    let secs = Duration::from_secs(5);
    // The call_tool doc promises "every tool call is logged" on EVERY path —
    // including the early-return branches that never reach the router tail.
    // Path 1: Rail-A refusal (non-declaring client) must log REQUEST and a
    // RESPONSE carrying the refusal text.
    let mut m1 = Mcp::spawn_plain();
    let (is_err, _, _) = m1.call(
        "execute_command",
        serde_json::json!({"command": "write 1", "background": true}),
    );
    assert!(is_err);
    assert!(
        m1.wait_stderr_contains("── execute_command ── RESPONSE", secs),
        "Rail-A refusal path logged nothing:\n{}",
        m1.stderr_text()
    );
    let logs = m1.stderr_text();
    assert!(
        logs.contains("── execute_command ── REQUEST"),
        "Rail-A refusal path logged no REQUEST:\n{logs}"
    );
    assert!(
        logs.contains("Tasks extension"),
        "Rail-A refusal RESPONSE must carry the guidance:\n{logs}"
    );
    drop(m1);

    // Path 2: task-mode spawn (returns CallToolResponse::Task before the
    // router) — REQUEST banner + RESPONSE banner carrying the task handle.
    let mut m = Mcp::spawn();
    let res =
        m.task_start(serde_json::json!({"command": "write \"logged\",!", "timeout_secs": 30}));
    let id = Mcp::task_id(&res);
    assert!(
        m.wait_stderr_contains(&format!("\"taskId\": \"{id}\""), secs),
        "task-spawn RESPONSE log missing the task handle:\n{}",
        m.stderr_text()
    );
    let _ = m.wait_terminal(&id, Duration::from_secs(15));
    let logs = m.stderr_text();
    assert!(
        logs.contains("── execute_command ── REQUEST"),
        "task-spawn path logged no REQUEST:\n{logs}"
    );
    // Path 3: the normal router tail logs exactly one pair per call —
    // never zero and never duplicated by the hoist. The sync call is
    // retried like probe_alive in the matrix: a transient #6713 under the
    // suite-wide launch storm still logs ONE legitimate pair (the router
    // tail logs the RESPONSE even for an isError result), so the honest
    // contract is one pair per call, not the literal count 2.
    let mut sync_calls = 0usize;
    let mut healthy = false;
    for _ in 0..10 {
        sync_calls += 1;
        let (is_err, p, _) = m.call(
            "execute_command",
            serde_json::json!({"command": "write 2", "timeout_secs": 30}),
        );
        healthy = !is_err && p["output"].as_str().is_some_and(|s| s.contains('2'));
        if healthy {
            break;
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    assert!(
        healthy,
        "sync door never came back after {sync_calls} tries"
    );
    // The RESPONSE banner arrives on stderr asynchronously (different fd
    // than the stdout frame the client just read), and the REQUEST banner
    // body already contains "write 2" in its params — so wait on the
    // banner COUNT reaching its target, not on a text needle.
    let want = 1 + sync_calls;
    let deadline = Instant::now() + secs;
    loop {
        let logs = m.stderr_text();
        let req = logs.matches("── execute_command ── REQUEST").count();
        let resp = logs.matches("── execute_command ── RESPONSE").count();
        if (req, resp) == (want, want) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "banner pairs never settled (want {want} each): REQUEST={req} RESPONSE={resp}\n{logs}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        m.stderr_text().contains("write 2"),
        "call never logged: {}",
        m.stderr_text()
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn running_cap_enforced_then_releasable() {
    let mut m = Mcp::spawn();
    let mut live = Vec::new();
    // ~15 s jobs so they outlive the start loop itself
    for _ in 0..16 {
        let res = m.task_start(serde_json::json!({
            "command": "for j=1:1:300 { write \"cap:\",j,!  h .05 }",
            "timeout_secs": 60
        }));
        live.push(Mcp::task_id(&res));
    }
    // the 17th must be refused, actionably (task-creation refusal is a
    // JSON-RPC error — there is no task to carry it)
    let msg = m.rpc(
        "tools/call",
        serde_json::json!({
            "name": "execute_command",
            "arguments": {"command": "write 1", "timeout_secs": 60, "background": true}
        }),
    );
    let err_text = msg
        .get("error")
        .map(|e| e["message"].as_str().unwrap_or_default().to_lowercase())
        .unwrap_or_default();
    assert!(
        !err_text.is_empty(),
        "cap not enforced (17th started): {msg}"
    );
    assert!(
        err_text.contains("too many"),
        "cap error must say too many: {err_text}"
    );
    // releases: cancelling frees slots (cancel-all, verify all terminal)
    for id in &live {
        let c = m.task_cancel(id);
        assert!(c.get("error").is_none(), "cancel {id} failed: {c}");
    }
    for id in &live {
        let t = m.wait_terminal(id, Duration::from_secs(20));
        // `cancelled` (cancel won) or `failed` with an honest error (IRIS
        // lost the child-slot at launch under the 16-wide storm — #6713
        // transient, same class the churn test tolerates). The release
        // contract is that the slot ENDS and frees, not which error won.
        let st = t["status"].as_str().unwrap_or_default();
        assert!(
            st == "cancelled"
                || (st == "failed"
                    && t["error"]["message"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())),
            "task {id} must end cancelled (or failed with an honest error \
             under the launch storm): {t}"
        );
    }
    // A freed slot lets a fresh start RUN to completion (retry: after a
    // 16-cancel storm IRIS needs a beat to release child-process slots —
    // eventual success is the honest contract, per probe_alive precedent).
    let mut done = Value::Null;
    for _ in 0..12 {
        let res = m.task_start(serde_json::json!({
            "command": "write \"slot-free\",!",
            "timeout_secs": 30
        }));
        let id = Mcp::task_id(&res);
        done = m.wait_terminal(&id, Duration::from_secs(15));
        if done["status"] != "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(
        done["status"], "completed",
        "slot not reclaimed after cancels"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Round 5 — re-validation regression pins (see testing-notes ledger)
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn legacy_tasks_result_answers_32601() {
    // SEP-2663 compat: the legacy `tasks/result` method is NOT a named
    // request in rmcp 3.4.1 — it dispatches as CustomRequest and the
    // default on_custom_request MUST answer -32601 (never -32602/-32603,
    // never a closed connection), on BOTH session shapes.
    let mut m = Mcp::spawn();
    let res = m.task_start(serde_json::json!({"command": "write 1", "timeout_secs": 30}));
    let id = Mcp::task_id(&res);
    let msg = m.rpc("tasks/result", serde_json::json!({"taskId": &id}));
    assert!(
        msg.get("result").is_none(),
        "tasks/result must not answer: {msg}"
    );
    assert_eq!(
        msg["error"]["code"],
        serde_json::json!(-32601),
        "inline door: {msg}"
    );
    // server survives the unknown method (no silence, no crash; storm-
    // retried health probe — a #6713 transient here is not a door death)
    let p = m.call_survives_storm("execute_command", serde_json::json!({"command": "write 1"}));
    assert!(
        p["output"].is_string(),
        "inline door died after tasks/result: {p}"
    );
    drop(m);

    // Handshake door (rmcp negotiates <= 2025-11-25): same expectation —
    // the tasks/* -32021 gate never even runs (not a tasks variant).
    let mut h =
        Mcp::spawn_with(serde_json::json!({"extensions": {"io.modelcontextprotocol/tasks": {}}}));
    let msg = h.rpc("tasks/result", serde_json::json!({"taskId": "x"}));
    assert!(
        msg.get("result").is_none(),
        "tasks/result must not answer: {msg}"
    );
    assert_eq!(
        msg["error"]["code"],
        serde_json::json!(-32601),
        "handshake door: {msg}"
    );
    let msg = h.rpc("tools/list", serde_json::json!({}));
    assert!(msg.get("result").is_some(), "handshake door died: {msg}");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn cancel_at_completion_boundary_stays_coherent() {
    // Item 3a: `tasks/cancel` landing in the last milliseconds of a
    // running task — the interrupt reaches a child that already exited,
    // and IRIS answers the interrupt with `ERROR #6704: Target has exited
    // debugger` BEFORE the prompt frame. That error must resolve to
    // `cancelled` (or a clean `completed` if the finish won the race),
    // NEVER `failed` with the debugger's internal error (round-5 sweep
    // hit it 2/26 rounds). Sweep delays across the natural end so BOTH
    // branches (pre/post completion) are exercised — a single delay would
    // pass the contract vacuously on one outcome only.
    let mut m = Mcp::spawn();
    let mut saw_completed = false;
    let mut saw_cancelled = false;
    let mut round = 0u32;
    let mut storm_retries = 0u32;
    while round < 8 {
        let res = m.task_start(serde_json::json!({
            "command": "for i=1:1:40 { write \"b:\",i,!  h .05 }",
            "timeout_secs": 60
        }));
        let id = Mcp::task_id(&res);
        // ~2.0 s of work; sweep 1.85..2.35 s across the boundary
        std::thread::sleep(Duration::from_millis(1850 + u64::from(round) * 70));
        let c = m.task_cancel(&id);
        assert!(c.get("error").is_none(), "cancel on live id must ACK: {c}");
        let done = m.wait_terminal(&id, Duration::from_secs(20));
        let st = done["status"].as_str().unwrap_or_default();
        match st {
            "completed" => {
                saw_completed = true;
                round += 1;
                assert!(
                    done.get("result").is_some(),
                    "completed carries result: {done}"
                );
                assert!(
                    done.get("error").is_none(),
                    "completed has no error: {done}"
                );
            }
            "cancelled" => {
                saw_cancelled = true;
                round += 1;
                assert!(
                    done.get("result").is_none(),
                    "cancelled carries no result: {done}"
                );
                assert!(
                    done.get("error").is_none(),
                    "cancelled has no error: {done}"
                );
            }
            "failed"
                if done["error"]["message"]
                    .as_str()
                    .is_some_and(|s| s.contains("#6713"))
                    && storm_retries < 20 =>
            {
                // IRIS lost the child-slot at LAUNCH under the suite-wide
                // storm (#6713 transient, same class the churn test and
                // probe_alive tolerate): the round never crossed the
                // boundary, so REPLAY this delay rather than pretend it
                // exercised the race. Honest error = coherent either way.
                assert!(done.get("result").is_none(), "failed has no result: {done}");
                storm_retries += 1;
            }
            other => panic!("boundary cancel must land completed|cancelled, got {other}: {done}"),
        }
    }
    assert!(
        saw_completed && saw_cancelled,
        "race sweep must cross the boundary (completed={saw_completed} cancelled={saw_cancelled} \
         storm_retries={storm_retries})"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn failed_task_carries_honest_error_payload() {
    // Item 5c: a background job whose session cannot open (unknown
    // namespace) must land `failed` with a JSON-RPC-style error object
    // (code + message) and an honest statusMessage — never a silent
    // `completed`, never the "streamed N bytes" working shape.
    let mut m = Mcp::spawn();
    let msg = m.rpc(
        "tools/call",
        serde_json::json!({
            "name": "execute_command",
            "arguments": {"command": "write 1", "timeout_secs": 30,
                          "background": true, "namespace": "RISM-NOPE-9"}
        }),
    );
    let id = msg["result"]["taskId"].as_str().unwrap().to_string();
    let done = m.wait_terminal(&id, Duration::from_secs(20));
    assert_eq!(done["status"], "failed", "bad namespace must fail: {done}");
    assert_eq!(done["statusMessage"], "failed", "message must match status");
    assert!(
        done.get("result").is_none(),
        "failed must not carry result: {done}"
    );
    let err = &done["error"];
    assert!(err["code"].is_number(), "failed payload needs code: {err}");
    assert!(
        err["message"].as_str().is_some_and(|s| !s.is_empty()),
        "failed payload needs a non-empty message: {err}"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn task_final_result_truncation_matches_sync_door() {
    // Item 6: the task door's completed `result` must equal the sync door
    // byte-for-byte EVEN when the cap cuts mid-multibyte (the general
    // case the constitution demands): 120 Cyrillic chars against a
    // 64-byte bound -> 32 chars / 64 bytes retained, 88 CHARS omitted.
    let cmd = "set s=\"\" for i=1:1:40 { set s=s_$c(1040,1041,1042) } write s";
    let mut m = Mcp::spawn_with_env(&[("RISM_TERMINAL_MAX_OUTPUT_CHARS", "64")]);
    let id = m.task_start_live(serde_json::json!({"command": cmd, "timeout_secs": 60}));
    let done = m.wait_terminal(&id, Duration::from_secs(30));
    assert_eq!(done["status"], "completed", "{done}");
    let task_text = done["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let payload: Value = serde_json::from_str(&task_text).unwrap();
    let out = payload["output"].as_str().unwrap_or_default();
    assert_eq!(
        payload["output_truncated"],
        serde_json::json!(true),
        "{payload}"
    );
    assert!(
        out.chars().count() == 32 && out.len() == 64,
        "task door must retain 32 chars / 64 bytes: {} chars {} bytes",
        out.chars().count(),
        out.len()
    );
    assert_eq!(
        payload["output_omitted_chars"],
        serde_json::json!(88),
        "omitted must be CHARS (bytes math would say 176): {}",
        payload["output_omitted_chars"]
    );
    // Same session, sync door: byte-equal payloads generalize the
    // equivalence claim to the multibyte straddle. Raw text (not the
    // parsed payload) is compared, so the retry loop lives here rather
    // than in call_survives_storm — a #6713 on the storm-hit door is a
    // transient, not a divergence in the truncation math.
    let mut sync_text = String::new();
    let mut synced = false;
    for _ in 0..10 {
        let (is_err, _, text) = m.call("execute_command", serde_json::json!({"command": cmd}));
        sync_text = text;
        synced = !is_err;
        if synced {
            break;
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    assert!(synced, "sync door never came back: {sync_text}");
    assert_eq!(
        task_text, sync_text,
        "task result must be the sync result verbatim"
    );
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn get_racing_cancel_both_ids_answer() {
    // Item 3b (refactorer): tasks/get and tasks/cancel for the SAME live
    // task sent back-to-back WITHOUT waiting — rmcp may answer out of
    // order, so every interleaved id must still be RECOVERED (harness
    // park-lot) and answered. No id may be lost; no silence.
    let mut m = Mcp::spawn();
    let res = m.task_start(serde_json::json!({
        "command": "for i=1:1:400 { write \"rc:\",i,!  h .05 }",
        "timeout_secs": 120
    }));
    let id = Mcp::task_id(&res);
    std::thread::sleep(Duration::from_millis(800));
    let get_id = m.send_request("tasks/get", serde_json::json!({"taskId": &id}));
    let cancel_id = m.send_request("tasks/cancel", serde_json::json!({"taskId": &id}));
    let cancel = m
        .rpc_take(cancel_id, Duration::from_secs(10))
        .expect("tasks/cancel went silent behind the interleaved get");
    assert!(
        cancel.get("error").is_none(),
        "cancel on a live id must ACK: {cancel}"
    );
    let get = m
        .rpc_take(get_id, Duration::from_secs(10))
        .expect("tasks/get went silent");
    let st = get["result"]["status"].as_str().unwrap_or_default();
    assert!(
        st == "working" || st == "cancelled",
        "get during the race answers a coherent state, got {get}"
    );
    let done = m.wait_terminal(&id, Duration::from_secs(20));
    // The race itself: `cancelled` (cancel won) — or `failed` with an
    // honest error when IRIS loses the child-slot at launch under the
    // suite-wide storm (#6713 transient, same class probe_alive retry
    // tolerates in the matrix). NEVER wedged-working, NEVER a wire shape
    // with result AND error — coherence is the contract, not the winner.
    let st = done["status"].as_str().unwrap_or_default();
    assert!(
        st == "cancelled" || st == "failed",
        "cancel-raced task must end cancelled|failed: {done}"
    );
    if st == "cancelled" {
        assert!(done.get("result").is_none() && done.get("error").is_none());
    } else {
        assert!(
            done["error"]["message"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "failed needs an honest message: {done}"
        );
    }
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn cap_churn_concurrent_starts_and_cancels_all_answer() {
    // Item 3c (refactorer): the CONCURRENT version of the serial
    // running_cap test — cancel 8 slots WITHOUT waiting for the acks,
    // then start 9 more mid-flight (slots may not be freed yet). Every
    // start must ANSWER — task handle or an actionable "too many" —
    // never silence; the 16-cap must hold at every snapshot; the server
    // never wedges (tools/list answers at every step).
    let mut m = Mcp::spawn();
    let mut live = Vec::new();
    for _ in 0..16 {
        let res = m.task_start(serde_json::json!({
            "command": "for j=1:1:300 { write \"churn:\",j,!  h .05 }",
            "timeout_secs": 60
        }));
        live.push(Mcp::task_id(&res));
    }
    for id in &live[..8] {
        m.send_request("tasks/cancel", serde_json::json!({"taskId": id}));
    }
    // pipelined burst of 9 starts while 8 cancels are landing
    let mut pending = Vec::new();
    for _ in 0..9 {
        pending.push(m.send_request(
            "tools/call",
            serde_json::json!({
                "name": "execute_command",
                "arguments": {"command": "for j=1:1:300 { write \"late:\",j,!  h .05 }",
                              "timeout_secs": 60, "background": true}
            }),
        ));
    }
    let mut started = Vec::new();
    for p in pending {
        let msg = m
            .rpc_take(p, Duration::from_secs(15))
            .expect("start went silent");
        if msg["result"]["resultType"] == serde_json::json!("task") {
            started.push(Mcp::task_id(&msg));
        } else {
            let err = msg["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase();
            assert!(
                err.contains("too many"),
                "refusal must be actionable: {msg}"
            );
        }
    }
    // CAP INVARIANT under churn: a snapshot of all 25 ids never shows
    // more than 16 `working` (admission checks the cap under the registry
    // lock; transient #6713 start failures only LOWER the count).
    let all: Vec<&String> = live.iter().chain(started.iter()).collect();
    let mut working = 0usize;
    for id in &all {
        if m.task(id)["status"] == serde_json::json!("working") {
            working += 1;
        }
    }
    assert!(
        working <= 16,
        "cap broken under churn: {working} working at once"
    );
    // every churned task ends TERMINALLY and coherently — `cancelled`
    // (cancel won) or `failed` with an honest error (a #6713 child-slot
    // transient during the 16-wide launch storm — the same contention
    // probe_alive tolerates in the matrix). NEVER `working` forever,
    // NEVER cancelled-with-result.
    for id in live.iter().take(8) {
        let t = m.wait_terminal(id, Duration::from_secs(25));
        assert!(
            t["status"] == serde_json::json!("cancelled")
                || t["status"] == serde_json::json!("failed"),
            "churned cancel {id} must land cancelled|failed: {t}"
        );
        assert!(
            t["status"] != serde_json::json!("cancelled") || t.get("result").is_none(),
            "cancelled carries no result: {t}"
        );
    }
    for id in live.iter().skip(8).chain(started.iter()) {
        let c = m.task_cancel(id);
        assert!(c.get("error").is_none(), "cleanup cancel {id}: {c}");
    }
    for id in live.iter().skip(8).chain(started.iter()) {
        let t = m.wait_terminal(id, Duration::from_secs(25));
        assert!(
            t["status"] != serde_json::json!("working"),
            "churned job {id} never ended: {t}"
        );
    }
    // a freed slot lets a fresh start RUN to COMPLETION (retry: after a
    // 25-child storm IRIS needs a beat to release child-process slots —
    // eventual success is the honest contract, per probe_alive precedent).
    let mut done = Value::Null;
    for _ in 0..12 {
        let res = m.task_start(serde_json::json!({
            "command": "write \"slot-reopened\",!",
            "timeout_secs": 30
        }));
        let reopened = Mcp::task_id(&res);
        done = m.wait_terminal(&reopened, Duration::from_secs(15));
        if done["status"] != "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(done["status"], "completed", "freed slot unusable: {done}");
    let msg = m.rpc("tools/list", serde_json::json!({}));
    assert!(msg.get("result").is_some(), "server wedged by the churn");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn cancel_racing_wall_timeout_stays_coherent() {
    // Item 3 (boundary ms): the cancel-vs-timeout race cannot be timed
    // deterministically, so pin the OBSERVABLE invariants instead: a
    // cancel landing at ~95% of the deadline must end the task
    // TERMINALLY — `cancelled` (cancel won) or `failed` with a
    // non-empty honest error (timeout won) — never `completed`, never a
    // wedged `working`, never a both-flags wire shape.
    let mut m = Mcp::spawn();
    let mut saw_cancelled = false;
    let mut saw_failed = false;
    for round in 0..6 {
        let res = m.task_start(serde_json::json!({
            "command": "for i=1:1:80 { write \"w:\",i,!  h .05 }",
            "timeout_secs": 3
        }));
        let id = Mcp::task_id(&res);
        std::thread::sleep(Duration::from_millis(2400 + round * 100));
        let c = m.task_cancel(&id);
        assert!(c.get("error").is_none(), "cancel on live id must ACK: {c}");
        let done = m.wait_terminal(&id, Duration::from_secs(25));
        match done["status"].as_str().unwrap_or_default() {
            "cancelled" => {
                saw_cancelled = true;
                assert!(
                    done.get("result").is_none() && done.get("error").is_none(),
                    "cancelled carries neither result nor error: {done}"
                );
            }
            "failed" => {
                saw_failed = true;
                assert!(
                    done["error"]["message"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()),
                    "timeout-won failure needs an honest message: {done}"
                );
            }
            other => panic!("boundary race must land cancelled|failed, got {other}: {done}"),
        }
    }
    assert!(
        saw_cancelled || saw_failed,
        "sweep observed no terminal outcome"
    );
    let msg = m.rpc("tools/list", serde_json::json!({}));
    assert!(msg.get("result").is_some(), "server died in the race");
}

#[test]
#[ignore = "requires live IRIS (RISM_IRIS_BASE_URL + config.toml)"]
fn task_read_frame_and_objectscript_error_semantics() {
    // Item 4 (cross-boundary): a task command that hits `read` must get
    // the server-side EMPTY auto-answer (JobHooks::read_prompt) and
    // COMPLETE — never wedge to the wall timeout (Prism lesson: never
    // drop a read frame). And a command whose ObjectScript fails with a
    // trap (`<NOROUTINE>`) is a COMPLETED task carrying the error text —
    // `failed` is reserved for transport/session errors (the two doors
    // must agree; the sync door has always returned the trap text).
    let mut m = Mcp::spawn();
    let id = m.task_start_live(serde_json::json!({
        "command": "read x  write \"after-read\",x,!",
        "timeout_secs": 30
    }));
    let done = m.wait_terminal(&id, Duration::from_secs(25));
    assert_eq!(
        done["status"], "completed",
        "read-frame task must auto-answer and complete, not time out: {done}"
    );
    assert!(
        done["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("after-read")),
        "auto-answered read must let the command finish: {done}"
    );
    let id = m.task_start_live(serde_json::json!({
        "command": "do ^RismNoSuchRoutine5",
        "timeout_secs": 30
    }));
    let done = m.wait_terminal(&id, Duration::from_secs(25));
    assert_eq!(
        done["status"], "completed",
        "an ObjectScript trap is program output, not a task failure: {done}"
    );
    assert!(
        done["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("NOROUTINE")),
        "trap text must be in the result output: {done}"
    );
    let sync = m.call_survives_storm(
        "execute_command",
        serde_json::json!({"command": "do ^RismNoSuchRoutine5"}),
    );
    let task_payload: Value =
        serde_json::from_str(done["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        task_payload["output"], sync["output"],
        "trap semantics must match the sync door"
    );
}
