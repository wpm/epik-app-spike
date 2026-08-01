//! What the window shows instead of chat when the engine is not usable.
//!
//! Rendered from an [`EngineReport`] the host produced — the same struct, not a
//! JSON blob picked apart here — so the wording of an engine problem is decided
//! once, in `epik-core`, and this only decides where it goes on screen.

use epik_core::{EngineReport, EngineStatus};
use leptos::prelude::*;

use crate::ipc;

#[component]
pub fn DoctorScreen(
    report: EngineReport,
    /// Re-runs resolution. Someone who installs Claude Code with the window open
    /// should not have to restart the app.
    on_retry: Callback<()>,
    retrying: Signal<bool>,
) -> impl IntoView {
    let status_colour = match report.status {
        EngineStatus::Ok => "text-success",
        EngineStatus::TooOld | EngineStatus::Unusable => "text-warning",
        EngineStatus::NotFound => "text-error",
    };
    let headline = report.headline();
    let remedy = report.remedy();
    let details = report.report_text();

    view! {
        <main class="h-full flex items-center justify-center bg-root px-8">
            <div class="w-full max-w-2xl">
                <div class="flex items-baseline gap-3 mb-6">
                    <span class="text-accent font-mono text-sm">"epik"</span>
                    <h1 class="text-lg font-medium text-fg">"Claude Code is required"</h1>
                </div>

                <p class=format!("text-sm mb-6 {status_colour}")>{headline}</p>

                {remedy.map(|remedy| view! {
                    <p class="text-sm text-fg-secondary mb-6 leading-relaxed selectable">
                        {remedy}
                    </p>
                })}

                <details class="mb-6">
                    <summary class="text-xs text-fg-muted cursor-pointer hover:text-fg-secondary">
                        "Details"
                    </summary>
                    <pre class="mt-3 p-4 rounded-md bg-surface border border-line text-xs
                                font-mono text-fg-secondary whitespace-pre-wrap selectable">
                        {details}
                    </pre>
                </details>

                <button
                    class="px-4 py-2 rounded-md bg-accent text-on-accent text-sm font-medium
                           hover:bg-accent-hover disabled:opacity-50 disabled:cursor-default"
                    disabled=move || retrying.get()
                    on:click=move |_| on_retry.run(())
                >
                    {move || if retrying.get() { "Checking…" } else { "Check again" }}
                </button>
            </div>
        </main>
    }
}

/// Shown when the frontend is loaded outside the Tauri window — `trunk serve` in
/// a browser, most likely. Distinct from a doctor screen because the problem is
/// not the engine.
#[component]
pub fn NoHostScreen(error: ipc::IpcError) -> impl IntoView {
    view! {
        <main class="h-full flex items-center justify-center bg-root px-8">
            <div class="max-w-xl text-center">
                <h1 class="text-lg font-medium text-fg mb-3">"No Epik host"</h1>
                <p class="text-sm text-fg-secondary mb-4 selectable">{error.to_string()}</p>
                <p class="text-xs text-fg-muted">
                    "Run the app with "
                    <code class="font-mono text-fg-secondary">"cargo tauri dev"</code>
                    " rather than opening the frontend in a browser."
                </p>
            </div>
        </main>
    }
}
