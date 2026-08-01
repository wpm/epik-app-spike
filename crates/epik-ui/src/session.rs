//! The live session, from the frontend's side: one channel in, commands out.
//!
//! [`SessionClient`] is `Copy`, so components take it by value and every one of
//! them reads the same [`Transcript`] signal. The IPC channel itself is not
//! `Copy` and must outlive the call that created it — the host writes to a JS
//! closure it holds — so it lives in a thread-local rather than in the signal
//! graph. That is safe here because wasm is single-threaded, and replacing it
//! drops the previous session's handler, which is exactly what should happen.

use std::cell::RefCell;

use epik_core::{PermissionDecision, PermissionRule, Profile};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::ipc::{self, EventChannel};
use crate::transcript::{Answer, Level, Status, Transcript};

thread_local! {
    /// The channel the host writes session events to. Held for as long as the
    /// session lives; replaced (and so dropped) when a new session starts.
    static CHANNEL: RefCell<Option<EventChannel>> = const { RefCell::new(None) };
}

/// Argument envelopes. These are not mirrors of host types — they are the shape
/// of a command's parameter list, and their fields are `epik-core` types.
#[derive(Debug, Clone, Serialize)]
struct StartArgs {
    profile: Profile,
}

#[derive(Debug, Clone, Serialize)]
struct SendArgs {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct RespondArgs {
    #[serde(rename = "requestId")]
    request_id: String,
    decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize)]
struct RuleArgs {
    rule: PermissionRule,
}

#[derive(Clone, Copy)]
pub struct SessionClient {
    pub transcript: RwSignal<Transcript>,
    /// The profile the live session was started with, if any.
    pub profile: RwSignal<Option<Profile>>,
}

impl Default for SessionClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionClient {
    pub fn new() -> Self {
        Self {
            transcript: RwSignal::new(Transcript::default()),
            profile: RwSignal::new(None),
        }
    }

    fn note(self, message: impl Into<String>, level: Level) {
        self.transcript
            .update(|t| t.push_notice(message.into(), level));
    }

    /// Report a failed command in the flow rather than the console. A command
    /// that silently does nothing is indistinguishable from a hung engine.
    fn fail(self, what: &str, err: ipc::IpcError) {
        self.note(format!("{what} failed: {err}"), Level::Error);
    }

    /// Start a session for `profile` and begin consuming its events.
    pub fn start(self, profile: Profile) {
        self.transcript.update(|t| {
            *t = Transcript {
                status: Status::Starting,
                ..Transcript::default()
            };
        });

        let transcript = self.transcript;
        let channel = EventChannel::new(
            // Every event the host serialized, deserialized back into the same
            // `epik_core::SessionEvent` and folded into the view state.
            move |event| transcript.update(|t| t.apply(event)),
            move |error| {
                transcript.update(|t| {
                    t.push_notice(format!("undecodable event: {error}"), Level::Error);
                });
            },
        );
        let channel = match channel {
            Ok(channel) => channel,
            Err(err) => {
                self.fail("opening the event channel", err);
                self.transcript.update(|t| t.status = Status::Offline);
                return;
            }
        };

        spawn_local(async move {
            let started =
                ipc::call_with_channel::<()>("start_session", &StartArgs { profile }, &channel)
                    .await;
            match started {
                Ok(()) => {
                    // Only now is the channel worth keeping: a failed start has
                    // nothing to deliver. Everything else about the session
                    // arrives on it as the `Init` event.
                    CHANNEL.with(|slot| *slot.borrow_mut() = Some(channel));
                    self.profile.set(Some(profile));
                }
                Err(err) => {
                    self.fail("starting the session", err);
                    self.transcript.update(|t| t.status = Status::Offline);
                }
            }
        });
    }

    /// Send a user turn. The transcript records it immediately: the CLI does not
    /// echo user turns, so waiting for confirmation would mean typing into a void.
    pub fn send(self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.transcript.update(|t| t.push_user(text.clone()));
        spawn_local(async move {
            if let Err(err) = ipc::call_with::<()>("send_user", &SendArgs { text }).await {
                self.fail("sending", err);
                self.transcript.update(|t| t.status = Status::Idle);
            }
        });
    }

    /// Answer a permission ask. `answer` decides what the model is told and
    /// whether a session rule is added first.
    pub fn respond(self, request_id: String, answer: Answer, deny_message: Option<String>) {
        // Mark it answered before the round trip so a second click cannot answer
        // the same request twice — the CLI would reject the second answer, and
        // the user would see an error for having been quick.
        let accepted = self
            .transcript
            .try_update(|t| t.answer_permission(&request_id, answer))
            .unwrap_or(false);
        if !accepted {
            return;
        }

        let tool_name = self.tool_name_for(&request_id);
        spawn_local(async move {
            // "Always allow" is a rule *then* an allow: the rule has to be in
            // place before the next ask for that tool arrives, and the CLI may
            // ask again immediately.
            if answer == Answer::AllowedAlways
                && let Some(tool_name) = tool_name
            {
                let rule = PermissionRule::allow(tool_name);
                if let Err(err) =
                    ipc::call_with::<()>("add_session_allow_rule", &RuleArgs { rule }).await
                {
                    self.fail("adding the session rule", err);
                }
            }

            let decision = match answer {
                Answer::Allowed | Answer::AllowedAlways => PermissionDecision::Allow {
                    updated_input: None,
                },
                Answer::Denied => PermissionDecision::Deny {
                    message: deny_message.unwrap_or_else(|| {
                        "The user declined this tool call in Epik.".to_owned()
                    }),
                },
            };
            if let Err(err) = ipc::call_with::<()>(
                "respond_permission",
                &RespondArgs {
                    request_id,
                    decision,
                },
            )
            .await
            {
                self.fail("answering the permission request", err);
            }
        });
    }

    fn tool_name_for(self, request_id: &str) -> Option<String> {
        self.transcript.with_untracked(|t| {
            t.entries.iter().find_map(|entry| match entry {
                crate::transcript::Entry::Permission(card) if card.request_id == request_id => {
                    Some(card.request.tool_name.clone())
                }
                _ => None,
            })
        })
    }

    /// Interrupt the turn in flight.
    pub fn interrupt(self) {
        spawn_local(async move {
            if let Err(err) = ipc::call::<()>("interrupt").await {
                self.fail("interrupting", err);
            }
        });
    }

    /// End the session. Resolves once the engine is reaped.
    pub fn end(self) {
        spawn_local(async move {
            if let Err(err) = ipc::call::<()>("end_session").await {
                self.fail("ending the session", err);
            }
            CHANNEL.with(|slot| *slot.borrow_mut() = None);
        });
    }
}
