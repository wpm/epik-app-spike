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
pub mod event;
pub mod permission;
pub mod profile;
pub mod protocol;
#[cfg(feature = "host")]
pub mod session;

// The API a host is expected to need, flattened to the crate root: the surface
// worth keeping stable, in one place, so `use epik_core::...` is the whole
// import story for an ordinary embedder.
pub use doctor::{EngineReport, EngineSearch, EngineStatus, MIN_SUPPORTED, parse_version};
#[cfg(feature = "host")]
pub use doctor::{inspect, inspect_with, run_doctor};
pub use event::{SessionConfig, SessionEvent};
pub use permission::{PermissionAction, PermissionPolicy, PermissionRule, ToolMatcher};
pub use profile::Profile;
#[cfg(feature = "host")]
pub use profile::{EPIK_READ_ONLY_TOOLS, EpikLayout, READ_ONLY_BUILTINS};
pub use protocol::{
    ApiMessage, AssistantEnvelope, CanUseToolRequest, ContentBlock, ControlRequest,
    ControlRequestEnvelope, ControlResponseEnvelope, ControlResponsePayload, PermissionDecision,
    ResultMessage, StreamEvent, StreamMessage, SystemMessage, UserEnvelope,
};
#[cfg(feature = "host")]
pub use session::{SHUTDOWN_GRACE, STDERR_BUFFER_LINES, Session, SessionHandle};
