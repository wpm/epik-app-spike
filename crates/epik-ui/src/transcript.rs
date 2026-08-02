//! The frontend's state, as a pure function of the event stream.
//!
//! Everything the window shows is derived here from `epik_core::SessionEvent`s by
//! [`Transcript::apply`]. Deliberately free of Leptos and of IPC: a reducer with
//! no I/O in it can be tested by feeding it events and reading the result, which
//! is how the properties #8, #9 and #10 ask about — delta-then-final replacement,
//! tool results pairing with their calls, cost accumulating across turns — are
//! checked without a browser.
//!
//! These are view models, not event types. Nothing here re-declares anything
//! `epik-core` defines; `Entry::Permission` holds a real `CanUseToolRequest`, and
//! the tool card holds the real `serde_json::Value` input.

use epik_core::{CanUseToolRequest, PermissionAction, SessionEvent};
use serde_json::Value;

/// What the session is doing. Drives the status bar and whether input is live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// No session has been started.
    #[default]
    Offline,
    /// Spawn requested; the engine has not sent `init` yet.
    Starting,
    /// Live, no turn in flight.
    Idle,
    /// A turn is in flight.
    Streaming,
    /// The engine exited.
    Closed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Starting => "starting",
            Self::Idle => "idle",
            Self::Streaming => "streaming",
            Self::Closed => "closed",
        }
    }

    /// Whether a session exists to send to.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Idle | Self::Streaming)
    }
}

/// How a permission ask was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allowed,
    /// Allowed, and a session rule was added so it will not be asked again.
    AllowedAlways,
    Denied,
}

impl Answer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Allowed => "Allowed",
            Self::AllowedAlways => "Allowed for this session",
            Self::Denied => "Denied",
        }
    }
}

/// A tool call and, once it lands, its result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCard {
    pub tool_use_id: String,
    pub name: String,
    pub input: Value,
    /// `None` while the call is in flight.
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub is_error: bool,
    pub content: Value,
}

impl ToolCard {
    pub fn is_pending(&self) -> bool {
        self.result.is_none()
    }

    pub fn is_error(&self) -> bool {
        self.result.as_ref().is_some_and(|r| r.is_error)
    }
}

/// A permission ask, and the answer if it has been given.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionCard {
    pub request_id: String,
    pub request: CanUseToolRequest,
    /// `None` while it is still blocking the turn.
    pub answer: Option<Answer>,
}

impl PermissionCard {
    pub fn is_pending(&self) -> bool {
        self.answer.is_none()
    }
}

/// How prominent a notice is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// One thing in the chat flow, in arrival order.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    User(String),
    /// Assistant prose. `streaming` while it is still being assembled from
    /// coalesced deltas.
    Assistant { text: String, streaming: bool },
    Tool(ToolCard),
    Permission(PermissionCard),
    Notice { text: String, level: Level },
}

/// Everything the window renders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub entries: Vec<Entry>,
    pub status: Status,
    pub session_id: String,
    pub model: String,
    pub cli_version: String,
    pub tools: Vec<String>,
    /// Running sum of `TurnComplete.total_cost_usd`.
    pub cost_total_usd: f64,
    pub turns: u32,
    /// Set when the session closes.
    pub exit_code: Option<i32>,
    /// The `subtype` of the last turn that ended in error, e.g.
    /// `error_max_turns`. Kept so the status bar can flag it rather than only
    /// showing that something went wrong.
    pub error_subtype: Option<String>,
    /// The engine's stderr, for a post-mortem after a bad exit.
    pub stderr: Vec<String>,
}

impl Transcript {
    /// Whether a turn is in flight — the input box is disabled and the interrupt
    /// control is shown while this is true.
    pub fn is_busy(&self) -> bool {
        self.status == Status::Streaming
    }

    /// Unanswered asks, in arrival order. The turn is blocked until these are
    /// answered, so the first one is what the user is being asked about.
    pub fn pending_permissions(&self) -> Vec<&PermissionCard> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Permission(card) if card.is_pending() => Some(card),
                _ => None,
            })
            .collect()
    }

    /// The engine is spawned and its stdin is open, so a turn can be sent.
    ///
    /// Not derived from `Init`, because the CLI emits `system/init` when the first
    /// *turn* begins rather than at handshake time. Waiting for it would leave the
    /// input box disabled with no way to ever enable it — the session would be
    /// live and unusable. Verified against the real CLI, which is the only place
    /// this shows: a stub that emits `init` up front hides it completely.
    pub fn mark_started(&mut self) {
        if self.status == Status::Starting {
            self.status = Status::Idle;
        }
    }

    /// Record a turn the user just sent. Not derived from an event: the CLI does
    /// not echo user turns back, so the only record of one is the act of sending.
    pub fn push_user(&mut self, text: String) {
        self.entries.push(Entry::User(text));
        self.status = Status::Streaming;
    }

    pub fn push_notice(&mut self, text: impl Into<String>, level: Level) {
        self.entries.push(Entry::Notice {
            text: text.into(),
            level,
        });
    }

    /// Mark an ask answered. Idempotent by construction: a card that already has
    /// an answer is left alone, so a double-click cannot answer twice — which
    /// matters because the second answer would be a protocol error on a
    /// request_id the CLI has already closed.
    pub fn answer_permission(&mut self, request_id: &str, answer: Answer) -> bool {
        for entry in &mut self.entries {
            if let Entry::Permission(card) = entry
                && card.request_id == request_id
            {
                if card.answer.is_some() {
                    return false;
                }
                card.answer = Some(answer);
                return true;
            }
        }
        false
    }

    /// Fold one event in.
    pub fn apply(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Init {
                session_id,
                model,
                cli_version,
                tools,
            } => {
                self.session_id = session_id;
                self.model = model;
                self.cli_version = cli_version;
                self.tools = tools;
                // A greeting turn may already be in flight, so an init does not
                // by itself mean idle.
                if self.status == Status::Starting {
                    self.status = Status::Idle;
                }
            }

            SessionEvent::TextDelta(text) => {
                self.status = Status::Streaming;
                match self.streaming_assistant() {
                    Some(existing) => existing.push_str(&text),
                    None => self.entries.push(Entry::Assistant {
                        text,
                        streaming: true,
                    }),
                }
            }

            // The final block *replaces* the accumulated deltas rather than being
            // appended to them: the CLI sends both, and appending would show every
            // sentence twice. Replacing also repairs anything the coalescer split
            // across a flush boundary mid-grapheme.
            SessionEvent::AssistantText(text) => match self.last_assistant_mut() {
                Some(entry) => {
                    *entry = Entry::Assistant {
                        text,
                        streaming: false,
                    };
                }
                None => self.entries.push(Entry::Assistant {
                    text,
                    streaming: false,
                }),
            },

            SessionEvent::ToolUse { id, name, input } => {
                self.settle_streaming();
                self.entries.push(Entry::Tool(ToolCard {
                    tool_use_id: id,
                    name,
                    input,
                    result: None,
                }));
            }

            SessionEvent::ToolResult {
                tool_use_id,
                is_error,
                content,
            } => {
                // Matched by id rather than by position: tool calls can be
                // in flight concurrently, and results do not have to come back
                // in the order the calls went out.
                let matched = self.entries.iter_mut().find_map(|entry| match entry {
                    Entry::Tool(card) if card.tool_use_id == tool_use_id => Some(card),
                    _ => None,
                });
                match matched {
                    Some(card) => card.result = Some(ToolResult { is_error, content }),
                    // A result with no call is not something to drop silently.
                    None => self.push_notice(
                        format!("result for an unknown tool call {tool_use_id}"),
                        Level::Warning,
                    ),
                }
            }

            SessionEvent::PermissionRequest {
                request_id,
                request,
            } => {
                self.settle_streaming();
                self.entries.push(Entry::Permission(PermissionCard {
                    request_id,
                    request,
                    answer: None,
                }));
            }

            // Answered by policy, so there is no card to show — but silence would
            // leave the user wondering why a tool they expected to be asked about
            // just ran.
            SessionEvent::PermissionResolved {
                tool_name, action, ..
            } => {
                let (text, level) = match action {
                    PermissionAction::Allow => (
                        format!("{tool_name} allowed automatically by this session's policy"),
                        Level::Info,
                    ),
                    PermissionAction::Deny => (
                        format!("{tool_name} denied by this session's policy"),
                        Level::Warning,
                    ),
                    // Unreachable in practice: an Ask surfaces instead.
                    PermissionAction::Ask => {
                        (format!("{tool_name} awaiting a decision"), Level::Info)
                    }
                };
                self.push_notice(text, level);
            }

            SessionEvent::TurnComplete {
                subtype,
                is_error,
                total_cost_usd,
                ..
            } => {
                self.settle_streaming();
                // Sum the per-turn costs rather than trusting a running total:
                // `total_cost_usd` is the cost of the turn that just ended.
                self.cost_total_usd += total_cost_usd.unwrap_or(0.0);
                self.turns += 1;
                if is_error {
                    self.error_subtype = Some(subtype.clone());
                    self.push_notice(format!("turn ended with {subtype}"), Level::Error);
                }
                if self.status != Status::Closed {
                    self.status = Status::Idle;
                }
            }

            SessionEvent::Stderr(line) => self.stderr.push(line),

            SessionEvent::ControlAck { request_id, result } => {
                // Successful acks are noise; a failed one explains why an
                // interrupt or a start did nothing.
                if let Err(error) = result {
                    self.push_notice(
                        format!("control request {request_id} failed: {error}"),
                        Level::Warning,
                    );
                }
            }

            SessionEvent::Unknown(detail) => self.push_notice(detail, Level::Info),

            SessionEvent::Closed { exit_code } => {
                self.settle_streaming();
                self.status = Status::Closed;
                self.exit_code = exit_code;
            }
        }
    }

    /// The text of the assistant entry currently streaming, if the last entry is
    /// one. Only the *last* entry counts: anything after it means that block is
    /// finished, whatever its flag says.
    fn streaming_assistant(&mut self) -> Option<&mut String> {
        match self.entries.last_mut() {
            Some(Entry::Assistant {
                text,
                streaming: true,
            }) => Some(text),
            _ => None,
        }
    }

    fn last_assistant_mut(&mut self) -> Option<&mut Entry> {
        match self.entries.last_mut() {
            entry @ Some(Entry::Assistant {
                streaming: true, ..
            }) => entry,
            _ => None,
        }
    }

    /// Stop treating the last entry as in-progress. Called before anything that
    /// interrupts prose, so a block left streaming by a tool call or a close does
    /// not keep a caret blinking on it forever.
    fn settle_streaming(&mut self) {
        if let Some(Entry::Assistant { streaming, .. }) = self.entries.last_mut() {
            *streaming = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(text: &str) -> SessionEvent {
        SessionEvent::TextDelta(text.to_owned())
    }

    fn turn(cost: f64) -> SessionEvent {
        SessionEvent::TurnComplete {
            subtype: "success".into(),
            is_error: false,
            total_cost_usd: Some(cost),
            num_turns: Some(1),
        }
    }

    fn ask(request_id: &str, tool: &str) -> SessionEvent {
        SessionEvent::PermissionRequest {
            request_id: request_id.to_owned(),
            request: CanUseToolRequest {
                tool_name: tool.to_owned(),
                input: serde_json::json!({}),
                tool_use_id: None,
                display_name: None,
                description: None,
                decision_reason: None,
                decision_reason_type: None,
                blocked_path: None,
            },
        }
    }

    fn assistant_texts(t: &Transcript) -> Vec<(&str, bool)> {
        t.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Assistant { text, streaming } => Some((text.as_str(), *streaming)),
                _ => None,
            })
            .collect()
    }

    /// #8's acceptance criterion: no duplicated text after a turn.
    #[test]
    fn the_final_block_replaces_the_streamed_deltas() {
        let mut t = Transcript::default();
        t.apply(delta("Hello, "));
        t.apply(delta("world"));
        assert_eq!(assistant_texts(&t), vec![("Hello, world", true)]);

        t.apply(SessionEvent::AssistantText("Hello, world".into()));
        assert_eq!(
            assistant_texts(&t),
            vec![("Hello, world", false)],
            "the final block must replace the deltas, not follow them"
        );
        assert_eq!(
            t.entries.len(),
            1,
            "one block of prose is one entry: {:#?}",
            t.entries
        );
    }

    #[test]
    fn a_final_block_that_differs_from_the_deltas_wins() {
        // The deltas can be a prefix if the stream was cut short, or differ if a
        // grapheme straddled a flush. The complete block is authoritative.
        let mut t = Transcript::default();
        t.apply(delta("Hel"));
        t.apply(SessionEvent::AssistantText("Hello, world".into()));
        assert_eq!(assistant_texts(&t), vec![("Hello, world", false)]);
    }

    #[test]
    fn two_blocks_in_one_turn_stay_separate() {
        let mut t = Transcript::default();
        t.apply(delta("first"));
        t.apply(SessionEvent::AssistantText("first".into()));
        t.apply(delta("second"));
        t.apply(SessionEvent::AssistantText("second".into()));
        assert_eq!(
            assistant_texts(&t),
            vec![("first", false), ("second", false)]
        );
    }

    #[test]
    fn a_tool_call_is_completed_by_its_own_result() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::ToolUse {
            id: "a".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        });
        t.apply(SessionEvent::ToolUse {
            id: "b".into(),
            name: "Read".into(),
            input: serde_json::json!({}),
        });
        // Out of order on purpose: results are matched by id, not by position.
        t.apply(SessionEvent::ToolResult {
            tool_use_id: "b".into(),
            is_error: false,
            content: serde_json::json!("contents"),
        });

        let cards: Vec<&ToolCard> = t
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Tool(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 2);
        assert!(cards[0].is_pending(), "Bash had no result yet");
        assert!(!cards[1].is_pending(), "Read's result should have landed");
        assert!(!cards[1].is_error());
    }

    /// #8's other acceptance criterion: an error result renders as a completed
    /// error card, not as a pending one and not as a success.
    #[test]
    fn an_error_result_completes_the_card_in_an_error_state() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::ToolUse {
            id: "a".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "false"}),
        });
        t.apply(SessionEvent::ToolResult {
            tool_use_id: "a".into(),
            is_error: true,
            content: serde_json::json!("exit status 1"),
        });

        let card = t
            .entries
            .iter()
            .find_map(|e| match e {
                Entry::Tool(c) => Some(c),
                _ => None,
            })
            .expect("a tool card");
        assert!(!card.is_pending(), "an error result still completes the card");
        assert!(card.is_error());
        assert_eq!(card.result.as_ref().unwrap().content, "exit status 1");
    }

    #[test]
    fn a_result_with_no_matching_call_is_reported_not_swallowed() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::ToolResult {
            tool_use_id: "ghost".into(),
            is_error: false,
            content: Value::Null,
        });
        assert!(matches!(
            t.entries.first(),
            Some(Entry::Notice {
                level: Level::Warning,
                ..
            })
        ));
    }

    /// #9: multiple asks in arrival order, each answerable exactly once.
    #[test]
    fn asks_queue_in_arrival_order_and_are_answerable_once() {
        let mut t = Transcript::default();
        t.apply(ask("r1", "Write"));
        t.apply(ask("r2", "Bash"));

        let pending: Vec<&str> = t
            .pending_permissions()
            .iter()
            .map(|c| c.request.tool_name.as_str())
            .collect();
        assert_eq!(pending, vec!["Write", "Bash"], "arrival order");

        assert!(t.answer_permission("r1", Answer::Allowed));
        assert!(
            !t.answer_permission("r1", Answer::Denied),
            "an answered card must be inert"
        );

        let card = t
            .entries
            .iter()
            .find_map(|e| match e {
                Entry::Permission(c) if c.request_id == "r1" => Some(c),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            card.answer,
            Some(Answer::Allowed),
            "the first answer stands"
        );

        let still_pending: Vec<&str> = t
            .pending_permissions()
            .iter()
            .map(|c| c.request_id.as_str())
            .collect();
        assert_eq!(still_pending, vec!["r2"]);
    }

    #[test]
    fn answering_an_unknown_request_is_refused() {
        let mut t = Transcript::default();
        assert!(!t.answer_permission("nope", Answer::Allowed));
    }

    /// #9: after "always allow", a later ask for the same tool does not surface a
    /// card. The suppression happens in the core's policy; what the frontend must
    /// do is not invent a card from the resolution notice.
    #[test]
    fn a_policy_resolved_ask_produces_a_notice_not_a_card() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::PermissionResolved {
            request_id: "r9".into(),
            tool_name: "Bash".into(),
            action: PermissionAction::Allow,
        });
        assert!(
            t.pending_permissions().is_empty(),
            "a resolved ask must not become a card"
        );
        assert!(matches!(
            t.entries.first(),
            Some(Entry::Notice {
                level: Level::Info,
                ..
            })
        ));
    }

    /// #10: cost accumulates across turns, and turns are counted.
    #[test]
    fn cost_accumulates_across_turns() {
        let mut t = Transcript::default();
        t.apply(turn(0.0125));
        assert_eq!(t.turns, 1);
        t.apply(turn(0.0250));
        assert_eq!(t.turns, 2);
        assert!(
            (t.cost_total_usd - 0.0375).abs() < 1e-9,
            "expected 0.0375, got {}",
            t.cost_total_usd
        );
    }

    #[test]
    fn a_turn_with_no_cost_reported_does_not_poison_the_total() {
        let mut t = Transcript::default();
        t.apply(turn(0.01));
        t.apply(SessionEvent::TurnComplete {
            subtype: "success".into(),
            is_error: false,
            total_cost_usd: None,
            num_turns: None,
        });
        assert!((t.cost_total_usd - 0.01).abs() < 1e-9);
        assert_eq!(t.turns, 2);
    }

    #[test]
    fn an_error_subtype_is_flagged_and_named() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::TurnComplete {
            subtype: "error_max_turns".into(),
            is_error: true,
            total_cost_usd: Some(0.02),
            num_turns: Some(9),
        });
        assert_eq!(t.error_subtype.as_deref(), Some("error_max_turns"));
        assert!(t.entries.iter().any(|e| matches!(
            e,
            Entry::Notice {
                level: Level::Error,
                ..
            }
        )));
    }

    /// #10: a closed session shows its exit code.
    #[test]
    fn closing_records_the_exit_code_and_ends_streaming() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::Init {
            session_id: "s".into(),
            model: "m".into(),
            cli_version: "2.1.220".into(),
            tools: vec![],
        });
        t.apply(delta("half a sen"));
        t.apply(SessionEvent::Closed { exit_code: Some(1) });

        assert_eq!(t.status, Status::Closed);
        assert_eq!(t.exit_code, Some(1));
        assert!(!t.is_busy());
        assert_eq!(
            assistant_texts(&t),
            vec![("half a sen", false)],
            "text cut off by a close must stop looking like it is still arriving"
        );
    }

    #[test]
    fn a_turn_completing_after_a_close_does_not_revive_the_session() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::Closed { exit_code: Some(0) });
        t.apply(turn(0.01));
        assert_eq!(t.status, Status::Closed, "closed is terminal");
    }

    #[test]
    fn init_moves_starting_to_idle_and_records_the_engine_facts() {
        let mut t = Transcript {
            status: Status::Starting,
            ..Transcript::default()
        };
        t.apply(SessionEvent::Init {
            session_id: "abc123".into(),
            model: "claude-sonnet-5".into(),
            cli_version: "2.1.220".into(),
            tools: vec!["Bash".into()],
        });
        assert_eq!(t.status, Status::Idle);
        assert_eq!(t.model, "claude-sonnet-5");
        assert_eq!(t.cli_version, "2.1.220");
        assert_eq!(t.session_id, "abc123");
        assert_eq!(t.tools, vec!["Bash".to_owned()]);
    }

    /// The real CLI emits `init` when the first turn begins, not at handshake
    /// time. If the input box waited for `init`, the first turn could never be
    /// sent and the session would be live and unusable.
    #[test]
    fn a_started_session_accepts_input_before_init_arrives() {
        let mut t = Transcript {
            status: Status::Starting,
            ..Transcript::default()
        };
        assert!(!t.status.is_live(), "starting is not yet sendable");

        t.mark_started();
        assert_eq!(t.status, Status::Idle);
        assert!(t.status.is_live(), "a started session must accept a turn");

        // And `init`, whenever it turns up, fills in the facts without disturbing
        // a turn that is already in flight.
        t.push_user("hello".into());
        assert_eq!(t.status, Status::Streaming);
        t.apply(SessionEvent::Init {
            session_id: "s".into(),
            model: "claude-sonnet-5".into(),
            cli_version: "2.1.220".into(),
            tools: vec![],
        });
        assert_eq!(
            t.status,
            Status::Streaming,
            "init must not interrupt a turn already running"
        );
        assert_eq!(t.model, "claude-sonnet-5");
    }

    #[test]
    fn mark_started_does_not_revive_a_closed_session() {
        let mut t = Transcript {
            status: Status::Closed,
            ..Transcript::default()
        };
        t.mark_started();
        assert_eq!(t.status, Status::Closed);
    }

    #[test]
    fn a_user_turn_makes_the_session_busy_until_the_turn_completes() {
        let mut t = Transcript {
            status: Status::Idle,
            ..Transcript::default()
        };
        t.push_user("hello".into());
        assert!(t.is_busy());
        t.apply(delta("hi"));
        assert!(t.is_busy());
        t.apply(turn(0.001));
        assert!(!t.is_busy());
        assert_eq!(t.status, Status::Idle);
    }

    #[test]
    fn stderr_is_collected_rather_than_shown_in_the_flow() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::Stderr("node: warning".into()));
        assert_eq!(t.stderr, vec!["node: warning".to_owned()]);
        assert!(
            t.entries.is_empty(),
            "engine diagnostics are not conversation"
        );
    }

    #[test]
    fn a_failed_control_ack_is_surfaced() {
        let mut t = Transcript::default();
        t.apply(SessionEvent::ControlAck {
            request_id: "epik-req-2".into(),
            result: Ok(Value::Null),
        });
        assert!(t.entries.is_empty(), "a successful ack is noise");

        t.apply(SessionEvent::ControlAck {
            request_id: "epik-req-3".into(),
            result: Err("interrupt rejected".into()),
        });
        assert!(matches!(
            t.entries.first(),
            Some(Entry::Notice {
                level: Level::Warning,
                ..
            })
        ));
    }

    #[test]
    fn a_tool_call_settles_the_prose_that_preceded_it() {
        let mut t = Transcript::default();
        t.apply(delta("Let me check"));
        t.apply(SessionEvent::ToolUse {
            id: "a".into(),
            name: "Bash".into(),
            input: Value::Null,
        });
        assert_eq!(assistant_texts(&t), vec![("Let me check", false)]);
    }
}
