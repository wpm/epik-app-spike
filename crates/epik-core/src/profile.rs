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
//! rather than nearly true. Resolving a profile into a `SessionConfig` touches the
//! filesystem, so that part is behind the `host` feature; the name is not.

use serde::{Deserialize, Serialize};

#[cfg(feature = "host")]
use std::path::PathBuf;

#[cfg(feature = "host")]
use crate::event::SessionConfig;
#[cfg(feature = "host")]
use crate::permission::{PermissionPolicy, PermissionRule};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Epik: the persona applied, EpikMCP attached, settings isolated, read-only
    /// tools pre-allowed and mutating ones gated. The default, because this app is
    /// Epik's interface — a plain chat window is the special case.
    #[default]
    Epik,
    /// Plain chat against the CLI's defaults, with every permission ask surfaced.
    /// The profile to reach for when the question is what the engine will try to
    /// do, with nothing pre-decided on its behalf.
    Plain,
}

/// EpikMCP tools that only read. Pre-allowed, because an agent monitoring a build
/// polls these every few seconds and a prompt for each would make the profile
/// unusable — the user would learn to click Allow without reading, which is worse
/// than not asking.
#[cfg(feature = "host")]
pub const EPIK_READ_ONLY_TOOLS: &[&str] = &[
    "issue_list",
    "issue_get",
    "issue_list_relationships",
    "run_list",
    "run_get",
    "run_logs",
    "feature_status",
    "repo_get",
    "repo_default_branch",
    "pr_list",
    "pr_get",
    "project_list_items",
];

/// Built-in tools that only read the local checkout. Pre-allowed on the same
/// reasoning. `Bash` is deliberately absent: it can do anything, so it is gated
/// however innocent the command looks.
#[cfg(feature = "host")]
pub const READ_ONLY_BUILTINS: &[&str] = &["Read", "Glob", "Grep", "NotebookRead", "TodoWrite"];

/// Where the Epik checkout is, and what could be found in it.
///
/// The packaged app would carry the persona and ship EpikMCP itself; 0.2.0 finds a
/// local checkout instead, which is what the spike's M3 demo did. A profile with
/// neither still starts — an app that refused to open because an unrelated
/// repository is missing would be worse than one that says what it could not find.
#[cfg(feature = "host")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpikLayout {
    /// Root of the Epik checkout.
    pub root: Option<PathBuf>,
    /// `plugin/skills/summon/persona.md`, if present.
    pub persona: Option<PathBuf>,
    /// The `mcp` directory EpikMCP is run from, if present.
    pub mcp_project: Option<PathBuf>,
}

#[cfg(feature = "host")]
impl EpikLayout {
    /// Candidate checkout locations, in order: `$EPIK_CHECKOUT` first, because an
    /// explicit answer beats a guess.
    pub fn candidates() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(explicit) = std::env::var_os("EPIK_CHECKOUT") {
            roots.push(PathBuf::from(explicit));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.push(home.join("Projects/Epik/Epik"));
            roots.push(home.join("Epik/Epik"));
            roots.push(home.join("Epik"));
        }
        roots
    }

    pub fn discover() -> Self {
        Self::candidates()
            .into_iter()
            .map(Self::at)
            .find(|layout| layout.root.is_some())
            .unwrap_or_default()
    }

    /// What `root` contains, if it looks like an Epik checkout at all.
    pub fn at(root: PathBuf) -> Self {
        let persona = root.join("plugin/skills/summon/persona.md");
        let mcp_project = root.join("mcp");
        // "Is this an Epik checkout" is decided by finding something in it we
        // need, not by the directory existing — an empty directory of the right
        // name should not shadow a real checkout later in the list.
        let has_persona = persona.is_file();
        let has_mcp = mcp_project.is_dir();
        if !has_persona && !has_mcp {
            return Self::default();
        }
        Self {
            root: Some(root),
            persona: has_persona.then_some(persona),
            mcp_project: has_mcp.then_some(mcp_project),
        }
    }

    /// The `--mcp-config` value attaching EpikMCP, if it can be attached.
    pub fn mcp_config_json(&self) -> Option<String> {
        let project = self.mcp_project.as_ref()?;
        Some(
            serde_json::json!({
                "mcpServers": {
                    "EpikMCP": {
                        "command": "uv",
                        "args": ["run", "--project", project.to_string_lossy(), "epik-mcp"],
                    }
                }
            })
            .to_string(),
        )
    }

    /// The persona text, if it could be read.
    pub fn persona_text(&self) -> Option<String> {
        std::fs::read_to_string(self.persona.as_ref()?).ok()
    }

    /// What is missing, phrased for a UI to show. Empty when nothing is.
    pub fn shortfalls(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.root.is_none() {
            missing.push(
                "No Epik checkout found — set EPIK_CHECKOUT to one. Running without \
                 the Epik persona and without EpikMCP."
                    .to_owned(),
            );
            return missing;
        }
        if self.persona.is_none() {
            missing.push("Epik persona not found in the checkout.".to_owned());
        }
        if self.mcp_project.is_none() {
            missing.push("EpikMCP not found in the checkout.".to_owned());
        }
        missing
    }
}

impl Profile {
    /// Human-readable name, for the session panel.
    pub fn label(self) -> &'static str {
        match self {
            Self::Epik => "Epik",
            Self::Plain => "Plain",
        }
    }

    /// The session this profile describes.
    ///
    /// `claude_bin` is left at its default because the host overrides it with the
    /// binary the doctor resolved — a session must run the engine the report is
    /// about, not whatever a later `PATH` search turns up.
    #[cfg(feature = "host")]
    pub fn config(self) -> SessionConfig {
        match self {
            Self::Plain => SessionConfig {
                include_partial: true,
                permission_policy: PermissionPolicy::ask(),
                ..SessionConfig::default()
            },
            Self::Epik => Self::epik_config(&EpikLayout::discover()),
        }
    }

    /// The Epik profile against an explicit layout. Separate from [`Profile::config`]
    /// so its shape can be tested without a checkout being present.
    #[cfg(feature = "host")]
    pub fn epik_config(layout: &EpikLayout) -> SessionConfig {
        SessionConfig {
            model: Some("claude-sonnet-5".to_owned()),
            append_system_prompt: layout.persona_text(),
            mcp_config: layout.mcp_config_json(),
            // Settings isolation. An empty settings object replaces the user's
            // project and user settings for this session, so nothing they have
            // allowed elsewhere silently pre-approves a tool this profile means to
            // gate: the app's PermissionPolicy becomes the only policy. Auth is
            // unaffected — it is not part of settings — so the session still
            // inherits whatever the `claude` CLI is logged in as.
            settings_json: Some("{}".to_owned()),
            permission_mode: Some("default".to_owned()),
            include_partial: true,
            permission_policy: Self::epik_policy(),
            // --strict-mcp-config: use only the MCP servers named above, ignoring
            // any the user has configured. A profile that inherited unknown MCP
            // servers would be gating a tool list it cannot predict.
            extra_args: vec!["--strict-mcp-config".to_owned()],
            ..SessionConfig::default()
        }
    }

    /// Read-only tools allowed, everything else asked about.
    ///
    /// No wildcard deny at the end: an unmatched tool falls through to `Ask`, which
    /// is both the safe default and the useful one — a tool nobody thought about
    /// should raise a card, not fail silently.
    #[cfg(feature = "host")]
    pub fn epik_policy() -> PermissionPolicy {
        let mut policy = PermissionPolicy::ask();
        for tool in EPIK_READ_ONLY_TOOLS {
            policy.push(PermissionRule::allow(format!("mcp__EpikMCP__{tool}")));
        }
        for tool in READ_ONLY_BUILTINS {
            policy.push(PermissionRule::allow(*tool));
        }
        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trips_through_json() {
        // The frontend names a profile across IPC, so the name is a wire value.
        for (profile, json) in [(Profile::Epik, "\"epik\""), (Profile::Plain, "\"plain\"")] {
            assert_eq!(serde_json::to_string(&profile).unwrap(), json);
            assert_eq!(serde_json::from_str::<Profile>(json).unwrap(), profile);
        }
    }

    #[test]
    fn epik_is_the_default_profile() {
        // This app is Epik's interface; a plain chat window is the special case.
        assert_eq!(Profile::default(), Profile::Epik);
    }

    #[cfg(feature = "host")]
    mod host {
        use super::*;
        use crate::permission::PermissionAction;

        #[test]
        fn plain_surfaces_every_ask() {
            let config = Profile::Plain.config();
            assert!(
                config.permission_policy.is_empty(),
                "an empty policy is what makes every ask surface"
            );
            assert!(config.include_partial);
        }

        #[test]
        fn epik_pre_allows_read_only_tools() {
            let policy = Profile::epik_policy();
            for tool in EPIK_READ_ONLY_TOOLS {
                let name = format!("mcp__EpikMCP__{tool}");
                assert_eq!(
                    policy.decide(&name),
                    PermissionAction::Allow,
                    "{name} should not raise a card"
                );
            }
            for tool in READ_ONLY_BUILTINS {
                assert_eq!(policy.decide(tool), PermissionAction::Allow, "{tool}");
            }
        }

        #[test]
        fn epik_gates_mutating_tools() {
            let policy = Profile::epik_policy();
            // The ones the FINDINGS demo exercises, plus the obvious hazards.
            for tool in [
                "mcp__EpikMCP__issue_create",
                "mcp__EpikMCP__feature_launch",
                "mcp__EpikMCP__issue_update",
                "mcp__EpikMCP__pr_create",
                "Bash",
                "Write",
                "Edit",
                "WebFetch",
            ] {
                assert_eq!(
                    policy.decide(tool),
                    PermissionAction::Ask,
                    "{tool} must be gated"
                );
            }
        }

        #[test]
        fn bash_is_gated_even_though_it_is_a_builtin() {
            // Worth its own test: Bash is the tool most tempting to pre-allow and
            // the one where doing so gives away everything.
            assert!(!READ_ONLY_BUILTINS.contains(&"Bash"));
            assert_eq!(Profile::epik_policy().decide("Bash"), PermissionAction::Ask);
        }

        #[test]
        fn an_unknown_tool_is_asked_about_rather_than_ignored() {
            assert_eq!(
                Profile::epik_policy().decide("mcp__SomethingNew__do_a_thing"),
                PermissionAction::Ask
            );
        }

        #[test]
        fn epik_isolates_settings_and_mcp_config() {
            let config = Profile::epik_config(&EpikLayout::default());
            assert_eq!(
                config.settings_json.as_deref(),
                Some("{}"),
                "an empty settings object is what isolates the session"
            );
            assert!(
                config
                    .extra_args
                    .contains(&"--strict-mcp-config".to_owned()),
                "the profile must not inherit unknown MCP servers"
            );
            assert_eq!(config.model.as_deref(), Some("claude-sonnet-5"));
        }

        #[test]
        fn epik_starts_without_a_checkout_and_says_what_is_missing() {
            let layout = EpikLayout::default();
            let config = Profile::epik_config(&layout);
            // Degraded, not broken: no persona, no MCP, but a usable session.
            assert!(config.append_system_prompt.is_none());
            assert!(config.mcp_config.is_none());
            assert!(!config.permission_policy.is_empty());

            let shortfalls = layout.shortfalls();
            assert_eq!(shortfalls.len(), 1);
            assert!(shortfalls[0].contains("EPIK_CHECKOUT"), "{shortfalls:?}");
        }

        #[test]
        fn a_directory_that_is_not_a_checkout_is_not_mistaken_for_one() {
            let empty = std::env::temp_dir().join("epik-not-a-checkout");
            std::fs::create_dir_all(&empty).unwrap();
            assert_eq!(EpikLayout::at(empty), EpikLayout::default());
        }

        #[test]
        fn a_checkout_is_recognised_by_its_contents() {
            let root = std::env::temp_dir().join("epik-fake-checkout");
            let persona_dir = root.join("plugin/skills/summon");
            std::fs::create_dir_all(&persona_dir).unwrap();
            std::fs::create_dir_all(root.join("mcp")).unwrap();
            std::fs::write(persona_dir.join("persona.md"), "You are Epik.\n").unwrap();

            let layout = EpikLayout::at(root.clone());
            assert_eq!(layout.root.as_ref(), Some(&root));
            assert_eq!(layout.persona_text().as_deref(), Some("You are Epik.\n"));
            assert!(layout.shortfalls().is_empty());

            let mcp = layout.mcp_config_json().expect("EpikMCP config");
            assert!(mcp.contains("EpikMCP"), "{mcp}");
            assert!(mcp.contains("epik-mcp"), "{mcp}");
            assert!(mcp.contains("--project"), "{mcp}");

            let config = Profile::epik_config(&layout);
            assert_eq!(
                config.append_system_prompt.as_deref(),
                Some("You are Epik.\n"),
                "the persona has to reach the session as the system prompt"
            );
            assert!(config.mcp_config.is_some());

            std::fs::remove_dir_all(&root).ok();
        }

        #[test]
        fn an_explicit_checkout_is_preferred_over_a_guess() {
            let candidates = EpikLayout::candidates();
            // Only assert the ordering property when the variable is set, so the
            // test does not depend on the environment it runs in.
            if let Some(explicit) = std::env::var_os("EPIK_CHECKOUT") {
                assert_eq!(candidates.first(), Some(&PathBuf::from(explicit)));
            } else {
                assert!(
                    !candidates.is_empty(),
                    "there should always be somewhere to look"
                );
            }
        }
    }
}
