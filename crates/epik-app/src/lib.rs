//! Tauri host for `epik-core`: one window, one session, no protocol knowledge.
//!
//! Everything about driving Claude Code lives in `epik-core`. This crate is the
//! adapter between that and a window — managed state ([`state::AppState`]),
//! commands the frontend calls ([`commands`]), and the process lifecycle that
//! guarantees no engine outlives the window that started it.

pub mod commands;
pub mod pump;
pub mod state;

use std::sync::Arc;

use tauri::{RunEvent, WindowEvent};

use crate::state::{AppState, SharedState};

/// Build and run the application.
pub fn run() {
    let state: SharedState = Arc::new(AppState::new());
    let app = tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::engine_report,
            commands::doctor,
            commands::start_session,
            commands::send_user,
            commands::respond_permission,
            commands::add_session_allow_rule,
            commands::interrupt,
            commands::end_session,
            commands::session_running,
            commands::session_policy,
            commands::session_stderr,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Epik application");

    // Ending the session is the last thing the process does, and it has to
    // finish: `end_session` closes stdin, waits out the grace period, and kills
    // the engine if it must. Blocking the exit handler is the point — a quit
    // that returns early is exactly how an orphaned `claude` outlives its
    // window. `Destroyed` covers closing the window, `Exit` covers quit and
    // anything else that ends the loop; `end` is idempotent, so both firing is
    // harmless.
    app.run(move |_app, event| match event {
        RunEvent::WindowEvent {
            event: WindowEvent::Destroyed,
            ..
        }
        | RunEvent::Exit => {
            tauri::async_runtime::block_on(state.end());
        }
        _ => {}
    });
}
