//! The root component: decide what the window is showing, then show it.
//!
//! Exactly one of three things is on screen — no host, the doctor report, or the
//! interface — and which one is a function of the engine report rather than of
//! anything the user did. That is the shape #6 asks for: an unusable engine means
//! the window renders the doctor report *instead of* chat, not chat with an error
//! banner over it.

use epik_core::EngineReport;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::doctor_screen::{DoctorScreen, NoHostScreen};
use crate::ipc;

/// What the window is showing.
#[derive(Debug, Clone, PartialEq)]
enum Screen {
    /// Resolving the engine. Brief, but it does spawn a process, so it is a
    /// state rather than an assumption.
    Loading,
    /// The frontend is not inside the Tauri window.
    NoHost(ipc::IpcError),
    /// Engine resolved. `report.supported()` decides doctor vs interface.
    Resolved(EngineReport),
}

#[component]
pub fn App() -> impl IntoView {
    let (screen, set_screen) = signal(Screen::Loading);
    let (retrying, set_retrying) = signal(false);

    // The component body runs once in a CSR app, so this is the mount hook.
    let resolve = move |command: &'static str| {
        spawn_local(async move {
            match ipc::call::<EngineReport>(command).await {
                Ok(report) => set_screen.set(Screen::Resolved(report)),
                Err(err) => set_screen.set(Screen::NoHost(err)),
            }
            set_retrying.set(false);
        });
    };
    resolve("engine_report");

    let on_retry = Callback::new(move |()| {
        set_retrying.set(true);
        // `doctor` re-runs resolution; `engine_report` would return the cached
        // answer and the retry button would do nothing.
        resolve("doctor");
    });

    view! {
        {move || match screen.get() {
            Screen::Loading => view! { <SplashScreen /> }.into_any(),
            Screen::NoHost(err) => view! { <NoHostScreen error=err /> }.into_any(),
            Screen::Resolved(report) if !report.supported() => view! {
                <DoctorScreen report=report on_retry=on_retry retrying=retrying.into() />
            }
            .into_any(),
            Screen::Resolved(report) => view! { <Shell report=report /> }.into_any(),
        }}
    }
}

#[component]
fn SplashScreen() -> impl IntoView {
    view! {
        <main class="h-full flex items-center justify-center bg-root">
            <span class="text-sm text-fg-muted">"Looking for Claude Code…"</span>
        </main>
    }
}

/// The two-panel window: chat on the left, a right panel that holds session
/// metadata in 0.2.0. The panes themselves arrive with #8, #9 and #10; this is
/// the frame they mount into, and it is here so the doctor/chat decision above
/// has something real to choose between.
#[component]
fn Shell(report: EngineReport) -> impl IntoView {
    let version = report
        .version_raw
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());
    let engine_path = report
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    view! {
        <div class="h-full flex flex-col bg-root">
            <div class="flex-1 flex min-h-0">
                <section class="flex-1 flex flex-col min-w-0 border-r border-line">
                    <div class="flex-1 flex items-center justify-center">
                        <p class="text-sm text-fg-muted">"Chat pane arrives with #8."</p>
                    </div>
                </section>
                <aside class="w-72 shrink-0 flex flex-col bg-surface">
                    <h2 class="px-4 py-3 text-xs uppercase tracking-wide text-fg-muted
                               border-b border-line">
                        "Session"
                    </h2>
                    <dl class="p-4 space-y-3 text-xs">
                        <div>
                            <dt class="text-fg-muted mb-0.5">"Engine"</dt>
                            <dd class="font-mono text-fg-secondary break-all selectable">
                                {engine_path}
                            </dd>
                        </div>
                        <div>
                            <dt class="text-fg-muted mb-0.5">"CLI version"</dt>
                            <dd class="font-mono text-fg-secondary selectable">{version}</dd>
                        </div>
                    </dl>
                </aside>
            </div>
        </div>
    }
}
