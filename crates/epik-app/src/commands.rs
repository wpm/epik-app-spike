//! Commands the frontend calls.
//!
//! Every one of these returns a type defined in `epik-core`, so the frontend
//! deserializes the core's own types rather than a mirrored set. Errors come back
//! as `String` because that is what crosses IPC; the message is the one
//! `anyhow` already produced, so nothing is lost turning it into text.

use epik_core::EngineReport;

use crate::state::SharedState;

/// The engine report resolved at startup. What the window's first render uses to
/// decide between the chat interface and the doctor screen.
#[tauri::command]
pub async fn engine_report(state: tauri::State<'_, SharedState>) -> Result<EngineReport, String> {
    Ok(state.engine().await)
}

/// Re-resolve the engine and return the fresh report. This is the doctor
/// screen's retry: someone who installs Claude Code while the window is open
/// should not have to restart the app for it to be noticed.
#[tauri::command]
pub async fn doctor(state: tauri::State<'_, SharedState>) -> Result<EngineReport, String> {
    Ok(state.refresh_engine().await)
}
