# FINDINGS — Can a single Rust app be Epik's interface?

## Verdict: GREEN

A locally-running Rust application can serve as Epik's interface, with Claude Code as
the engine, and the spike proves it end to end: M0 through M4 all landed in one
session, none of the kill criteria came close to firing, and the load-bearing joint —
the control protocol (permission asks answered from Rust, interrupts, multi-turn
sessions) — worked on the first live attempt and kept working. The success demo ran:
the app summoned Epik (persona loaded, "Hello, I'm Epik"), attached EpikMCP, created a
feature issue in the test repo, dispatched `feature_launch`, and monitored the real
GitHub Actions build from inside the app. The protocol surface is not prose-documented
but it is *documented* — the official TypeScript SDK ships a ~300KB richly-commented
type specification (`sdk.d.ts`) that describes every message, and the Rust side needs
only a small subset. Total protocol + session layer: ~700 lines of Rust including
tests.

## The architecture, as validated

```
┌─────────────────────────── epik-app (Rust, one binary, 1.9MB) ──┐
│  ratatui TUI  ←→  Session (tokio task, mpsc events)             │
│                        │ spawn, stdin/stdout pipes              │
└────────────────────────┼────────────────────────────────────────┘
                         ▼
  claude -p --input-format stream-json --output-format stream-json
           --verbose --permission-prompt-tool stdio
           [--include-partial-messages --mcp-config … --settings …]
                         │
                         ▼
              EpikMCP (uv run … epik-mcp) → gh → GitHub
```

The premise held: this is architecturally identical to the official SDKs, which are
also just subprocess drivers for the same CLI. Nothing was reimplemented; the harness
(agent loop, tools, MCP, permissions, hooks, session persistence) is all Claude Code.

## Protocol notes — what the stream-json surface actually is

- One JSON object per line on stdout, tagged by `type`: `system` (subtypes `init`,
  `status`, `hook_started`, …), `assistant` / `user` (wrapping complete API messages),
  `stream_event` (raw API streaming deltas, only with `--include-partial-messages`),
  `result` (per-turn: `subtype`, `is_error`, `total_cost_usd`, `num_turns`, usage),
  and the control plane: `control_request`, `control_response`,
  `control_cancel_request`.
- Client → CLI on stdin, same shape: `user` messages, `control_request`
  (`initialize`, `interrupt`, `set_permission_mode`, `set_model`, …),
  `control_response` (answers to `can_use_tool`).
- **Permissions**: with `--permission-prompt-tool stdio` (hidden from `--help`, but
  functional — it's what the official SDK passes), any tool call that the permission
  system would prompt for arrives as a `can_use_tool` control_request carrying
  `tool_name`, full `input`, `tool_use_id`, human-readable `description`,
  `decision_reason_type`, and suggestion metadata. Answer
  `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`; deny
  surfaces to the model as an is_error tool_result with your message and the model
  continues gracefully. Verified live in both directions.
- **Interrupts**: `{"subtype":"interrupt"}` control_request mid-stream → success ack,
  turn truncates with result subtype `error_during_execution`. Verified live (stopped
  a 1-to-300 count at 32).
- **Sessions**: one process = one session; each turn ends in a `result`; the process
  exits cleanly on stdin EOF. Costs arrive per turn — budget telemetry is free.
- **Stability outlook**: the surface is the official SDKs' load-bearing contract, and
  the CLI advertises `capabilities` in its init message for feature detection
  (`interrupt_receipt_v1`, …). The `sdk.d.ts` doc-comments show deliberate
  compatibility discipline (fields documented as "absent on older CLIs", "open set —
  ignore unknown values"). Risk of silent breakage is low; risk of additive drift is
  real but the parser tolerates unknown message types/fields by design.
- **Gotchas found**:
  - `-p --output-format stream-json` requires `--verbose`.
  - User-level config bleeds into embedded sessions: global CLAUDE.md, hooks, and
    plugins all load (a cold turn cached 24.5k tokens of Bill's config; global
    `"Bash"` allow rules suppress permission asks). An embedding app must decide per
    session between inheritance and isolation (`--settings`, `--strict-mcp-config`,
    custom system prompt); the spike demos both modes.
  - Auth is inherited from the CLI (`ANTHROPIC_API_KEY` here; claude.ai login
    otherwise). The app never touches credentials — a feature.

## Crate evaluation

Candidates from crates.io, sources reviewed in-tree:

| crate | version | state | control protocol |
|---|---|---|---|
| `claude-agent-sdk` (Wally869) | 0.1.1 | stale (2025-09-30, pre-2.x CLIs) | yes: `can_use_tool` callback, `--permission-prompt-tool` |
| `claude-code-sdk-rust` (PandelisZ) | 0.4.1 | active (2026-07-21) | yes: full control module — initialize, can_use_tool, deny-with-interrupt |
| `claude-code-sdk` | 0.0.3 | abandoned (2025-06) | not evaluated further |
| `claude-cli-sdk` | 0.5.1 | semi-active (2026-03) | not evaluated further |

**Choice: hand-roll.** Recorded reasoning:

1. The needed surface is small — the spike's whole protocol module plus session task
   is ~700 lines with tests, written in about an hour against live captures. A
   dependency must clear a low bar to beat that, and neither crate clears it.
2. `claude-agent-sdk` predates ten months of CLI evolution; its parity target is the
   Python SDK of late 2025. Betting Epik's interface on an unmaintained wrapper of a
   fast-moving protocol is the worst of both worlds.
3. `claude-code-sdk-rust` is credible and current (it would be the fallback), but it
   owns the exact joints Epik cares about — permission UX, event shape, session
   lifecycle — behind its own opinions, while the authoritative contract (`sdk.d.ts`)
   is Anthropic's, not the crate's. Tracking upstream directly removes a middleman
   that can lag or reinterpret.
4. The protocol's failure mode is additive drift, and a vendored parser with
   unknown-tolerant enums degrades gracefully by construction; a third-party crate's
   strictness decisions are outside our control.

## What broke and why (all resolved)

- GitHub's contents API refused to create the workflow file (`workflow` scope missing
  from the keyring token) — pushed via git-over-SSH instead. Not app-related.
- No `can_use_tool` requests appeared at first: Bill's global `"Bash"` allow rule
  auto-approved everything. Diagnosis above under gotchas; forced with ask rules.
- Nothing in the protocol or process layer itself broke during the spike.

## Distribution story vs MCPB

- The app is a **single 1.9MB release binary** (stable Rust, tokio+serde+ratatui). It
  resolves the engine at startup: PATH, then known install locations; parses
  `claude --version`; enforces a floor (2.1); fails with actionable guidance
  (`epik-app doctor` prints the same report).
- **The uv pattern is confirmed feasible**: the official installer is a thin shell
  over a versioned, checksummed release endpoint —
  `downloads.claude.ai/claude-code-releases/{stable|latest}` → version,
  `/<version>/manifest.json` → per-platform sha256, `/<version>/<platform>/claude` →
  standalone binary. A future epik-app can fetch and pin its own engine exactly the
  way uv manages Pythons (~100 lines: reqwest + sha256 + unpack to app support dir).
  Not implemented in the spike; verified against the live endpoint.
- **Vs MCPB**: the MCPB path distributes Epik *into* Claude Desktop — user installs
  Desktop, then the bundle; Epik lives inside a host Anthropic controls, and the
  "assembly" (Desktop + MCPB + terminal for builds) is the friction the spike was
  chartered to remove. The app path is: install Claude Code (one installer, which the
  app can eventually do itself), download epik-app, run. Auth inherits from the CLI
  either way. One binary, one engine, no Desktop dependency, and the interface is
  Epik's own — at the cost of owning a UI (the ~350-line TUI today, more if adopted).

## Recommendation for Epik

**Adopt-with-conditions.**

1. **Design-history first.** ADR-0001 decision 8 ("GitHub is the dashboard, no custom
   dashboard") and 2026-07-28-presence-not-a-connector ("no chat UI, no owned
   infrastructure") both stand against an Epik-owned surface. This spike is evidence
   to supersede them deliberately, not around them. Draft ADR:
   `docs/2026-08-01-rust-app-spike.md` (in this repo, ready to move).
   Note the app keeps "no owned infrastructure" intact — it is local software; only
   "no chat UI" is genuinely superseded.
2. **Vendor the protocol, track `sdk.d.ts`.** Keep the parser unknown-tolerant; add a
   canary test that runs the installed CLI and asserts the handshake shape; check the
   init `capabilities` list at runtime.
3. **Settings isolation policy.** Decide explicitly what an Epik session inherits
   from `~/.claude` (auth: yes; hooks/plugins/allow-rules: probably not — use
   `--settings` + `--strict-mcp-config` for a reproducible surface).
4. **V-next shape if adopted**: this session layer + a real UI pass (the V2
   dashboard sketch: dependency-graph pane fed from GitHub state, build-status
   colors), engine self-management (uv pattern), `summon-epik` MCP prompt instead of
   a persona file path, and macOS/Linux release builds in CI.

## Budget and time

- Wall clock: kickoff 2026-07-31 23:54 CDT → all milestones by ~03:00 CDT (one
  session, well inside the two-day limit).
- Spend: CLI probes + demos ≈ $2 (haiku for mechanics, sonnet for the Epik demo);
  the `feature_launch` build ran on the target repo's own Actions + API key as
  designed. Session total estimated well under $50 against the $200 cap.
