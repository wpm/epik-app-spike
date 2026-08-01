//! Tool activity as a collapsible card.
//!
//! A card opens pending when the call goes out and completes when the result with
//! the matching id comes back — success or error. Payloads are collapsed by
//! default: a `Read` result is thousands of lines, and a transcript that inlined
//! them would bury the conversation. The summary line carries the one detail that
//! usually answers "what is it doing" without expanding anything.

use leptos::prelude::*;
use serde_json::Value;

use crate::transcript::ToolCard;

/// The field worth showing on the summary line, per tool. These are the inputs a
/// reader actually wants: which command, which file, which query.
fn summary(input: &Value) -> Option<String> {
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
    ] {
        if let Some(text) = input.get(key).and_then(Value::as_str) {
            return Some(text.replace('\n', " "));
        }
    }
    None
}

fn pretty(value: &Value) -> String {
    // Strings arrive as strings often enough (tool results especially) that
    // quoting and escaping them would be noise.
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[component]
pub fn ToolCardView(card: ToolCard) -> impl IntoView {
    let pending = card.is_pending();
    let errored = card.is_error();

    // The status dot carries the state at a glance; the border tint reinforces it
    // for the error case, which is the one worth noticing without reading.
    let (dot, dot_title) = if pending {
        ("bg-warning animate-pulse", "running")
    } else if errored {
        ("bg-error", "failed")
    } else {
        ("bg-success", "succeeded")
    };
    let border = if errored {
        "border-error/40"
    } else {
        "border-line"
    };

    let detail = summary(&card.input);
    let input_json = pretty(&card.input);
    let result = card.result.clone();

    view! {
        <div class=format!(
            "my-2 rounded-md border bg-surface overflow-hidden {border}"
        )>
            <details>
                <summary class="flex items-center gap-2 px-3 py-2 cursor-pointer
                                hover:bg-hover text-xs">
                    <span
                        class=format!("w-1.5 h-1.5 rounded-full shrink-0 {dot}")
                        title=dot_title
                    ></span>
                    <span class="font-mono text-fg font-medium">{card.name.clone()}</span>
                    {detail.map(|detail| view! {
                        <span class="font-mono text-fg-muted truncate">{detail}</span>
                    })}
                    {errored.then(|| view! {
                        <span class="ml-auto shrink-0 px-1.5 py-0.5 rounded text-[10px]
                                     uppercase tracking-wide bg-error-muted text-error">
                            "error"
                        </span>
                    })}
                </summary>

                <div class="px-3 pb-3 pt-1 space-y-3 border-t border-line">
                    <Payload label="Input" body=input_json is_error=false />
                    {match result {
                        Some(result) => view! {
                            <Payload
                                label=if result.is_error { "Error" } else { "Result" }
                                body=pretty(&result.content)
                                is_error=result.is_error
                            />
                        }
                        .into_any(),
                        None => view! {
                            <p class="text-xs text-fg-muted">"Waiting for the result…"</p>
                        }
                        .into_any(),
                    }}
                </div>
            </details>
        </div>
    }
}

#[component]
fn Payload(label: &'static str, body: String, is_error: bool) -> impl IntoView {
    let tone = if is_error {
        "text-error"
    } else {
        "text-fg-secondary"
    };
    view! {
        <div>
            <p class="text-[10px] uppercase tracking-wide text-fg-muted mb-1">{label}</p>
            // max-h with scroll rather than unbounded: one enormous payload must
            // not push the rest of the conversation off the screen.
            <pre class=format!(
                "text-xs font-mono whitespace-pre-wrap break-all max-h-64 overflow-auto
                 p-2 rounded bg-root selectable {tone}"
            )>{body}</pre>
        </div>
    }
}
