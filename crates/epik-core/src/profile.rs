//! Named session configurations.
//!
//! A profile answers "what kind of session is this?" — model, MCP servers,
//! persona, and which tools are pre-decided. A frontend names one instead of
//! assembling a `SessionConfig`, so what a session *is* stays in Rust rather than
//! being reconstructed by whatever calls `start_session`.
//!
//! This lives in the core rather than in the host because both sides of the IPC
//! boundary need the name: the frontend sends it, the host resolves it. Keeping
//! it here is what makes "no mirrored type definitions in the frontend" literal
//! rather than nearly true.

use crate::event::SessionConfig;
use crate::permission::PermissionPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Plain chat against the CLI's defaults, with every permission ask
    /// surfaced to the user. Nothing is pre-allowed, which makes this the
    /// profile to reach for when the question is what the engine will try to do.
    #[default]
    Plain,
}

impl Profile {
    /// The session this profile describes. `claude_bin` is left at its default
    /// because the host overrides it with the binary the doctor resolved — the
    /// session must run the engine the report is about, not whatever a later
    /// `PATH` search turns up.
    pub fn config(self) -> SessionConfig {
        match self {
            Self::Plain => SessionConfig {
                include_partial: true,
                permission_policy: PermissionPolicy::ask(),
                ..SessionConfig::default()
            },
        }
    }

    /// Human-readable name, for the session panel.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_surfaces_every_ask() {
        let config = Profile::Plain.config();
        assert!(
            config.permission_policy.is_empty(),
            "an empty policy is what makes every ask surface"
        );
        assert!(
            config.include_partial,
            "streaming text needs --include-partial-messages"
        );
    }

    #[test]
    fn profile_round_trips_through_json() {
        // The frontend names a profile across IPC, so the name is a wire value.
        assert_eq!(serde_json::to_string(&Profile::Plain).unwrap(), "\"plain\"");
        assert_eq!(
            serde_json::from_str::<Profile>("\"plain\"").unwrap(),
            Profile::Plain
        );
    }
}
