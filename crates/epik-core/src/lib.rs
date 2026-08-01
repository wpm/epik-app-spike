//! `epik-core` — the session layer, with no interface attached.
//!
//! Claude Code is the engine; this crate is everything needed to drive it and
//! nothing about how to display it. A host — the Tauri app, an example binary,
//! a test — spawns a [`Session`], reads [`SessionEvent`]s off one channel in
//! stream order, and writes back through a cloneable [`SessionHandle`].
//!
//! ```no_run
//! # async fn run() -> anyhow::Result<()> {
//! use epik_core::{Session, SessionConfig, SessionEvent};
//!
//! let mut session = Session::spawn(SessionConfig::default()).await?;
//! session.handle().send_user("hello").await?;
//! while let Some(event) = session.next_event().await {
//!     if let SessionEvent::AssistantText(text) = event {
//!         println!("{text}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The three modules are also public, so a host that wants the wire-level view
//! can reach past the re-exports without this crate having to anticipate it.

pub mod doctor;
pub mod permission;
pub mod protocol;
pub mod session;

// The API a host is expected to need, flattened to the crate root: the surface
// worth keeping stable, in one place, so `use epik_core::...` is the whole
// import story for an ordinary embedder.
pub use doctor::{EngineReport, MIN_SUPPORTED, inspect, parse_version, run_doctor};
pub use permission::{PermissionAction, PermissionPolicy, PermissionRule, ToolMatcher};
pub use protocol::{
    ApiMessage, AssistantEnvelope, CanUseToolRequest, ContentBlock, ControlRequest,
    ControlRequestEnvelope, ControlResponseEnvelope, ControlResponsePayload, PermissionDecision,
    ResultMessage, StreamEvent, StreamMessage, SystemMessage, UserEnvelope,
};
pub use session::{STDERR_BUFFER_LINES, Session, SessionConfig, SessionEvent, SessionHandle};
