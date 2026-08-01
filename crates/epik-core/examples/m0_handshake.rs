//! M0 demo — handshake: spawn `claude -p` one-shot, parse the stream-json output,
//! print the assistant text and run stats.

use std::process::Stdio;

use anyhow::{Context, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use epik_core::protocol::{ContentBlock, StreamMessage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly the word: hello".to_owned());

    let mut child = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "stream-json", "--verbose"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `claude` — is the CLI on PATH?")?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let mut lines = BufReader::new(stdout).lines();

    let mut saw_result = false;
    while let Some(line) = lines.next_line().await? {
        let Some(parsed) = StreamMessage::parse_line(&line) else {
            continue;
        };
        match parsed.context("stream-json line failed to parse")? {
            StreamMessage::System(s) if s.subtype == "init" => {
                eprintln!(
                    "[init] session={} model={} cli={}",
                    s.session_id.as_deref().unwrap_or("?"),
                    s.model.as_deref().unwrap_or("?"),
                    s.claude_code_version.as_deref().unwrap_or("?"),
                );
            }
            StreamMessage::System(_) => {}
            StreamMessage::Assistant(a) => {
                for block in &a.message.content {
                    match block {
                        ContentBlock::Text { text } => println!("{text}"),
                        ContentBlock::ToolUse { name, .. } => {
                            eprintln!("[tool-use] {name}");
                        }
                        _ => {}
                    }
                }
            }
            StreamMessage::User(_) | StreamMessage::StreamEvent(_) => {}
            StreamMessage::Result(r) => {
                saw_result = true;
                eprintln!(
                    "[result] subtype={} turns={} cost=${:.4} duration={}ms",
                    r.subtype,
                    r.num_turns.unwrap_or(0),
                    r.total_cost_usd.unwrap_or(0.0),
                    r.duration_ms.unwrap_or(0),
                );
                if r.is_error {
                    bail!("claude run failed: {:?}", r.result);
                }
            }
            StreamMessage::Unknown(v) => {
                eprintln!("[unknown message] {v}");
            }
            // Control traffic doesn't occur in one-shot stdin-less runs.
            StreamMessage::ControlRequest(_)
            | StreamMessage::ControlResponse(_)
            | StreamMessage::ControlCancelRequest { .. } => {}
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        bail!("claude exited with {status}");
    }
    if !saw_result {
        bail!("stream ended without a result message");
    }
    Ok(())
}
