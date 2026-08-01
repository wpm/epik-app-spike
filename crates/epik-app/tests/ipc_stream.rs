//! The IPC layer end to end: a stub engine speaking stream-json, a real
//! `epik_core::Session` reading it, and the app's real event pump translating the
//! result.
//!
//! The stub is a shell script emitting the exact line format the CLI emits,
//! captured from a live CLI in `epik-core`'s protocol tests. So this exercises
//! parse → translate → coalesce → deliver, with a real child process, no CLI, no
//! API key, and no network.
//!
//! The sink is a recording buffer rather than a `tauri::ipc::Channel` because a
//! `Channel` cannot be constructed outside a running Tauri app. The pump is the
//! same function the app runs, and `Channel`'s `EventSink` impl is a one-line
//! forward, so what is untested here is that one line rather than the ordering
//! and coalescing logic this is about.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use epik_app::pump::{DELTA_FLUSH, EventSink, pump};
use epik_core::{PermissionPolicy, PermissionRule, Session, SessionConfig, SessionEvent};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<SessionEvent>>>);

impl Recorder {
    fn events(&self) -> Vec<SessionEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for Recorder {
    fn send(&self, event: SessionEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

/// Write an executable stub engine and return its path.
fn stub_engine(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("epik-ipc-stub-{name}"));
    let mut file = std::fs::File::create(&path).expect("create stub engine");
    write!(file, "#!/bin/sh\n{body}").expect("write stub engine");
    file.flush().expect("flush stub engine");
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod stub engine");
    path
}

fn config(engine: PathBuf) -> SessionConfig {
    SessionConfig {
        claude_bin: engine.to_string_lossy().into_owned(),
        ..SessionConfig::default()
    }
}

/// One `stream_event` line carrying a text delta, as the CLI emits them.
fn delta_line(text: &str) -> String {
    format!(
        r#"echo '{{"type":"stream_event","session_id":"s1","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{text}"}}}}}}'"#
    )
}

/// Run a stub engine through the real pump and return the events the frontend
/// would have seen, in order.
async fn observe(config: SessionConfig, turn: Option<&str>) -> Vec<SessionEvent> {
    let mut session = Session::spawn(config).await.expect("spawn stub engine");
    let handle = session.handle();
    if let Some(text) = turn {
        handle.send_user(text).await.expect("send the user turn");
    }
    let events = std::mem::replace(&mut session.events, tokio::sync::mpsc::channel(1).1);
    let sink = Recorder::default();
    let pumping = tokio::spawn(pump(events, sink.clone()));

    tokio::time::timeout(Duration::from_secs(20), pumping)
        .await
        .expect("the pump did not finish in time")
        .expect("the pump panicked");
    sink.events()
}

/// A full turn: init, streaming text, the final block, a tool call and its
/// result, then the result line. What the frontend has to render.
#[tokio::test]
async fn a_full_turn_arrives_in_order_with_deltas_coalesced() {
    let script = format!(
        r#"
# Consume the initialize control request and the user turn so the ordering below
# is the stub's rather than a race with our writes.
read -r _init
read -r _turn
echo '{{"type":"system","subtype":"init","session_id":"s1","model":"claude-sonnet-5","claude_code_version":"2.1.220","tools":["Bash","Read"]}}'
{d1}
{d2}
{d3}
echo '{{"type":"assistant","session_id":"s1","message":{{"role":"assistant","content":[{{"type":"text","text":"Let me look."}}]}}}}'
echo '{{"type":"assistant","session_id":"s1","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_01","name":"Bash","input":{{"command":"ls"}}}}]}}}}'
echo '{{"type":"user","session_id":"s1","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_01","content":"a.txt","is_error":false}}]}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.0123,"num_turns":2,"session_id":"s1"}}'
"#,
        d1 = delta_line("Let "),
        d2 = delta_line("me "),
        d3 = delta_line("look."),
    );
    let engine = stub_engine("full-turn", &script);
    let events = observe(config(engine), Some("what is here?")).await;

    // The initialize ack is timing-dependent (the stub reads it before replying),
    // so it is filtered out; everything else is asserted exactly.
    let observed: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| !matches!(e, SessionEvent::ControlAck { .. }))
        .collect();

    let expected = [
        SessionEvent::Init {
            session_id: "s1".into(),
            model: "claude-sonnet-5".into(),
            cli_version: "2.1.220".into(),
            tools: vec!["Bash".into(), "Read".into()],
        },
        // Three deltas, one event: the coalescing this issue asks for.
        SessionEvent::TextDelta("Let me look.".into()),
        SessionEvent::AssistantText("Let me look.".into()),
        SessionEvent::ToolUse {
            id: "toolu_01".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        },
        SessionEvent::ToolResult {
            tool_use_id: "toolu_01".into(),
            is_error: false,
            content: serde_json::json!("a.txt"),
        },
        SessionEvent::TurnComplete {
            subtype: "success".into(),
            is_error: false,
            total_cost_usd: Some(0.0123),
            num_turns: Some(2),
        },
        SessionEvent::Closed { exit_code: Some(0) },
    ];

    assert_eq!(
        observed,
        expected.iter().collect::<Vec<_>>(),
        "stream order or coalescing is wrong.\nobserved: {observed:#?}"
    );
}

/// The ordering guarantee, with the timing arranged so a naive implementation
/// would get it wrong: the deltas are still inside their flush window when the
/// tool call arrives, so the pump must flush them *before* forwarding it.
#[tokio::test]
async fn an_event_arriving_mid_window_does_not_overtake_buffered_text() {
    let script = format!(
        r#"
read -r _init
{d1}
{d2}
echo '{{"type":"assistant","session_id":"s1","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Read","input":{{}}}}]}}}}'
{d3}
echo '{{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.001,"num_turns":1,"session_id":"s1"}}'
"#,
        d1 = delta_line("before"),
        d2 = delta_line("-tool"),
        d3 = delta_line("after-tool"),
    );
    let engine = stub_engine("ordering", &script);
    let events = observe(config(engine), None).await;

    let interesting: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::TextDelta(t) => Some(format!("text:{t}")),
            SessionEvent::ToolUse { name, .. } => Some(format!("tool:{name}")),
            _ => None,
        })
        .collect();

    assert_eq!(
        interesting,
        vec![
            "text:before-tool".to_owned(),
            "tool:Read".to_owned(),
            "text:after-tool".to_owned(),
        ],
        "text buffered when an event arrived must precede that event"
    );
}

/// A permission ask the policy does not cover surfaces on the channel; one it
/// covers does not, and reports itself as resolved instead.
#[tokio::test]
async fn permission_asks_surface_or_resolve_according_to_policy() {
    let script = r#"
read -r _init
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"mcp__epik__issue_list","input":{}}}'
echo '{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/tmp/x"}}}'
# Block until the policy's answer to r1 arrives. A real CLI waits for its answer
# too; a stub that exits immediately would close the pipe under the write and the
# resolution would fail rather than being observed. r2 is deliberately left
# unanswered — it is the ask that must surface.
read -r _answer_to_r1
echo '{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.001,"num_turns":1,"session_id":"s1"}'
"#;
    let engine = stub_engine("permissions", script);
    let mut cfg = config(engine.clone());
    cfg.permission_policy =
        PermissionPolicy::from_rules([PermissionRule::allow_prefix("mcp__epik__")]);
    let events = observe(cfg, None).await;

    let resolved: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PermissionResolved { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        })
        .collect();
    let asked: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PermissionRequest { request, .. } => Some(request.tool_name.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(resolved, vec!["mcp__epik__issue_list"]);
    assert_eq!(asked, vec!["Write"]);
}

/// A burst longer than one flush window becomes several events rather than one,
/// so the display updates while text is still arriving instead of at the end.
#[tokio::test]
async fn a_long_stream_flushes_progressively() {
    // Ten deltas, each after a pause longer than the flush window.
    let sleeps: String = (0..10)
        .map(|i| format!("{}\nsleep 0.08\n", delta_line(&format!("chunk{i} "))))
        .collect();
    let script = format!(
        "read -r _init\n{sleeps}\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":1,\"session_id\":\"s1\"}}'\n"
    );
    let engine = stub_engine("progressive", &script);
    let events = observe(config(engine), None).await;

    let deltas: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::TextDelta(t) => Some(t),
            _ => None,
        })
        .collect();

    assert!(
        deltas.len() > 1,
        "a stream spanning many flush windows should produce many events, got {deltas:?}"
    );
    // Nothing is lost or duplicated by the batching.
    let joined: String = deltas.iter().map(|s| s.as_str()).collect();
    let expected: String = (0..10).map(|i| format!("chunk{i} ")).collect();
    assert_eq!(joined, expected, "coalescing changed the text");
    assert!(
        DELTA_FLUSH < Duration::from_millis(80),
        "this test assumes the stub's pauses exceed the flush window"
    );
}
