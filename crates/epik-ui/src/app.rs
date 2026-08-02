//! The root component: decide what the window is showing, then show it.
//!
//! Exactly one of three things is on screen — no host, the doctor report, or the
//! interface — and which one is a function of the engine report rather than of
//! anything the user did. That is the shape #6 asks for: an unusable engine means
//! the window renders the doctor report *instead of* chat, not chat with an error
//! banner over it.

use epik_core::{EngineReport, Profile};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::components::chat::ChatPane;
use crate::components::session_panel::SessionPanel;
use crate::components::status_bar::StatusBar;
use crate::doctor_screen::{DoctorScreen, NoHostScreen};
use crate::ipc;
use crate::session::SessionClient;

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

/// The two-panel window: chat on the left, session metadata on the right.
///
/// The session starts on its own. An app whose first action is always "click to
/// begin" is asking a question with one answer; the engine is already resolved by
/// the time this mounts, so there is nothing left to decide.
#[component]
fn Shell(report: EngineReport) -> impl IntoView {
    let client = SessionClient::new();
    client.start(Profile::default());

    view! {
        <div class="h-full flex flex-col bg-root">
            <div class="flex-1 flex min-h-0">
                <ChatPane client=client />
                <SessionPanel client=client report=report />
            </div>
            // Full width beneath both panels: the session's state belongs to the
            // window, not to one half of it.
            <StatusBar client=client />
        </div>
    }
}
