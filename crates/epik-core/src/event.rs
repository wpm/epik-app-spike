//! The types that cross the host's IPC boundary.
//!
//! Split out from [`crate::session`] so a frontend can depend on them without
//! depending on tokio. `epik-ui` compiles to wasm32, where tokio's process and
//! network features cannot build at all — but it still has to deserialize
//! exactly the values the host serializes. Putting the vocabulary in its own
//! module, behind no feature flag, is what makes "one set of Rust types shared
//! across the IPC boundary" true rather than aspirational: the alternative is a
//! mirrored set of frontend structs that drift silently the first time an event
//! gains a field.
//!
//! Nothing here runs a process. The machinery that does is in
//! [`crate::session`], behind the `host` feature.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permission::{PermissionAction, PermissionPolicy};
use crate::protocol::CanUseToolRequest;

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
///
/// Adjacently tagged (`{"type": ..., "payload": ...}`) because that is the one
/// serde representation that handles every variant shape here — internal tagging
/// cannot encode a newtype variant wrapping a string, and `Stderr(String)` is
/// one.
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
    /// answer. Answer via `SessionHandle::respond_permission` using
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
    /// `SessionHandle::recent_stderr`.
    Stderr(String),
    /// A line that didn't parse, or an unmodeled message type. Informational.
    Unknown(String),
    /// stdout closed and the child was reaped. `exit_code` is `None` when the
    /// child was killed by a signal or could not be waited on.
    Closed { exit_code: Option<i32> },
}

impl SessionEvent {
    /// Whether this event ends a turn — the frontend's cue to re-enable input.
    pub fn ends_turn(&self) -> bool {
        matches!(self, Self::TurnComplete { .. } | Self::Closed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionRule;

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

    #[test]
    fn only_turn_ending_events_end_a_turn() {
        assert!(
            SessionEvent::TurnComplete {
                subtype: "success".into(),
                is_error: false,
                total_cost_usd: None,
                num_turns: None,
            }
            .ends_turn()
        );
        assert!(SessionEvent::Closed { exit_code: None }.ends_turn());
        assert!(!SessionEvent::TextDelta("x".into()).ends_turn());
        assert!(!SessionEvent::AssistantText("x".into()).ends_turn());
    }
}
