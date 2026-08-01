//! The permission card: the one place the user is in charge.
//!
//! A card blocks the turn. Nothing else in the session proceeds until it is
//! answered, so it is rendered as an interruption rather than as a notification —
//! accent border, full input shown, three explicit choices, no default action and
//! nothing dismissible. A card that could be ignored would look like one that did
//! not matter.
//!
//! An answered card stays in the flow rather than disappearing. What was asked
//! and what was decided is the most useful part of a transcript to look back at,
//! and a card that vanished on click would leave the tool call it authorised with
//! no visible provenance.

use leptos::ev::KeyboardEvent;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use serde_json::Value;

use crate::session::SessionClient;
use crate::transcript::{Answer, PermissionCard};

/// The formatted input. A tool's arguments are what is actually being authorised,
/// so unlike a tool card this is shown expanded — deciding without reading the
/// command would make the prompt theatre.
fn formatted_input(input: &Value) -> String {
    match input {
        Value::String(text) => text.clone(),
        Value::Null => "(no arguments)".to_owned(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[component]
pub fn PermissionCardView(card: PermissionCard, client: SessionClient) -> impl IntoView {
    match card.answer {
        Some(answer) => view! { <Answered card=card answer=answer /> }.into_any(),
        None => view! { <Pending card=card client=client /> }.into_any(),
    }
}

#[component]
fn Pending(card: PermissionCard, client: SessionClient) -> impl IntoView {
    let (deny_message, set_deny_message) = signal(String::new());
    let (denying, set_denying) = signal(false);

    // In a `StoredValue` rather than captured directly: a closure holding a
    // `String` is `FnOnce`, and these have to be `Copy` to be shared between the
    // three buttons and the message field's Enter handler.
    let request_id = StoredValue::new(card.request_id.clone());
    let tool_name = card.request.tool_name.clone();
    let display_name = card
        .request
        .display_name
        .clone()
        .unwrap_or_else(|| tool_name.clone());

    let answer = move |answer: Answer, message: Option<String>| {
        client.respond(request_id.get_value(), answer, message);
    };

    // `answer` captures only `Copy` values, so these three can each take their
    // own copy of it without cloning anything.
    let allow = move |_| answer(Answer::Allowed, None);
    let always = move |_| answer(Answer::AllowedAlways, None);
    let deny = move |_| {
        let message = deny_message.get_untracked();
        let message = message.trim();
        answer(
            Answer::Denied,
            (!message.is_empty()).then(|| message.to_owned()),
        )
    };

    view! {
        <div class="my-4 rounded-lg border border-accent/50 bg-surface overflow-hidden">
            <div class="px-4 py-3 border-b border-line bg-accent-muted">
                <div class="flex items-baseline gap-2 flex-wrap">
                    <span class="text-xs uppercase tracking-wide text-accent font-medium">
                        "Permission required"
                    </span>
                    <span class="font-mono text-sm text-fg">{display_name}</span>
                </div>
                {card.request.description.clone().map(|description| view! {
                    <p class="mt-1 text-xs text-fg-secondary selectable">{description}</p>
                })}
            </div>

            <div class="px-4 py-3 space-y-3">
                <div>
                    <p class="text-[10px] uppercase tracking-wide text-fg-muted mb-1">"Input"</p>
                    <pre class="text-xs font-mono whitespace-pre-wrap break-all max-h-48
                                overflow-auto p-2 rounded bg-root text-fg-secondary
                                selectable">
                        {formatted_input(&card.request.input)}
                    </pre>
                </div>

                // Why the engine is asking, when it says. Useful for judging
                // whether an ask is routine or unexpected.
                <Metadata card=card.clone() />

                {move || denying.get().then(|| view! {
                    <div>
                        <label class="block text-[10px] uppercase tracking-wide text-fg-muted
                                      mb-1">
                            "Message to the model (optional)"
                        </label>
                        <input
                            class="w-full rounded bg-input border border-line px-2 py-1.5
                                   text-xs text-fg placeholder:text-fg-faint
                                   focus:outline-none focus:border-line-strong selectable"
                            placeholder="Why not — the model reads this and adapts"
                            prop:value=move || deny_message.get()
                            on:input=move |event| {
                                let value = event
                                    .target()
                                    .and_then(|t| {
                                        t.dyn_into::<web_sys::HtmlInputElement>().ok()
                                    })
                                    .map(|t| t.value())
                                    .unwrap_or_default();
                                set_deny_message.set(value);
                            }
                            on:keydown=move |event: KeyboardEvent| {
                                if event.key() == "Enter" {
                                    event.prevent_default();
                                    deny(());
                                }
                            }
                        />
                    </div>
                })}

                <div class="flex flex-wrap items-center gap-2 pt-1">
                    <button
                        class="px-3 py-1.5 rounded-md text-xs font-medium bg-accent
                               text-on-accent hover:bg-accent-hover"
                        on:click=allow
                    >
                        "Allow"
                    </button>
                    <button
                        class="px-3 py-1.5 rounded-md text-xs font-medium border border-line
                               text-fg-secondary hover:bg-hover hover:text-fg"
                        title="Add a session rule allowing this tool, then allow"
                        on:click=always
                    >
                        "Always allow this tool"
                    </button>
                    <button
                        class="px-3 py-1.5 rounded-md text-xs font-medium border
                               border-error/40 text-error hover:bg-error-muted"
                        on:click=move |_| {
                            // First click reveals the message box, second denies.
                            // Deny is not destructive, but it does end a turn's
                            // plan, and the model copes far better with a reason
                            // than with a bare refusal — so it is worth one extra
                            // click to offer the chance to give one.
                            if denying.get_untracked() {
                                deny(());
                            } else {
                                set_denying.set(true);
                            }
                        }
                    >
                        {move || if denying.get() { "Confirm deny" } else { "Deny" }}
                    </button>
                    {move || denying.get().then(|| view! {
                        <button
                            class="px-2 py-1.5 rounded-md text-xs text-fg-muted hover:text-fg"
                            on:click=move |_| set_denying.set(false)
                        >
                            "Cancel"
                        </button>
                    })}
                </div>
            </div>
        </div>
    }
}

#[component]
fn Metadata(card: PermissionCard) -> impl IntoView {
    let reason = card.request.decision_reason.clone();
    let reason_type = card.request.decision_reason_type.clone();
    let blocked_path = card.request.blocked_path.clone();
    if reason.is_none() && reason_type.is_none() && blocked_path.is_none() {
        return ().into_any();
    }
    view! {
        <dl class="text-[11px] space-y-1">
            {reason.map(|reason| view! {
                <Row label="Reason">{reason}</Row>
            })}
            {reason_type.map(|kind| view! {
                <Row label="Reason type">{kind}</Row>
            })}
            {blocked_path.map(|path| view! {
                <Row label="Blocked path">{path}</Row>
            })}
        </dl>
    }
    .into_any()
}

#[component]
fn Row(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="flex gap-2">
            <dt class="text-fg-muted shrink-0">{label}</dt>
            <dd class="font-mono text-fg-secondary break-all selectable">{children()}</dd>
        </div>
    }
}

/// An answered card: inert, and showing what was decided.
#[component]
fn Answered(card: PermissionCard, answer: Answer) -> impl IntoView {
    let (tone, dot) = match answer {
        Answer::Allowed | Answer::AllowedAlways => ("text-success", "bg-success"),
        Answer::Denied => ("text-error", "bg-error"),
    };
    let tool_name = card.request.tool_name.clone();

    view! {
        <div class="my-3 rounded-md border border-line bg-surface">
            <details>
                <summary class="flex items-center gap-2 px-3 py-2 cursor-pointer
                                hover:bg-hover text-xs">
                    <span class=format!("w-1.5 h-1.5 rounded-full shrink-0 {dot}")></span>
                    <span class="text-fg-muted">"Permission"</span>
                    <span class="font-mono text-fg">{tool_name}</span>
                    <span class=format!("ml-auto shrink-0 {tone}")>{answer.label()}</span>
                </summary>
                <div class="px-3 pb-3 pt-1 border-t border-line">
                    <pre class="text-xs font-mono whitespace-pre-wrap break-all max-h-48
                                overflow-auto p-2 rounded bg-root text-fg-secondary
                                selectable">
                        {formatted_input(&card.request.input)}
                    </pre>
                </div>
            </details>
        </div>
    }
}
