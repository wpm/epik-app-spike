//! Which tool calls get answered without asking a human.
//!
//! A [`PermissionPolicy`] is an ordered list of rules; the first one whose
//! matcher covers the tool name decides. A tool no rule matches is asked about,
//! which makes the two interesting policies fall out of the same structure
//! rather than being special cases:
//!
//! - **ask about everything** is the empty policy ([`PermissionPolicy::ask`]),
//!   because nothing matches and the default is [`PermissionAction::Ask`];
//! - **allow everything** is one wildcard allow rule
//!   ([`PermissionPolicy::allow_all`]).
//!
//! Order matters, and it is first-match-wins rather than most-specific-wins.
//! That is the property "always allow this tool for this session" needs: a
//! runtime rule is pushed to the front, so it beats whatever the profile
//! already said about that tool without having to find and edit the old rule.

use serde::{Deserialize, Serialize};

/// What to do about a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Answer the CLI's ask with allow, without involving the host.
    Allow,
    /// Answer with deny, without involving the host.
    Deny,
    /// Surface the ask to the host and let a human decide.
    Ask,
}

/// Which tool names a rule covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolMatcher {
    /// Every tool.
    Any,
    /// One tool, by exact name — `"Read"`.
    Exact(String),
    /// Every tool whose name starts with this — `"mcp__epik__"` covers an
    /// entire MCP server, which is how MCP tools are grouped in practice.
    Prefix(String),
}

impl ToolMatcher {
    pub fn matches(&self, tool_name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(name) => name == tool_name,
            Self::Prefix(prefix) => tool_name.starts_with(prefix.as_str()),
        }
    }
}

/// One entry in a [`PermissionPolicy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub matcher: ToolMatcher,
    pub action: PermissionAction,
    /// What the model is told when this rule denies. Unused for the other two
    /// actions. A denial the model cannot read is a denial it will retry, so
    /// this is worth setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_message: Option<String>,
}

impl PermissionRule {
    pub fn new(matcher: ToolMatcher, action: PermissionAction) -> Self {
        Self {
            matcher,
            action,
            deny_message: None,
        }
    }

    /// Allow every tool. The whole of [`PermissionPolicy::allow_all`].
    pub fn allow_any() -> Self {
        Self::new(ToolMatcher::Any, PermissionAction::Allow)
    }

    pub fn allow(tool_name: impl Into<String>) -> Self {
        Self::new(
            ToolMatcher::Exact(tool_name.into()),
            PermissionAction::Allow,
        )
    }

    pub fn allow_prefix(prefix: impl Into<String>) -> Self {
        Self::new(ToolMatcher::Prefix(prefix.into()), PermissionAction::Allow)
    }

    pub fn deny(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            matcher: ToolMatcher::Exact(tool_name.into()),
            action: PermissionAction::Deny,
            deny_message: Some(message.into()),
        }
    }

    pub fn deny_prefix(prefix: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            matcher: ToolMatcher::Prefix(prefix.into()),
            action: PermissionAction::Deny,
            deny_message: Some(message.into()),
        }
    }

    /// Force a human decision for this tool even if a later rule would allow
    /// it. Useful in front of a broad prefix allow.
    pub fn ask(tool_name: impl Into<String>) -> Self {
        Self::new(ToolMatcher::Exact(tool_name.into()), PermissionAction::Ask)
    }

    pub fn ask_prefix(prefix: impl Into<String>) -> Self {
        Self::new(ToolMatcher::Prefix(prefix.into()), PermissionAction::Ask)
    }
}

/// An ordered list of rules. First match wins; no match means
/// [`PermissionAction::Ask`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionPolicy {
    rules: Vec<PermissionRule>,
}

impl PermissionPolicy {
    /// Ask about everything: the empty policy.
    pub fn ask() -> Self {
        Self::default()
    }

    /// Allow everything: one wildcard allow rule.
    pub fn allow_all() -> Self {
        Self::from_rules([PermissionRule::allow_any()])
    }

    pub fn from_rules(rules: impl IntoIterator<Item = PermissionRule>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
        }
    }

    /// Builder form: append a rule at the end, where it is consulted last.
    #[must_use]
    pub fn with(mut self, rule: PermissionRule) -> Self {
        self.push(rule);
        self
    }

    /// Append a rule at the end of the list.
    pub fn push(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    /// Insert a rule at the front, where it wins over everything already
    /// there. This is what "always allow this tool for this session" does — it
    /// has to override the profile's existing verdict on that tool, and
    /// first-match-wins means prepending is the whole implementation.
    pub fn prepend(&mut self, rule: PermissionRule) {
        self.rules.insert(0, rule);
    }

    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The first rule covering `tool_name`, if any.
    pub fn matching_rule(&self, tool_name: &str) -> Option<&PermissionRule> {
        self.rules.iter().find(|r| r.matcher.matches(tool_name))
    }

    /// What to do about `tool_name`.
    pub fn decide(&self, tool_name: &str) -> PermissionAction {
        self.matching_rule(tool_name)
            .map_or(PermissionAction::Ask, |rule| rule.action)
    }

    /// The message to send with a policy denial of `tool_name`.
    pub fn deny_message(&self, tool_name: &str) -> String {
        self.matching_rule(tool_name)
            .and_then(|rule| rule.deny_message.clone())
            .unwrap_or_else(|| format!("`{tool_name}` is not permitted by this session's policy."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_asks_about_everything() {
        let policy = PermissionPolicy::ask();
        assert!(policy.is_empty());
        assert_eq!(policy.decide("Read"), PermissionAction::Ask);
        assert_eq!(policy.decide("Bash"), PermissionAction::Ask);
        assert_eq!(policy.decide(""), PermissionAction::Ask);
    }

    #[test]
    fn allow_all_is_one_wildcard_rule() {
        let policy = PermissionPolicy::allow_all();
        assert_eq!(policy.rules().len(), 1);
        assert_eq!(policy.rules()[0].matcher, ToolMatcher::Any);
        assert_eq!(policy.decide("Read"), PermissionAction::Allow);
        assert_eq!(
            policy.decide("mcp__EpikMCP__issue_create"),
            PermissionAction::Allow
        );
    }

    #[test]
    fn first_matching_rule_wins() {
        // Ask about the one mutating tool, allow the rest of the server. The
        // ask has to come first, or the prefix allow would swallow it.
        let policy = PermissionPolicy::from_rules([
            PermissionRule::ask("mcp__EpikMCP__issue_create"),
            PermissionRule::allow_prefix("mcp__EpikMCP__"),
        ]);
        assert_eq!(
            policy.decide("mcp__EpikMCP__issue_create"),
            PermissionAction::Ask
        );
        assert_eq!(
            policy.decide("mcp__EpikMCP__issue_list"),
            PermissionAction::Allow
        );

        // Reversed, the broad rule shadows the specific one — order is the
        // only thing that decided this, which is the property being pinned.
        let shadowed = PermissionPolicy::from_rules([
            PermissionRule::allow_prefix("mcp__EpikMCP__"),
            PermissionRule::ask("mcp__EpikMCP__issue_create"),
        ]);
        assert_eq!(
            shadowed.decide("mcp__EpikMCP__issue_create"),
            PermissionAction::Allow
        );
    }

    #[test]
    fn exact_rules_do_not_match_by_prefix() {
        let policy = PermissionPolicy::from_rules([PermissionRule::allow("Read")]);
        assert_eq!(policy.decide("Read"), PermissionAction::Allow);
        assert_eq!(policy.decide("ReadFile"), PermissionAction::Ask);
        assert_eq!(policy.decide("Rea"), PermissionAction::Ask);
    }

    #[test]
    fn prefix_rules_cover_a_whole_mcp_server() {
        let policy = PermissionPolicy::from_rules([PermissionRule::allow_prefix("mcp__epik__")]);
        assert_eq!(
            policy.decide("mcp__epik__issue_list"),
            PermissionAction::Allow
        );
        assert_eq!(policy.decide("mcp__epik__run_get"), PermissionAction::Allow);
        assert_eq!(
            policy.decide("mcp__other__issue_list"),
            PermissionAction::Ask
        );
        // The bare prefix is itself covered; a prefix is not "strictly longer".
        assert_eq!(policy.decide("mcp__epik__"), PermissionAction::Allow);
    }

    #[test]
    fn prepended_runtime_rule_overrides_an_existing_verdict() {
        // The session started out asking about Bash.
        let mut policy = PermissionPolicy::from_rules([
            PermissionRule::ask("Bash"),
            PermissionRule::allow_prefix("mcp__epik__"),
        ]);
        assert_eq!(policy.decide("Bash"), PermissionAction::Ask);

        // "Always allow this tool for this session" — prepending is enough,
        // the stale ask rule does not have to be found and removed.
        policy.prepend(PermissionRule::allow("Bash"));
        assert_eq!(policy.decide("Bash"), PermissionAction::Allow);
        assert_eq!(policy.rules().len(), 3);
        // Unrelated tools are untouched.
        assert_eq!(policy.decide("Write"), PermissionAction::Ask);
        assert_eq!(policy.decide("mcp__epik__run_get"), PermissionAction::Allow);
    }

    #[test]
    fn appended_rule_loses_to_an_earlier_match() {
        let mut policy = PermissionPolicy::from_rules([PermissionRule::ask("Bash")]);
        policy.push(PermissionRule::allow("Bash"));
        assert_eq!(policy.decide("Bash"), PermissionAction::Ask);
    }

    #[test]
    fn deny_message_falls_back_to_a_readable_default() {
        let policy = PermissionPolicy::from_rules([
            PermissionRule::deny("Bash", "No shell in this session."),
            PermissionRule::new(ToolMatcher::Exact("Write".into()), PermissionAction::Deny),
        ]);
        assert_eq!(policy.deny_message("Bash"), "No shell in this session.");
        assert!(policy.deny_message("Write").contains("Write"));
    }

    #[test]
    fn policy_round_trips_through_json() {
        let policy = PermissionPolicy::allow_all()
            .with(PermissionRule::allow_prefix("mcp__epik__"))
            .with(PermissionRule::deny("Bash", "nope"));
        let json = serde_json::to_string(&policy).unwrap();
        // `transparent` means a policy is just its ordered list on the wire.
        assert!(json.starts_with('['), "expected a JSON array, got {json}");
        assert_eq!(
            serde_json::from_str::<PermissionPolicy>(&json).unwrap(),
            policy
        );
    }
}
