//! Tauri host for `epik-core`: one window, one session, no protocol knowledge.
//!
//! Everything about driving Claude Code lives in `epik-core`. This crate is the
//! adapter between that and a window — managed state ([`state::AppState`]),
//! commands the frontend calls ([`commands`]), and the process lifecycle that
//! guarantees no engine outlives the window that started it.
//!
//! # The content security policy
//!
//! `tauri.conf.json` sets `app.security.csp`, and the reasoning does not fit in
//! a JSON file, so it lives here.
//!
//! The window renders text an untrusted party wrote. Not the user — the model,
//! quoting a repository, a web page, or a tool result — and it renders inside a
//! WebView wired to this crate's command surface. `epik-ui`'s markdown renderer
//! escapes raw HTML and refuses any link or image destination that is not
//! `http`/`https`, but that is one layer, in one language, on one path into the
//! DOM. The CSP is the layer underneath: even if something did get injected,
//! the document it lands in cannot load or run anything that did not ship in
//! the bundle.
//!
//! Directive by directive:
//!
//! - `default-src 'self'` — the app is a bundle of local assets. Nothing it
//!   displays needs the network, so nothing it displays may reach it.
//! - `script-src 'self' 'wasm-unsafe-eval'` — no remote or inline script.
//!   `'wasm-unsafe-eval'` is what a Leptos frontend needs and all it needs: it
//!   permits `WebAssembly.instantiate`, not `eval`. Trunk's loader is an inline
//!   `<script type="module">`, which works because `tauri-codegen` hashes the
//!   inline scripts of every embedded HTML file and adds the hashes to
//!   `script-src` at runtime — so the loader is allowed by its exact content
//!   and a script with any other content is not.
//! - `style-src 'self'` — Tailwind and the brand tokens are compiled to
//!   stylesheets in the bundle; the frontend has no inline styles, so it does
//!   not need `'unsafe-inline'` and does not get it.
//! - `img-src 'self' data:` — deliberately no `https:`. A remote image is a
//!   request to a host the model chose, made by a client the user did not
//!   inspect: the classic way to turn "render this markdown" into an
//!   exfiltration channel and an IP leak. The markdown renderer will emit an
//!   `<img>` for an `https` source, and this is what stops it loading.
//! - `connect-src 'self' ipc: http://ipc.localhost` — the frontend's own
//!   requests plus Tauri's IPC, which is a custom protocol on Windows and
//!   Android and must be named explicitly there.
//! - `object-src 'none'`, `base-uri 'self'`, `form-action 'none'`,
//!   `frame-ancestors 'none'` — plugins, `<base>` rewriting, form posts, and
//!   embedding are all things this window has no use for, so they are off
//!   rather than merely constrained by `default-src`.
//!
//! That the interface actually renders under it is a property of a packaged
//! app, so it is checked by hand; see `docs/manual-checks.md`.

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
            commands::profile_shortfalls,
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
