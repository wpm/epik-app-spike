//! The right panel. Reserved by the two-panel layout; in 0.2.0 it holds session
//! metadata — the facts you would otherwise have to scroll the transcript to find.

use epik_core::EngineReport;
use leptos::prelude::*;

use crate::session::SessionClient;
use crate::transcript::Status;

#[component]
pub fn SessionPanel(client: SessionClient, report: EngineReport) -> impl IntoView {
    let transcript = client.transcript;
    let engine_path = report
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    view! {
        <aside class="w-80 shrink-0 flex flex-col bg-surface overflow-y-auto">
            <h2 class="px-4 py-3 text-xs uppercase tracking-wide text-fg-muted
                       border-b border-line shrink-0">
                "Session"
            </h2>

            <dl class="p-4 space-y-3 text-xs">
                <Field label="Profile">
                    {move || {
                        client
                            .profile
                            .get()
                            .map(|profile| profile.label().to_owned())
                            .unwrap_or_else(|| "—".to_owned())
                    }}
                </Field>
                <Field label="Model">
                    {move || or_dash(transcript.with(|t| t.model.clone()))}
                </Field>
                <Field label="Session id">
                    {move || or_dash(transcript.with(|t| t.session_id.clone()))}
                </Field>
                <Field label="CLI version">
                    {move || or_dash(transcript.with(|t| t.cli_version.clone()))}
                </Field>
                <Field label="Engine">{engine_path}</Field>
                <Field label="Tools">
                    {move || {
                        let count = transcript.with(|t| t.tools.len());
                        if count == 0 {
                            "—".to_owned()
                        } else {
                            format!("{count} available")
                        }
                    }}
                </Field>
            </dl>

            // Engine diagnostics, kept out of the conversation but not hidden:
            // this is the first thing to read when a session dies unexpectedly.
            {move || {
                let lines = transcript.with(|t| t.stderr.clone());
                (!lines.is_empty()).then(|| {
                    let closed_badly = transcript
                        .with(|t| t.status == Status::Closed && t.exit_code != Some(0));
                    view! {
                        <div class="px-4 pb-4">
                            <details open=closed_badly>
                                <summary class="text-xs text-fg-muted cursor-pointer
                                                hover:text-fg-secondary">
                                    {format!("Engine output ({} lines)", lines.len())}
                                </summary>
                                <pre class="mt-2 p-2 rounded bg-root text-[11px] font-mono
                                            text-fg-secondary whitespace-pre-wrap
                                            max-h-48 overflow-auto selectable">
                                    {lines.join("\n")}
                                </pre>
                            </details>
                        </div>
                    }
                })
            }}
        </aside>
    }
}

fn or_dash(value: String) -> String {
    if value.is_empty() {
        "—".to_owned()
    } else {
        value
    }
}

#[component]
fn Field(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div>
            <dt class="text-fg-muted mb-0.5">{label}</dt>
            <dd class="font-mono text-fg-secondary break-all selectable">{children()}</dd>
        </div>
    }
}
