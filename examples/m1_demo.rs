//! M1 demo: interactive session exercising the control protocol end to end.
//!
//! Scripted scenario (no human input): force `ask` rules for Bash, then
//!   turn 1 — model runs a Bash command; we ALLOW the permission ask;
//!   turn 2 — model runs another; we DENY it and watch the model cope;
//!   turn 3 — long generation; we INTERRUPT mid-stream.
//!
//! Run: cargo run --example m1_demo

use epik_app::protocol::PermissionDecision;
use epik_app::session::{Session, SessionConfig, SessionEvent, SessionPermissionMode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = SessionConfig {
        model: Some("claude-haiku-4-5-20251001".to_owned()),
        permission_mode: Some("default".to_owned()),
        settings_json: Some(r#"{"permissions":{"ask":["Bash"]}}"#.to_owned()),
        tools: Some("Bash".to_owned()),
        permission_handling: SessionPermissionMode::Ask,
        extra_args: vec!["--no-session-persistence".to_owned()],
        ..SessionConfig::default()
    };

    let mut session = Session::spawn(config).await?;
    let handle = session.handle();

    handle
        .send_user("Run exactly `echo m1-allow-ok` with Bash and report its output verbatim.")
        .await?;

    // Which scripted turn we're on: 0=allow, 1=deny, 2=interrupt.
    let mut turn = 0usize;
    let mut streamed = 0usize;
    let mut interrupt_sent = false;

    while let Some(event) = session.next_event().await {
        match event {
            SessionEvent::Init {
                session_id,
                model,
                cli_version,
                ..
            } => println!("[init] session={session_id} model={model} cli={cli_version}"),
            SessionEvent::TextDelta(t) => {
                streamed += t.len();
                // Interrupt turn 3 once the model is visibly mid-generation.
                if turn == 2 && streamed > 40 && !interrupt_sent {
                    interrupt_sent = true;
                    let id = handle.interrupt().await?;
                    println!("\n[interrupt sent as {id}]");
                }
            }
            SessionEvent::AssistantText(text) => println!("assistant: {text}"),
            SessionEvent::PermissionRequest {
                request_id,
                request,
            } => {
                let cmd = request.input.get("command").cloned().unwrap_or_default();
                println!("[permission ask] {} {}", request.tool_name, cmd);
                let decision = if turn == 0 {
                    println!("[answering ALLOW]");
                    PermissionDecision::Allow {
                        updated_input: None,
                    }
                } else {
                    println!("[answering DENY]");
                    PermissionDecision::Deny {
                        message: "Denied by m1_demo: not allowed in this test.".to_owned(),
                    }
                };
                handle.respond_permission(&request_id, &decision).await?;
            }
            SessionEvent::ToolUse { name, .. } => println!("[tool-use] {name}"),
            SessionEvent::ToolResult {
                is_error, content, ..
            } => {
                let text = serde_json::to_string(&content).unwrap_or_default();
                let text = text.chars().take(80).collect::<String>();
                println!("[tool-result] error={is_error} {text}");
            }
            SessionEvent::ControlAck { request_id, result } => match result {
                Ok(_) => println!("[ack] {request_id} ok"),
                Err(e) => println!("[ack] {request_id} ERROR: {e}"),
            },
            SessionEvent::TurnComplete {
                subtype,
                is_error,
                total_cost_usd,
                ..
            } => {
                println!(
                    "[turn complete] subtype={subtype} is_error={is_error} cost=${:.4}",
                    total_cost_usd.unwrap_or(0.0)
                );
                turn += 1;
                streamed = 0;
                match turn {
                    1 => {
                        handle
                            .send_user(
                                "Run exactly `echo m1-deny-attempt` with Bash and report its output.",
                            )
                            .await?;
                    }
                    2 => {
                        handle
                            .send_user(
                                "Count from 1 to 300, one number per line. No tools, no commentary.",
                            )
                            .await?;
                    }
                    _ => {
                        handle.shutdown().await?;
                    }
                }
            }
            SessionEvent::Unknown(info) => println!("[unknown] {info}"),
            SessionEvent::Closed { status } => {
                println!("[closed] status={status:?}");
                break;
            }
        }
    }

    println!("m1_demo finished: 3 turns, interrupt_sent={interrupt_sent}");
    Ok(())
}
