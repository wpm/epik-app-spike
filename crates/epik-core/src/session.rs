//! Interactive session with the `claude` CLI over bidirectional stream-json.
//!
//! `Session::spawn` starts the CLI as a child process, sends the `initialize`
//! control request, and runs a reader task that translates stdout lines into
//! [`SessionEvent`]s on a channel. Writes (user turns, permission decisions,
//! interrupts, policy edits) go through the cloneable [`SessionHandle`].
//!
//! Every [`SessionEvent`] is `Serialize + Deserialize`, because the host is not
//! necessarily in this process: the Tauri app forwards these values across IPC
//! and the frontend deserializes *these same types*, so there is one definition
//! of an event rather than a Rust one and a mirrored frontend one that drift.
//! That is also why `Closed` carries an `exit_code: Option<i32>` rather than a
//! `std::process::ExitStatus` — `ExitStatus` is not serializable, and an exit
//! code is the only part of it a UI has anything to say about.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, mpsc};

use crate::permission::{PermissionAction, PermissionPolicy, PermissionRule};
use crate::protocol::{
    CanUseToolRequest, ContentBlock, ControlRequest, ControlResponsePayload, PermissionDecision,
    StreamMessage, control_request, permission_response, user_message,
};

/// How many stderr lines a session keeps for post-mortem queries. The child's
/// stderr is diagnostics, not a log to archive: enough to explain a startup
/// failure or a crash, bounded so a chatty engine cannot grow the host's memory
/// without limit.
pub const STDERR_BUFFER_LINES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Path or name of the CLI binary.
    pub claude_bin: String,
    pub model: Option<String>,
    pub cwd: Option<PathBuf>,
    /// CLI-side permission mode (`default`, `acceptEdits`, `plan`, ...).
    pub permission_mode: Option<String>,
    /// Inline `--settings` JSON (e.g. ask rules).
    pub settings_json: Option<String>,
    /// Inline or path `--mcp-config`.
    pub mcp_config: Option<String>,
    /// `--tools` value (e.g. "Bash,Read"); None = CLI default set.
    pub tools: Option<String>,
    pub append_system_prompt: Option<String>,
    /// Emit `stream_event` deltas (`--include-partial-messages`).
    pub include_partial: bool,
    /// Which asks are answered without a human. Empty = ask about everything.
    pub permission_policy: PermissionPolicy,
    pub extra_args: Vec<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            claude_bin: "claude".to_owned(),
            model: None,
            cwd: None,
            permission_mode: None,
            settings_json: None,
            mcp_config: None,
            tools: None,
            append_system_prompt: None,
            include_partial: true,
            permission_policy: PermissionPolicy::ask(),
            extra_args: Vec::new(),
        }
    }
}

/// What the host sees. One channel, in stream order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SessionEvent {
    /// `system/init` — session is live.
    Init {
        session_id: String,
        model: String,
        cli_version: String,
        tools: Vec<String>,
    },
    /// A complete assistant text block (arrives at end of each block).
    AssistantText(String),
    /// Incremental text delta (only with `include_partial`).
    TextDelta(String),
    /// The assistant invoked a tool.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// A tool finished; `content` is the raw result payload.
    ToolResult {
        tool_use_id: String,
        is_error: bool,
        content: Value,
    },
    /// The CLI asks whether a tool may run, and the session's policy had no
    /// answer. Answer via [`SessionHandle::respond_permission`] using
    /// `request_id`; the turn does not proceed until you do.
    PermissionRequest {
        request_id: String,
        request: CanUseToolRequest,
    },
    /// An ask the policy answered on the host's behalf. Informational — it has
    /// already been answered and no response is expected. Without this, a
    /// pre-allowed tool would run with no trace of why it was never asked
    /// about, which is exactly the question someone asks when a permission
    /// prompt they expected does not appear.
    PermissionResolved {
        request_id: String,
        tool_name: String,
        action: PermissionAction,
    },
    /// Ack/response to one of our control requests (initialize, interrupt).
    ControlAck {
        request_id: String,
        result: Result<Value, String>,
    },
    /// End of a turn.
    TurnComplete {
        subtype: String,
        is_error: bool,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
    },
    /// One line the child wrote to stderr. Also retained on the session; see
    /// [`SessionHandle::recent_stderr`].
    Stderr(String),
    /// A line that didn't parse, or an unmodeled message type. Informational.
    Unknown(String),
    /// stdout closed and the child was reaped. `exit_code` is `None` when the
    /// child was killed by a signal or could not be waited on.
    Closed { exit_code: Option<i32> },
}

pub struct Session {
    pub events: mpsc::Receiver<SessionEvent>,
    handle: SessionHandle,
}

#[derive(Clone)]
pub struct SessionHandle {
    stdin: Arc<AsyncMutex<Option<ChildStdin>>>,
    next_request_id: Arc<AtomicU64>,
    /// Shared with the reader task, which consults it on every ask. Held under
    /// a std mutex rather than an async one: every critical section is a list
    /// walk with no await in it, so a guard never crosses a yield point.
    policy: Arc<Mutex<PermissionPolicy>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl Session {
    pub async fn spawn(config: SessionConfig) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.claude_bin);
        cmd.args([
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--permission-prompt-tool",
            "stdio",
        ]);
        if config.include_partial {
            cmd.arg("--include-partial-messages");
        }
        if let Some(model) = &config.model {
            cmd.args(["--model", model]);
        }
        if let Some(mode) = &config.permission_mode {
            cmd.args(["--permission-mode", mode]);
        }
        if let Some(settings) = &config.settings_json {
            cmd.args(["--settings", settings]);
        }
        if let Some(mcp) = &config.mcp_config {
            cmd.args(["--mcp-config", mcp]);
        }
        if let Some(tools) = &config.tools {
            cmd.args(["--tools", tools]);
        }
        if let Some(sys) = &config.append_system_prompt {
            cmd.args(["--append-system-prompt", sys]);
        }
        cmd.args(&config.extra_args);
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Piped, not null: when the engine fails to start, the reason is
            // here and nowhere else, and a GUI has no terminal to leak it to.
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `{}`", config.claude_bin))?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let handle = SessionHandle {
            stdin: Arc::new(AsyncMutex::new(Some(stdin))),
            next_request_id: Arc::new(AtomicU64::new(1)),
            policy: Arc::new(Mutex::new(config.permission_policy)),
            stderr: Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_BUFFER_LINES))),
        };

        let (tx, rx) = mpsc::channel(256);
        let stderr_pump = tokio::spawn(stderr_task(stderr, tx.clone(), handle.clone()));
        tokio::spawn(reader_task(child, stdout, tx, handle.clone(), stderr_pump));

        // Open the control channel; the ack arrives as a ControlAck event.
        handle
            .send_control(serde_json::json!({"subtype": "initialize"}))
            .await?;

        Ok(Self { events: rx, handle })
    }

    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }

    /// Receive the next event; `None` after `Closed`.
    pub async fn next_event(&mut self) -> Option<SessionEvent> {
        self.events.recv().await
    }

    /// The child's most recent stderr lines, oldest first.
    pub fn recent_stderr(&self) -> Vec<String> {
        self.handle.recent_stderr()
    }
}

impl SessionHandle {
    async fn send_line(&self, value: &Value) -> anyhow::Result<()> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().context("session stdin already closed")?;
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        stdin.write_all(&line).await?;
        stdin.flush().await?;
        Ok(())
    }

    fn fresh_request_id(&self) -> String {
        let n = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        format!("epik-req-{n}")
    }

    /// Send a user turn.
    pub async fn send_user(&self, text: &str) -> anyhow::Result<()> {
        self.send_line(&user_message(text)).await
    }

    /// Send a control request; returns its request_id (ack arrives as event).
    pub async fn send_control(&self, request: Value) -> anyhow::Result<String> {
        let id = self.fresh_request_id();
        self.send_line(&control_request(&id, request)).await?;
        Ok(id)
    }

    /// Interrupt the in-flight turn.
    pub async fn interrupt(&self) -> anyhow::Result<String> {
        self.send_control(serde_json::json!({"subtype": "interrupt"}))
            .await
    }

    /// Answer a `can_use_tool` ask.
    pub async fn respond_permission(
        &self,
        request_id: &str,
        decision: &PermissionDecision,
    ) -> anyhow::Result<()> {
        self.send_line(&permission_response(request_id, decision))
            .await
    }

    /// Add a rule that wins over everything already in the policy. This is the
    /// mechanism behind "always allow this tool for this session": the rule
    /// takes effect for every ask that arrives after it lands.
    pub fn add_rule(&self, rule: PermissionRule) {
        self.lock_policy().prepend(rule);
    }

    /// Convenience for the common case: always allow one tool by exact name.
    pub fn add_allow_rule(&self, tool_name: impl Into<String>) {
        self.add_rule(PermissionRule::allow(tool_name));
    }

    /// A snapshot of the session's current policy.
    pub fn policy(&self) -> PermissionPolicy {
        self.lock_policy().clone()
    }

    pub fn replace_policy(&self, policy: PermissionPolicy) {
        *self.lock_policy() = policy;
    }

    /// The child's most recent stderr lines, oldest first.
    pub fn recent_stderr(&self) -> Vec<String> {
        self.lock_stderr().iter().cloned().collect()
    }

    /// Close stdin; the CLI finishes the current turn and exits.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let mut guard = self.stdin.lock().await;
        if let Some(mut stdin) = guard.take() {
            stdin.shutdown().await?;
        }
        Ok(())
    }

    /// A poisoned mutex means another thread panicked mid-edit. Both values
    /// behind these locks are plain owned collections — a panic cannot leave
    /// them in a state that is unsafe to read — so recovering beats
    /// propagating the panic into a host that would then have no way to answer
    /// an ask or report why the engine died.
    fn lock_policy(&self) -> std::sync::MutexGuard<'_, PermissionPolicy> {
        self.policy.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_stderr(&self) -> std::sync::MutexGuard<'_, VecDeque<String>> {
        self.stderr.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn decide(&self, tool_name: &str) -> (PermissionAction, String) {
        let policy = self.lock_policy();
        (policy.decide(tool_name), policy.deny_message(tool_name))
    }

    fn record_stderr(&self, line: String) {
        let mut buffer = self.lock_stderr();
        if buffer.len() == STDERR_BUFFER_LINES {
            buffer.pop_front();
        }
        buffer.push_back(line);
    }
}

/// Forward the child's stderr, line by line, and keep a bounded copy.
async fn stderr_task(
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<SessionEvent>,
    handle: SessionHandle,
) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        handle.record_stderr(line.clone());
        if tx.send(SessionEvent::Stderr(line)).await.is_err() {
            return; // host dropped the receiver
        }
    }
}

async fn reader_task(
    mut child: Child,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::Sender<SessionEvent>,
    handle: SessionHandle,
    stderr_pump: tokio::task::JoinHandle<()>,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                let _ = tx
                    .send(SessionEvent::Unknown(format!("read error: {err}")))
                    .await;
                break;
            }
        };
        let Some(parsed) = StreamMessage::parse_line(&line) else {
            continue;
        };
        let msg = match parsed {
            Ok(msg) => msg,
            Err(err) => {
                let _ = tx
                    .send(SessionEvent::Unknown(format!("parse error: {err}: {line}")))
                    .await;
                continue;
            }
        };
        for event in translate(msg, &handle).await {
            if tx.send(event).await.is_err() {
                return; // host dropped the receiver
            }
        }
    }
    let exit_code = child.wait().await.ok().and_then(|status| status.code());
    // Drain stderr before announcing the close, so a host that reacts to
    // `Closed` by reading `recent_stderr()` sees the child's last words. The
    // child has exited, so its stderr is closed and this cannot hang.
    let _ = stderr_pump.await;
    let _ = tx.send(SessionEvent::Closed { exit_code }).await;
}

/// Translate one wire message into zero or more host events. Permission asks
/// the session's policy can answer are answered here and never surface as
/// [`SessionEvent::PermissionRequest`].
async fn translate(msg: StreamMessage, handle: &SessionHandle) -> Vec<SessionEvent> {
    match msg {
        StreamMessage::System(s) if s.subtype == "init" => vec![SessionEvent::Init {
            session_id: s.session_id.unwrap_or_default(),
            model: s.model.unwrap_or_default(),
            cli_version: s.claude_code_version.unwrap_or_default(),
            tools: s.tools,
        }],
        StreamMessage::System(_) => vec![],
        StreamMessage::Assistant(a) => a
            .message
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(SessionEvent::AssistantText(text)),
                ContentBlock::ToolUse { id, name, input } => {
                    Some(SessionEvent::ToolUse { id, name, input })
                }
                _ => None,
            })
            .collect(),
        StreamMessage::User(u) => u
            .message
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => Some(SessionEvent::ToolResult {
                    tool_use_id,
                    is_error,
                    content,
                }),
                _ => None,
            })
            .collect(),
        StreamMessage::Result(r) => vec![SessionEvent::TurnComplete {
            subtype: r.subtype,
            is_error: r.is_error,
            total_cost_usd: r.total_cost_usd,
            num_turns: r.num_turns,
        }],
        StreamMessage::StreamEvent(ev) => {
            // content_block_delta / text_delta → incremental text.
            let delta = ev
                .event
                .get("delta")
                .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
                .and_then(|d| d.get("text"))
                .and_then(Value::as_str);
            match delta {
                Some(text) => vec![SessionEvent::TextDelta(text.to_owned())],
                None => vec![],
            }
        }
        StreamMessage::ControlRequest(env) => match env.request {
            ControlRequest::CanUseTool(request) => {
                resolve_permission(env.request_id, request, handle).await
            }
            ControlRequest::Other(v) => vec![SessionEvent::Unknown(format!(
                "unhandled control_request: {v}"
            ))],
        },
        StreamMessage::ControlResponse(env) => match env.response {
            ControlResponsePayload::Success {
                request_id,
                response,
            } => vec![SessionEvent::ControlAck {
                request_id,
                result: Ok(response),
            }],
            ControlResponsePayload::Error { request_id, error } => {
                vec![SessionEvent::ControlAck {
                    request_id,
                    result: Err(error),
                }]
            }
        },
        StreamMessage::ControlCancelRequest { request_id } => vec![SessionEvent::Unknown(format!(
            "control_cancel_request for {request_id}"
        ))],
        StreamMessage::Unknown(v) => vec![SessionEvent::Unknown(v.to_string())],
    }
}

async fn resolve_permission(
    request_id: String,
    request: CanUseToolRequest,
    handle: &SessionHandle,
) -> Vec<SessionEvent> {
    let (action, deny_message) = handle.decide(&request.tool_name);
    let decision = match action {
        PermissionAction::Ask => {
            return vec![SessionEvent::PermissionRequest {
                request_id,
                request,
            }];
        }
        PermissionAction::Allow => PermissionDecision::Allow {
            updated_input: None,
        },
        PermissionAction::Deny => PermissionDecision::Deny {
            message: deny_message,
        },
    };
    match handle.respond_permission(&request_id, &decision).await {
        Ok(()) => vec![SessionEvent::PermissionResolved {
            request_id,
            tool_name: request.tool_name,
            action,
        }],
        // Failing to answer would hang the turn silently; say so instead.
        Err(err) => vec![SessionEvent::Unknown(format!(
            "failed to answer {action:?} for {}: {err}",
            request.tool_name
        ))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, so the round-trip test cannot silently stop covering one.
    fn every_variant() -> Vec<SessionEvent> {
        vec![
            SessionEvent::Init {
                session_id: "abc".into(),
                model: "claude-sonnet-5".into(),
                cli_version: "2.1.220".into(),
                tools: vec!["Bash".into(), "Read".into()],
            },
            SessionEvent::AssistantText("hello ünïcode 🎉".into()),
            SessionEvent::TextDelta("hel".into()),
            SessionEvent::ToolUse {
                id: "toolu_01".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "ls -l", "nested": {"n": 1}}),
            },
            SessionEvent::ToolResult {
                tool_use_id: "toolu_01".into(),
                is_error: true,
                content: serde_json::json!([{"type": "text", "text": "boom"}]),
            },
            SessionEvent::PermissionRequest {
                request_id: "dc74428c".into(),
                request: CanUseToolRequest {
                    tool_name: "Write".into(),
                    input: serde_json::json!({"file_path": "/tmp/x"}),
                    tool_use_id: Some("toolu_02".into()),
                    display_name: Some("Write".into()),
                    description: Some("Create x".into()),
                    decision_reason: None,
                    decision_reason_type: Some("rule".into()),
                    blocked_path: Some("/tmp/x".into()),
                },
            },
            SessionEvent::PermissionResolved {
                request_id: "dc74428d".into(),
                tool_name: "mcp__epik__issue_list".into(),
                action: PermissionAction::Allow,
            },
            SessionEvent::ControlAck {
                request_id: "epik-req-1".into(),
                result: Ok(serde_json::json!({"commands": []})),
            },
            SessionEvent::ControlAck {
                request_id: "epik-req-2".into(),
                result: Err("boom".into()),
            },
            SessionEvent::TurnComplete {
                subtype: "success".into(),
                is_error: false,
                total_cost_usd: Some(0.0321),
                num_turns: Some(3),
            },
            SessionEvent::Stderr("node: warning".into()),
            SessionEvent::Unknown("{\"type\":\"future\"}".into()),
            SessionEvent::Closed { exit_code: Some(0) },
            SessionEvent::Closed { exit_code: None },
        ]
    }

    #[test]
    fn every_event_variant_round_trips_through_json() {
        for event in every_variant() {
            let json = serde_json::to_string(&event)
                .unwrap_or_else(|e| panic!("serialize {event:?}: {e}"));
            let back: SessionEvent =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize {json}: {e}"));
            assert_eq!(back, event, "round-trip changed the value: {json}");
        }
    }

    #[test]
    fn every_event_variant_is_covered_by_the_round_trip() {
        // Guards the test above: a variant added without a sample here would
        // otherwise go untested and unnoticed.
        let covered: std::collections::BTreeSet<String> = every_variant()
            .iter()
            .map(|e| match serde_json::to_value(e).unwrap() {
                Value::Object(map) => map["type"].as_str().unwrap().to_owned(),
                other => panic!("expected a tagged object, got {other}"),
            })
            .collect();
        let expected: std::collections::BTreeSet<String> = [
            "init",
            "assistant_text",
            "text_delta",
            "tool_use",
            "tool_result",
            "permission_request",
            "permission_resolved",
            "control_ack",
            "turn_complete",
            "stderr",
            "unknown",
            "closed",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(covered, expected);
    }

    #[test]
    fn closed_carries_an_exit_code_not_an_exit_status() {
        let json = serde_json::to_string(&SessionEvent::Closed { exit_code: Some(2) }).unwrap();
        assert_eq!(json, r#"{"type":"closed","payload":{"exit_code":2}}"#);
    }

    #[test]
    fn config_round_trips_with_its_policy() {
        let config = SessionConfig {
            model: Some("claude-sonnet-5".into()),
            permission_policy: PermissionPolicy::from_rules([PermissionRule::allow_prefix(
                "mcp__epik__",
            )]),
            ..SessionConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: SessionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, config.model);
        assert_eq!(back.permission_policy, config.permission_policy);
    }

    // ---------------------------------------------------------------------
    // Live-child tests. These run a stub "engine" — a shell script standing in
    // for `claude` — so they exercise the real spawn / read / answer / reap
    // path without needing the CLI, an API key, or a network.
    // ---------------------------------------------------------------------

    #[cfg(unix)]
    mod stub {
        use super::*;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        /// Write `body` as an executable script and return its path. Named per
        /// test so tests running concurrently cannot collide.
        pub fn engine(name: &str, body: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!("epik-stub-{name}"));
            let mut file = std::fs::File::create(&path).expect("create stub engine");
            write!(file, "#!/bin/sh\n{body}").expect("write stub engine");
            file.flush().expect("flush stub engine");
            drop(file);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub engine");
            path
        }

        pub fn config(path: PathBuf) -> SessionConfig {
            SessionConfig {
                claude_bin: path.to_string_lossy().into_owned(),
                ..SessionConfig::default()
            }
        }

        const LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

        /// Collect events until the session closes, with a timeout so a broken
        /// stub fails the test instead of hanging it.
        pub async fn drain(session: &mut Session) -> Vec<SessionEvent> {
            let mut events = Vec::new();
            let collect = async {
                while let Some(event) = session.next_event().await {
                    let closed = matches!(event, SessionEvent::Closed { .. });
                    events.push(event);
                    if closed {
                        break;
                    }
                }
            };
            tokio::time::timeout(LIMIT, collect)
                .await
                .expect("stub engine session did not close in time");
            events
        }

        /// Read events until `f` returns `Some`, with the same timeout.
        pub async fn until<T>(
            session: &mut Session,
            what: &str,
            mut f: impl FnMut(SessionEvent) -> Option<T>,
        ) -> T {
            let search = async {
                while let Some(event) = session.next_event().await {
                    if let Some(found) = f(event) {
                        return Some(found);
                    }
                }
                None
            };
            tokio::time::timeout(LIMIT, search)
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .unwrap_or_else(|| panic!("stream ended before {what}"))
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stderr_lines_surface_as_events_and_are_retained() {
        let engine = stub::engine(
            "stderr",
            // Two stderr lines, no stdout at all, then a nonzero exit.
            "echo 'engine: could not find config' >&2\n\
             echo 'engine: giving up' >&2\n\
             exit 3\n",
        );
        let mut session = Session::spawn(stub::config(engine))
            .await
            .expect("spawn stub engine");
        let events = stub::drain(&mut session).await;

        let stderr_lines: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Stderr(line) => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            stderr_lines,
            vec!["engine: could not find config", "engine: giving up"],
            "stderr content did not reach the event stream: {events:?}"
        );

        // Retained for post-mortem, and readable once the session is closed —
        // which is the only time anyone wants it.
        assert_eq!(
            session.recent_stderr(),
            vec![
                "engine: could not find config".to_owned(),
                "engine: giving up".to_owned(),
            ]
        );
        assert_eq!(
            events.last(),
            Some(&SessionEvent::Closed { exit_code: Some(3) }),
            "expected the stub's exit code on Closed: {events:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stderr_buffer_is_bounded_and_keeps_the_most_recent_lines() {
        let overflow = STDERR_BUFFER_LINES + 50;
        let engine = stub::engine(
            "stderr-bounded",
            &format!(
                "i=1\nwhile [ $i -le {overflow} ]; do echo \"line $i\" >&2; i=$((i+1)); done\n"
            ),
        );
        let mut session = Session::spawn(stub::config(engine))
            .await
            .expect("spawn stub engine");
        stub::drain(&mut session).await;

        let retained = session.recent_stderr();
        assert_eq!(retained.len(), STDERR_BUFFER_LINES, "buffer is not bounded");
        // Oldest first, and it is the *tail* of the output that survives.
        assert_eq!(
            retained.first().unwrap(),
            &format!("line {}", overflow - STDERR_BUFFER_LINES + 1)
        );
        assert_eq!(retained.last().unwrap(), &format!("line {overflow}"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn policy_answers_asks_before_they_surface() {
        let engine = stub::engine(
            "policy",
            r#"
cat <<'EOF'
{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"mcp__epik__issue_list","input":{}}}
{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf /"}}}
{"type":"control_request","request_id":"r3","request":{"subtype":"can_use_tool","tool_name":"Write","input":{}}}
EOF
# Hold stdout open so the session stays live while the test reads.
cat > /dev/null
"#,
        );
        let mut config = stub::config(engine);
        config.permission_policy = PermissionPolicy::from_rules([
            PermissionRule::allow_prefix("mcp__epik__"),
            PermissionRule::deny("Bash", "No shell in this session."),
        ]);
        let mut session = Session::spawn(config).await.expect("spawn stub engine");

        let mut resolved = Vec::new();
        let asked = stub::until(&mut session, "the uncovered ask", |event| match event {
            SessionEvent::PermissionResolved {
                tool_name, action, ..
            } => {
                resolved.push((tool_name, action));
                None
            }
            SessionEvent::PermissionRequest { request, .. } => Some(request.tool_name),
            _ => None,
        })
        .await;

        assert_eq!(
            resolved,
            vec![
                ("mcp__epik__issue_list".to_owned(), PermissionAction::Allow),
                ("Bash".to_owned(), PermissionAction::Deny),
            ],
            "policy did not answer the asks it covers"
        );
        assert_eq!(
            asked, "Write",
            "the uncovered ask should be the only one to surface"
        );

        session.handle().shutdown().await.ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rule_added_at_runtime_covers_a_later_ask() {
        let engine = stub::engine(
            "runtime-rule",
            r#"
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}'
# Wait for the initialize control request and the first answer before asking
# again, so the ordering this test depends on is the stub's, not a race.
head -n 2 > /dev/null
echo '{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}'
sleep 5
"#,
        );
        let mut session = Session::spawn(stub::config(engine))
            .await
            .expect("spawn stub engine");
        let handle = session.handle();

        // The first ask surfaces: the policy is empty.
        let request_id = stub::until(&mut session, "the first ask", |event| match event {
            SessionEvent::PermissionRequest {
                request_id,
                request,
            } => {
                assert_eq!(request.tool_name, "Bash");
                Some(request_id)
            }
            _ => None,
        })
        .await;

        // "Always allow this tool for this session", then answer this one.
        handle.add_allow_rule("Bash");
        assert_eq!(handle.policy().decide("Bash"), PermissionAction::Allow);
        handle
            .respond_permission(
                &request_id,
                &PermissionDecision::Allow {
                    updated_input: None,
                },
            )
            .await
            .expect("answer the first ask");

        // The second ask is resolved by the new rule and never surfaces.
        let action = stub::until(&mut session, "the second ask", |event| match event {
            SessionEvent::PermissionResolved { action, .. } => Some(Some(action)),
            SessionEvent::PermissionRequest { .. } => Some(None),
            _ => None,
        })
        .await;
        assert_eq!(
            action,
            Some(PermissionAction::Allow),
            "the runtime rule did not cover the second ask"
        );

        handle.shutdown().await.ok();
    }

    /// The two modes `SessionPermissionMode` used to name, as policies, run
    /// against the same stub so the equivalence is behavioural rather than an
    /// assertion about the data structure.
    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_ask_mode_is_the_empty_policy() {
        const SCRIPT: &str = r#"
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}'
sleep 5
"#;
        let mut config = stub::config(stub::engine("legacy-ask", SCRIPT));
        config.permission_policy = PermissionPolicy::ask();
        let mut session = Session::spawn(config).await.expect("spawn");

        let surfaced = stub::until(&mut session, "a verdict on Bash", |event| match event {
            SessionEvent::PermissionRequest { .. } => Some(true),
            SessionEvent::PermissionResolved { .. } => Some(false),
            _ => None,
        })
        .await;
        assert!(surfaced, "the empty policy must ask");
        session.handle().shutdown().await.ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_allow_all_mode_is_one_wildcard_rule() {
        const SCRIPT: &str = r#"
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}'
sleep 5
"#;
        let mut config = stub::config(stub::engine("legacy-allow-all", SCRIPT));
        config.permission_policy = PermissionPolicy::allow_all();
        let mut session = Session::spawn(config).await.expect("spawn");

        let action = stub::until(&mut session, "a verdict on Bash", |event| match event {
            SessionEvent::PermissionResolved { action, .. } => Some(Some(action)),
            SessionEvent::PermissionRequest { .. } => Some(None),
            _ => None,
        })
        .await;
        assert_eq!(
            action,
            Some(PermissionAction::Allow),
            "the wildcard policy must allow without asking"
        );
        session.handle().shutdown().await.ok();
    }
}
