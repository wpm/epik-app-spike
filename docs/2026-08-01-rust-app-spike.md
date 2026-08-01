# ADR: Epik gets its own app

- **Status:** Draft — pending Bill's review of the epik-app spike (FINDINGS.md, GREEN)
- **Date:** 2026-08-01
- **Source:** Feasibility spike (epik-app-spike repo), August 2026

## Context

Epik's interface today is an assembly: Claude Desktop hosting EpikMCP via MCPB for
chat, plus a terminal for anything the Desktop surface can't do. Two standing
decisions shaped that assembly: ADR-0001 decision 8 ("GitHub is the database… no
custom dashboard") and 2026-07-28-presence-not-a-connector ("there is no chat UI and
no hosted service"). Both were written when the alternative to "stock surfaces" was
building and hosting infrastructure.

The epik-app spike tested a third option that existed in neither ADR's option space:
a single locally-running Rust binary that owns the interface while Claude Code
remains the engine — the same architecture as the official Agent SDKs, which are
subprocess drivers for the `claude` CLI, not reimplementations of it.

The spike (two-day budget; finished in one session) proved every layer live:
typed stream-json parsing; the control protocol — permission asks rendered in the
app's UI and answered from Rust, interrupts, multi-turn sessions; a minimal ratatui
chat; and the success demo — the app summoned Epik (persona loaded), attached
EpikMCP, created a feature issue, dispatched `feature_launch`, and monitored the
GitHub Actions build from inside the app. Distribution collapses to one 1.9MB binary
that locates and version-checks the engine, with a verified path to fetching its own
engine (the official installer is a thin shell over a versioned, checksummed release
endpoint — the uv-manages-Python pattern applies directly).

## Decision

Adopt the app architecture as Epik's interface direction:

1. **Epik ships a native app** (Rust; TUI first, richer UI later) that is the way a
   user talks to Epik. It supersedes the Claude Desktop + MCPB assembly as the
   primary surface. MCPB remains a supported connector for users already living in
   Claude Desktop, but new-user instructions lead with the app.
2. **Claude Code stays the engine.** The app spawns `claude -p` over stream-json and
   implements the client side of the control protocol. No agent loop, tool
   execution, or MCP handling is reimplemented. The protocol types are vendored and
   tracked against the official SDK's published type spec (`sdk.d.ts`), with
   unknown-tolerant parsing and a handshake canary test.
3. **"GitHub is the database" stands.** The app holds no state of its own; every
   pane it renders is a view of GitHub (issues, runs, PRs) through EpikMCP. What
   this ADR supersedes is only the "no chat UI" clause: Epik now owns a chat
   surface. "No owned infrastructure" also stands — the app is local software; there
   is still no hosted service.
4. **Auth stays inherited.** The app never touches credentials: Anthropic auth comes
   from the `claude` CLI's own login/key, GitHub auth from ambient `gh`.

## Consequences

- Epik owns UI code for the first time (~350 lines of TUI in the spike; a real UI
  pass is V-next work: dependency-graph pane fed from GitHub state, build-status
  colors per the V2 dashboard sketch).
- The Desktop-assembly friction disappears for app users: one binary + one engine
  install, no MCPB, no Desktop dependency; the permission prompts that MCPB
  delegates to Desktop are rendered by Epik itself via `can_use_tool`.
- New maintenance surface: protocol drift against future CLI releases. Mitigated by
  vendored unknown-tolerant types, the runtime `capabilities` handshake, a canary
  test in CI, and the documented `sdk.d.ts` contract to diff against.
- Session-settings policy becomes an explicit design point (what an Epik session
  inherits from `~/.claude`: auth yes; hooks/plugins/allow-rules decided per
  surface, via `--settings` / `--strict-mcp-config`).
- Supersedes: ADR-0001 decision 8's "no custom dashboard" clause and
  presence-not-a-connector's "no chat UI" constraint, in the narrow sense above.
  Their shared premise — GitHub remains the single source of truth — is preserved
  and restated here.
