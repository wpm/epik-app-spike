//! The app's managed state: one engine report, at most one live session.
//!
//! "At most one" is enforced here rather than trusted of the frontend. Starting
//! a session while one is running ends the old one first, so there is no path
//! that leaves two engines alive and only one of them reachable — the shape that
//! produces an orphan nobody can stop.

use std::sync::Arc;

use epik_core::{EngineReport, Session, SessionConfig, SessionEvent, SessionHandle, doctor};
use tokio::sync::{Mutex, mpsc};

/// A session and the task draining its events.
struct Active {
    handle: SessionHandle,
    /// The pump forwarding events to the frontend. Aborted when the session
    /// ends so a dead session cannot keep emitting.
    pump: tokio::task::JoinHandle<()>,
}

pub struct AppState {
    /// Resolved at startup and cached. The window shows chat or the doctor
    /// report based on this, and re-resolving per render would spawn a process
    /// per frame; `refresh_engine` exists for the "I just installed it" retry.
    engine: Mutex<EngineReport>,
    active: Mutex<Option<Active>>,
}

impl AppState {
    /// Resolve the engine. Called once during Tauri setup, before any window
    /// content decides what to render.
    pub fn new() -> Self {
        Self {
            engine: Mutex::new(doctor::inspect()),
            active: Mutex::new(None),
        }
    }

    /// The cached engine report.
    pub async fn engine(&self) -> EngineReport {
        self.engine.lock().await.clone()
    }

    /// Re-resolve the engine and cache the result. This is what the doctor
    /// screen's retry calls after the user installs Claude Code, so the app does
    /// not have to be restarted to notice.
    pub async fn refresh_engine(&self) -> EngineReport {
        let report = doctor::inspect();
        *self.engine.lock().await = report.clone();
        report
    }

    /// Whether a session is live.
    pub async fn is_running(&self) -> bool {
        self.active.lock().await.is_some()
    }

    /// The live session's handle, if there is one.
    pub async fn handle(&self) -> Option<SessionHandle> {
        self.active
            .lock()
            .await
            .as_ref()
            .map(|active| active.handle.clone())
    }

    /// Start a session, replacing any existing one. `pump` is handed the event
    /// receiver and runs until the stream ends; the caller decides where the
    /// events go, which is what keeps this type free of Tauri.
    pub async fn start<F, Fut>(
        &self,
        config: SessionConfig,
        pump: F,
    ) -> anyhow::Result<SessionHandle>
    where
        F: FnOnce(mpsc::Receiver<SessionEvent>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        // Hold the lock across the whole swap: two concurrent starts must not
        // both spawn an engine and race to record it.
        let mut guard = self.active.lock().await;
        if let Some(previous) = guard.take() {
            previous.handle.end_session().await;
            previous.pump.abort();
        }

        // The engine the doctor resolved, not whatever `claude` a PATH search
        // finds later. This is the whole point of resolving at startup: the
        // session must run the binary the report is about.
        let mut config = config;
        if let Some(path) = self.engine.lock().await.path.clone() {
            config.claude_bin = path.to_string_lossy().into_owned();
        }

        let session = Session::spawn(config).await?;
        let handle = session.handle();
        *guard = Some(Active {
            handle: handle.clone(),
            pump: tokio::spawn(pump(session.events)),
        });
        Ok(handle)
    }

    /// End the session cleanly: stdin closed, engine reaped. Returns once no
    /// engine process remains. Safe to call when nothing is running, and safe to
    /// call twice — a window close followed by an app quit does exactly that.
    pub async fn end(&self) {
        let mut guard = self.active.lock().await;
        if let Some(active) = guard.take() {
            active.handle.end_session().await;
            active.pump.abort();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared with Tauri's managed-state registry.
pub type SharedState = Arc<AppState>;
