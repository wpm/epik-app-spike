//! The chat pane: the transcript, and the box you type into.

use leptos::ev::KeyboardEvent;
use leptos::html;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;

use crate::components::permission_card::PermissionCardView;
use crate::components::tool_card::ToolCardView;
use crate::markdown;
use crate::session::SessionClient;
use crate::transcript::{Entry, Level, Status};

#[component]
pub fn ChatPane(client: SessionClient) -> impl IntoView {
    view! {
        <section class="flex-1 flex flex-col min-w-0 border-r border-line">
            <Transcript client=client />
            <Composer client=client />
        </section>
    }
}

#[component]
fn Transcript(client: SessionClient) -> impl IntoView {
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let transcript = client.transcript;

    // Follow the tail. Reading `entries` inside the effect is what subscribes it,
    // so this runs after every change — including each coalesced text flush, which
    // is the case that matters while a reply is streaming.
    Effect::new(move |_| {
        let _ = transcript.with(|t| t.entries.len());
        let _ = transcript.with(|t| {
            t.entries
                .last()
                .map(|e| format!("{e:?}").len())
                .unwrap_or(0)
        });
        if let Some(element) = scroller.get() {
            element.set_scroll_top(element.scroll_height());
        }
    });

    view! {
        <div node_ref=scroller class="flex-1 overflow-y-auto px-6 py-5">
            <div class="max-w-3xl mx-auto">
                {move || {
                    let entries = transcript.with(|t| t.entries.clone());
                    if entries.is_empty() {
                        return view! { <EmptyState client=client /> }.into_any();
                    }
                    // Rendered wholesale on each change rather than diffed by key.
                    // Deltas are already coalesced to ~30 ms, and a 0.2.0
                    // transcript is short; keying this is the optimisation to make
                    // when a long session shows it is needed, not before.
                    entries
                        .into_iter()
                        .map(|entry| view! { <EntryView entry=entry client=client /> })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </div>
    }
}

#[component]
fn EntryView(entry: Entry, client: SessionClient) -> impl IntoView {
    match entry {
        Entry::User(text) => view! {
            <div class="flex justify-end my-3">
                <div class="max-w-[85%] px-3 py-2 rounded-lg bg-raised text-sm text-fg
                            whitespace-pre-wrap selectable">
                    {text}
                </div>
            </div>
        }
        .into_any(),

        Entry::Assistant { text, streaming } => view! {
            <div class="my-3 text-sm text-fg selectable">
                // Markdown is rendered to HTML with raw markup escaped; see
                // crate::markdown for why that matters here.
                <div class="epik-prose" inner_html=markdown::to_html(&text)></div>
                {streaming.then(|| view! {
                    <span class="inline-block w-1.5 h-4 align-text-bottom bg-accent
                                 animate-pulse"></span>
                })}
            </div>
        }
        .into_any(),

        Entry::Tool(card) => view! { <ToolCardView card=card /> }.into_any(),

        Entry::Permission(card) => {
            view! { <PermissionCardView card=card client=client /> }.into_any()
        }

        Entry::Notice { text, level } => {
            let tone = match level {
                Level::Info => "text-fg-muted",
                Level::Warning => "text-warning",
                Level::Error => "text-error",
            };
            view! {
                <p class=format!("my-2 text-xs font-mono selectable {tone}")>{text}</p>
            }
            .into_any()
        }
    }
}

#[component]
fn EmptyState(client: SessionClient) -> impl IntoView {
    let status = Memo::new(move |_| client.transcript.with(|t| t.status));
    view! {
        <div class="h-full flex items-center justify-center py-24">
            <p class="text-sm text-fg-muted">
                {move || match status.get() {
                    Status::Starting => "Starting the engine…",
                    Status::Offline => "No session.",
                    Status::Closed => "The session has ended.",
                    _ => "Say something.",
                }}
            </p>
        </div>
    }
}

/// The input box and the controls that belong with it.
#[component]
fn Composer(client: SessionClient) -> impl IntoView {
    let (draft, set_draft) = signal(String::new());
    let transcript = client.transcript;
    let status = Memo::new(move |_| transcript.with(|t| t.status));
    let busy = Memo::new(move |_| transcript.with(|t| t.is_busy()));
    // A pending ask blocks the turn, so a message typed now would sit unread
    // behind it. Better to say so than to accept input that goes nowhere.
    let blocked = Memo::new(move |_| transcript.with(|t| !t.pending_permissions().is_empty()));
    let can_send = Memo::new(move |_| status.get().is_live() && !busy.get() && !blocked.get());

    let send = move || {
        let text = draft.get_untracked();
        if text.trim().is_empty() || !can_send.get_untracked() {
            return;
        }
        set_draft.set(String::new());
        client.send(text);
    };

    // Enter sends; Shift+Enter is a newline, because multi-line prompts are
    // common and losing one to a reflex is worse than needing a modifier.
    let on_keydown = move |event: KeyboardEvent| {
        if event.key() == "Enter" && !event.shift_key() {
            event.prevent_default();
            send();
        }
    };

    view! {
        <div class="shrink-0 border-t border-line bg-bar px-6 py-4">
            <div class="max-w-3xl mx-auto">
                <div class="flex items-end gap-2">
                    <textarea
                        class="flex-1 resize-none rounded-md bg-input border border-line
                               px-3 py-2 text-sm text-fg placeholder:text-fg-faint
                               focus:outline-none focus:border-line-strong
                               disabled:opacity-50 selectable"
                        rows="2"
                        placeholder=move || match status.get() {
                            Status::Offline => "No session.",
                            Status::Starting => "Starting…",
                            Status::Closed => "The session has ended.",
                            _ if blocked.get() => "Answer the permission request above.",
                            _ if busy.get() => "Working…",
                            _ => "Message Epik — Enter to send, Shift+Enter for a newline",
                        }
                        prop:value=move || draft.get()
                        disabled=move || !can_send.get()
                        on:input=move |event| {
                            let value = event
                                .target()
                                .and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
                                .map(|t| t.value())
                                .unwrap_or_default();
                            set_draft.set(value);
                        }
                        on:keydown=on_keydown
                    ></textarea>

                    // Interrupt replaces Send while a turn is in flight: they are
                    // never both useful, and one button in one place is easier to
                    // hit in a hurry — which is when interrupt gets used.
                    {move || if busy.get() {
                        view! {
                            <button
                                class="shrink-0 px-3 py-2 rounded-md text-sm font-medium
                                       bg-warning-muted text-warning border border-warning/40
                                       hover:bg-warning/20"
                                title="Interrupt this turn"
                                on:click=move |_| client.interrupt()
                            >
                                "Stop"
                            </button>
                        }
                        .into_any()
                    } else {
                        view! {
                            <button
                                class="shrink-0 px-3 py-2 rounded-md text-sm font-medium
                                       bg-accent text-on-accent hover:bg-accent-hover
                                       disabled:opacity-40 disabled:cursor-default"
                                disabled=move || !can_send.get()
                                on:click=move |_| send()
                            >
                                "Send"
                            </button>
                        }
                        .into_any()
                    }}
                </div>
            </div>
        </div>
    }
}
