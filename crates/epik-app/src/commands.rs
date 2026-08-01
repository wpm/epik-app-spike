//! Commands the frontend calls. The whole of the frontend's authority over a
//! session is this list.
//!
//! Every argument and return value is a type defined in `epik-core` — including
//! `Profile`, which lives there rather than here precisely so both sides of the
//! boundary can name it. Nothing the frontend sends or receives is declared in
//! this crate, so there is nothing for it to mirror. Errors come back as `String`
//! because that is what crosses IPC; the text is the message `anyhow` already
//! produced, so nothing is lost in the conversion.

use epik_core::{EngineReport, PermissionDecision, PermissionRule, Profile, SessionEvent};

use crate::pump;
use crate::state::SharedState;

/// The engine report resolved at startup — what the window's first render uses to
/// decide between the interface and the doctor screen.
#[tauri::command]
pub async fn engine_report(state: tauri::State<'_, SharedState>) -> Result<EngineReport, String> {
    Ok(state.engine().await)
}

/// Re-resolve the engine and return the fresh report. The doctor screen's retry:
/// installing Claude Code with the window open should not require a restart.
#[tauri::command]
pub async fn doctor(state: tauri::State<'_, SharedState>) -> Result<EngineReport, String> {
    Ok(state.refresh_engine().await)
}

/// Start a session for `profile`, forwarding its events to `channel`.
///
/// Refuses when the engine is unusable rather than spawning something that
/// cannot work: the failure belongs in the doctor report, not in a session that
/// dies a second later with a confusing message.
#[tauri::command]
pub async fn start_session(
    profile: Profile,
    channel: tauri::ipc::Channel<SessionEvent>,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let engine = state.engine().await;
    if !engine.supported() {
        return Err(engine.headline());
    }
    // Returns nothing on purpose. Everything a caller might want back — model,
    // CLI version, session id — arrives on the channel as the `Init` event, and
    // the engine path is already in the report the frontend fetched. A richer
    // return value would be a struct the frontend has to define a twin of, which
    // is the one thing this layer is meant to avoid.
    state
        .start(profile.config(), move |events| pump::pump(events, channel))
        .await
        .map(|_handle| ())
        .map_err(|e| format!("{e:#}"))
}

/// Send a user turn.
#[tauri::command]
pub async fn send_user(text: String, state: tauri::State<'_, SharedState>) -> Result<(), String> {
    with_session(
        &state,
        |handle| async move { handle.send_user(&text).await },
    )
    .await
}

/// Answer a permission ask that surfaced as `SessionEvent::PermissionRequest`.
#[tauri::command]
pub async fn respond_permission(
    request_id: String,
    decision: PermissionDecision,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    with_session(&state, |handle| async move {
        handle.respond_permission(&request_id, &decision).await
    })
    .await
}

/// Add a rule to the live session's policy — "always allow this tool for this
/// session". Takes effect for every ask that arrives after it lands.
#[tauri::command]
pub async fn add_session_allow_rule(
    rule: PermissionRule,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let handle = state.handle().await.ok_or(NO_SESSION)?;
    handle.add_rule(rule);
    Ok(())
}

/// Interrupt the in-flight turn.
#[tauri::command]
pub async fn interrupt(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    with_session(&state, |handle| async move {
        handle.interrupt().await.map(|_request_id| ())
    })
    .await
}

/// End the session: stdin closed, engine reaped. Returns once no engine process
/// remains, so a frontend that awaits it can be sure.
#[tauri::command]
pub async fn end_session(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.end().await;
    Ok(())
}

/// Whether a session is live. Cheap enough to poll, and it lets a reloaded
/// frontend discover a session it did not start.
#[tauri::command]
pub async fn session_running(state: tauri::State<'_, SharedState>) -> Result<bool, String> {
    Ok(state.is_running().await)
}

/// The live session's current permission policy — what the session panel shows
/// when asked which tools are pre-decided.
#[tauri::command]
pub async fn session_policy(
    state: tauri::State<'_, SharedState>,
) -> Result<Option<epik_core::PermissionPolicy>, String> {
    Ok(state.handle().await.map(|handle| handle.policy()))
}

/// The engine's recent stderr. What a post-mortem asks for after a session
/// closes with a nonzero exit code.
#[tauri::command]
pub async fn session_stderr(state: tauri::State<'_, SharedState>) -> Result<Vec<String>, String> {
    Ok(state
        .handle()
        .await
        .map(|handle| handle.recent_stderr())
        .unwrap_or_default())
}

const NO_SESSION: &str = "no session is running";

/// Run `f` against the live session, or fail with one consistent message. Every
/// command that needs a session needs this same guard, and a frontend that races
/// a command against `end_session` should get a sentence rather than a panic.
async fn with_session<F, Fut>(state: &SharedState, f: F) -> Result<(), String>
where
    F: FnOnce(epik_core::SessionHandle) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let handle = state.handle().await.ok_or(NO_SESSION)?;
    f(handle).await.map_err(|e| format!("{e:#}"))
}
