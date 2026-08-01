//! Typed view of the `claude` CLI stream-json protocol.
//!
//! Every line on stdout is one JSON object tagged by `type`. The CLI attaches
//! many fields beyond what we consume; structs here keep only what the app
//! needs and rely on serde's default of ignoring unknown fields. Unknown
//! message types fall through to `Unknown` rather than failing the stream.

// The module models the full wire format; M0 only consumes part of it.
// Remove once M1/M2 read the remaining fields.
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamMessage {
    System(SystemMessage),
    Assistant(AssistantEnvelope),
    User(UserEnvelope),
    Result(ResultMessage),
    StreamEvent(StreamEvent),
    #[serde(untagged)]
    Unknown(Value),
}

/// `type: "system"` — lifecycle notices. `subtype: "init"` opens every session.
#[derive(Debug, Deserialize)]
pub struct SystemMessage {
    pub subtype: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub claude_code_version: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// `type: "assistant"` — wraps a complete API assistant message.
#[derive(Debug, Deserialize)]
pub struct AssistantEnvelope {
    pub message: ApiMessage,
    pub session_id: Option<String>,
}

/// `type: "user"` — echoed user turns (tool results come back this way).
#[derive(Debug, Deserialize)]
pub struct UserEnvelope {
    pub message: ApiMessage,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApiMessage {
    pub role: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(untagged)]
    Unknown(Value),
}

/// `type: "result"` — closes a `-p` run; carries cost and the final text.
#[derive(Debug, Deserialize)]
pub struct ResultMessage {
    pub subtype: String,
    #[serde(default)]
    pub is_error: bool,
    pub result: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
    pub session_id: Option<String>,
}

/// `type: "stream_event"` — raw API streaming deltas (only with
/// `--include-partial-messages`). Kept loose for M0; M1/M2 consume these.
#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    pub event: Value,
    pub session_id: Option<String>,
}

impl StreamMessage {
    /// Parse one stdout line. Returns `None` for blank lines.
    pub fn parse_line(line: &str) -> Option<serde_json::Result<Self>> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        Some(serde_json::from_str(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_system_message() {
        let line = r#"{"type":"system","subtype":"init","cwd":"/x","session_id":"abc","tools":["Bash"],"model":"claude-haiku-4-5-20251001","claude_code_version":"2.1.220","apiKeySource":"ANTHROPIC_API_KEY"}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::System(s) => {
                assert_eq!(s.subtype, "init");
                assert_eq!(s.session_id.as_deref(), Some("abc"));
                assert_eq!(s.tools, vec!["Bash"]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_assistant_text_message() {
        let line = r#"{"type":"assistant","message":{"model":"m","id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":null,"usage":{"input_tokens":3,"output_tokens":4}},"session_id":"abc","uuid":"u"}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::Assistant(a) => match &a.message.content[0] {
                ContentBlock::Text { text } => assert_eq!(text, "hello"),
                other => panic!("wrong block: {other:?}"),
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_result_message() {
        let line = r#"{"is_error":false,"duration_api_ms":1217,"num_turns":1,"stop_reason":"end_turn","session_id":"abc","total_cost_usd":0.03,"subtype":"success","result":"hello","type":"result","duration_ms":1329,"uuid":"u"}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::Result(r) => {
                assert_eq!(r.subtype, "success");
                assert_eq!(r.result.as_deref(), Some("hello"));
                assert!(!r.is_error);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_message_type_does_not_fail() {
        let line = r#"{"type":"some_future_thing","payload":1}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        assert!(matches!(msg, StreamMessage::Unknown(_)));
    }

    #[test]
    fn unknown_content_block_does_not_fail() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"server_tool_use","id":"x"}]},"session_id":"abc"}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::Assistant(a) => {
                assert!(matches!(a.message.content[0], ContentBlock::Unknown(_)));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn blank_line_is_none() {
        assert!(StreamMessage::parse_line("  ").is_none());
    }
}
