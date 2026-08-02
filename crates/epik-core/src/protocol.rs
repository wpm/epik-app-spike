//! Typed view of the `claude` CLI stream-json protocol.
//!
//! Every line on stdout is one JSON object tagged by `type`. The CLI attaches
//! many fields beyond what we consume; structs here keep only what the app
//! needs and rely on serde's default of ignoring unknown fields. Unknown
//! message types fall through to `Unknown` rather than failing the stream.

// The module models the full wire format; the app only consumes part of it.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamMessage {
    System(SystemMessage),
    Assistant(AssistantEnvelope),
    User(UserEnvelope),
    Result(ResultMessage),
    StreamEvent(StreamEvent),
    ControlRequest(ControlRequestEnvelope),
    ControlResponse(ControlResponseEnvelope),
    ControlCancelRequest {
        request_id: String,
    },
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

/// `type: "control_request"` — CLI → client. The one that matters is
/// `can_use_tool`; everything else degrades to `Other`.
#[derive(Debug, Deserialize)]
pub struct ControlRequestEnvelope {
    pub request_id: String,
    pub request: ControlRequest,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum ControlRequest {
    CanUseTool(CanUseToolRequest),
    #[serde(untagged)]
    Other(Value),
}

/// Permission ask for one tool call. The CLI sends more advisory fields than
/// listed here (suggestions, rule matches); we keep what a UI needs to render
/// a prompt and answer it.
///
/// `Serialize` as well as `Deserialize`, because this is the one wire type that
/// rides inside a [`crate::SessionEvent`] and therefore has to cross the host's
/// IPC boundary intact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanUseToolRequest {
    pub tool_name: String,
    #[serde(default)]
    pub input: Value,
    pub tool_use_id: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub decision_reason: Option<String>,
    pub decision_reason_type: Option<String>,
    pub blocked_path: Option<String>,
}

/// `type: "control_response"` — CLI → client, answering our control requests
/// (initialize, interrupt, ...).
#[derive(Debug, Deserialize)]
pub struct ControlResponseEnvelope {
    pub response: ControlResponsePayload,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum ControlResponsePayload {
    Success {
        request_id: String,
        #[serde(default)]
        response: Value,
    },
    Error {
        request_id: String,
        error: String,
    },
}

// ---------------------------------------------------------------------------
// Outbound: client → CLI stdin. Serialized one JSON object per line.
// ---------------------------------------------------------------------------

/// A user turn. `content` uses API content-block form.
pub fn user_message(text: &str) -> Value {
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": text}],
        },
    })
}

/// Client → CLI control request (`initialize`, `interrupt`, ...).
pub fn control_request(request_id: &str, request: Value) -> Value {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
}

/// Answer to a `can_use_tool` ask.
///
/// `Deserialize` as well as `Serialize`: the decision originates in the UI, so it
/// crosses the host's IPC boundary inbound before being serialized outbound to
/// the CLI. The shape on both wires is the same one, which is the point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "camelCase")]
pub enum PermissionDecision {
    #[serde(rename = "allow")]
    Allow {
        #[serde(
            rename = "updatedInput",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        updated_input: Option<Value>,
    },
    #[serde(rename = "deny")]
    Deny { message: String },
}

pub fn permission_response(request_id: &str, decision: &PermissionDecision) -> Value {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": decision,
        },
    })
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

    #[test]
    fn parses_can_use_tool_control_request() {
        // Captured from CLI 2.1.220 with --permission-prompt-tool stdio.
        let line = r#"{"type":"control_request","request_id":"dc74428c-e1d2","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"printf ok > f.txt","description":"Create f.txt"},"description":"Create f.txt","decision_reason_type":"rule","tool_use_id":"toolu_01","blocked_path":"/tmp/x"}}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::ControlRequest(env) => {
                assert_eq!(env.request_id, "dc74428c-e1d2");
                match env.request {
                    ControlRequest::CanUseTool(req) => {
                        assert_eq!(req.tool_name, "Bash");
                        assert_eq!(req.input["command"], "printf ok > f.txt");
                        assert_eq!(req.tool_use_id.as_deref(), Some("toolu_01"));
                        assert_eq!(req.decision_reason_type.as_deref(), Some("rule"));
                    }
                    other => panic!("wrong request: {other:?}"),
                }
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_control_request_subtype_degrades() {
        let line = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"request_user_dialog","dialog_kind":"x"}}"#;
        let msg = StreamMessage::parse_line(line).unwrap().unwrap();
        match msg {
            StreamMessage::ControlRequest(env) => {
                assert!(matches!(env.request, ControlRequest::Other(_)));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_control_response_success_and_error() {
        let ok = r#"{"type":"control_response","response":{"subtype":"success","request_id":"init1","response":{"commands":[]}}}"#;
        match StreamMessage::parse_line(ok).unwrap().unwrap() {
            StreamMessage::ControlResponse(env) => match env.response {
                ControlResponsePayload::Success { request_id, .. } => {
                    assert_eq!(request_id, "init1");
                }
                other => panic!("wrong payload: {other:?}"),
            },
            other => panic!("wrong variant: {other:?}"),
        }
        let err = r#"{"type":"control_response","response":{"subtype":"error","request_id":"r2","error":"boom"}}"#;
        match StreamMessage::parse_line(err).unwrap().unwrap() {
            StreamMessage::ControlResponse(env) => match env.response {
                ControlResponsePayload::Error { error, .. } => assert_eq!(error, "boom"),
                other => panic!("wrong payload: {other:?}"),
            },
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn serializes_permission_decisions() {
        let allow = permission_response(
            "r1",
            &PermissionDecision::Allow {
                updated_input: Some(serde_json::json!({"command": "ls"})),
            },
        );
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["subtype"], "success");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "ls"
        );

        let deny = permission_response(
            "r2",
            &PermissionDecision::Deny {
                message: "no".into(),
            },
        );
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert_eq!(deny["response"]["response"]["message"], "no");
        assert!(deny["response"]["response"].get("updatedInput").is_none());
    }

    #[test]
    fn serializes_user_message_and_control_request() {
        let u = user_message("hi");
        assert_eq!(u["type"], "user");
        assert_eq!(u["message"]["content"][0]["text"], "hi");
        let c = control_request("i1", serde_json::json!({"subtype": "interrupt"}));
        assert_eq!(c["request"]["subtype"], "interrupt");
        assert_eq!(c["request_id"], "i1");
    }
}
