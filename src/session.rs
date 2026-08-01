//! Interactive session with the `claude` CLI over bidirectional stream-json.
//!
//! `Session::spawn` starts the CLI as a child process, sends the `initialize`
//! control request, and runs a reader task that translates stdout lines into
//! [`SessionEvent`]s on a channel. Writes (user turns, permission decisions,
//! interrupts) go through the cloneable [`SessionHandle`].

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc};

use crate::protocol::{
    CanUseToolRequest, ContentBlock, ControlRequest, ControlResponsePayload, PermissionDecision,
    StreamMessage, control_request, permission_response, user_message,
};

/// How the app answers permission asks it hasn't handled explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPermissionMode {
    /// Surface every ask as a [`SessionEvent::PermissionRequest`].
    Ask,
    /// Answer every ask with allow (spike/demo use).
    AllowAll,
}

#[derive(Debug, Clone)]
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
    pub permission_handling: SessionPermissionMode,
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
            permission_handling: SessionPermissionMode::Ask,
            extra_args: Vec::new(),
        }
    }
}

/// What the app sees. One channel, in stream order.
#[derive(Debug)]
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
    /// The CLI asks whether a tool may run. Answer via
    /// [`SessionHandle::respond_permission`] using `request_id`.
    PermissionRequest {
        request_id: String,
        request: CanUseToolRequest,
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
    /// A line that didn't parse, or an unmodeled message type. Informational.
    Unknown(String),
    /// stdout closed; `status` is the exit status if the child was reaped.
    Closed {
        status: Option<std::process::ExitStatus>,
    },
}

pub struct Session {
    pub events: mpsc::Receiver<SessionEvent>,
    handle: SessionHandle,
}

#[derive(Clone)]
pub struct SessionHandle {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_request_id: Arc<AtomicU64>,
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
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `{}`", config.claude_bin))?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");

        let handle = SessionHandle {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            next_request_id: Arc::new(AtomicU64::new(1)),
        };

        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(reader_task(
            child,
            stdout,
            tx,
            handle.clone(),
            config.permission_handling,
        ));

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

    /// Close stdin; the CLI finishes the current turn and exits.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let mut guard = self.stdin.lock().await;
        if let Some(mut stdin) = guard.take() {
            stdin.shutdown().await?;
        }
        Ok(())
    }
}

async fn reader_task(
    mut child: Child,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::Sender<SessionEvent>,
    handle: SessionHandle,
    permission_handling: SessionPermissionMode,
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
        for event in translate(msg, &handle, permission_handling).await {
            if tx.send(event).await.is_err() {
                return; // app dropped the receiver
            }
        }
    }
    let status = child.wait().await.ok();
    let _ = tx.send(SessionEvent::Closed { status }).await;
}

/// Translate one wire message into zero or more app events. Auto-answers
/// permission asks when the session runs in `AllowAll`.
async fn translate(
    msg: StreamMessage,
    handle: &SessionHandle,
    permission_handling: SessionPermissionMode,
) -> Vec<SessionEvent> {
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
            ControlRequest::CanUseTool(request) => match permission_handling {
                SessionPermissionMode::Ask => vec![SessionEvent::PermissionRequest {
                    request_id: env.request_id,
                    request,
                }],
                SessionPermissionMode::AllowAll => {
                    let decision = PermissionDecision::Allow {
                        updated_input: None,
                    };
                    let answered = handle.respond_permission(&env.request_id, &decision).await;
                    match answered {
                        Ok(()) => vec![SessionEvent::ToolUse {
                            id: format!("auto-allowed:{}", env.request_id),
                            name: format!("[auto-allow] {}", request.tool_name),
                            input: request.input,
                        }],
                        Err(err) => vec![SessionEvent::Unknown(format!(
                            "failed to auto-allow {}: {err}",
                            request.tool_name
                        ))],
                    }
                }
            },
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
