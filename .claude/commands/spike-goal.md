# Goal: epik-app — Rust interface feasibility spike

## Mission

Determine, by building it, whether a single locally-running Rust application can serve
as Epik's interface — replacing the current assembly of Claude Desktop + MCPB + terminal
with one app — using Claude Code as the agent engine.

This is a spike. The deliverable is **an answer**, supported by working code. A clear,
well-evidenced "no" is a successful outcome. A prototype that works is a successful
outcome. An ambiguous shrug is the only failure mode.

## Architectural premise (test it; do not relitigate it)

- The official Agent SDKs (TypeScript, Python) do not reimplement the agent loop. They
  drive the `claude` CLI as a subprocess over the stream-json protocol. A Rust app doing
  the same is architecturally identical to an official SDK, not a fork of the harness.
- Therefore: Rust owns the UI, process lifecycle, and Epik state. Claude Code remains
  the engine. We are not rewriting the harness; the "never write our own chat UI" lesson
  applies to engines, and we are honoring it.
- Candidate protocol layers, to be evaluated in M1 (pick one, record why):
  a. Community crate `claude-agent-sdk-rust` (Wally869; parity target = Python SDK)
  b. Community crate `claude-code-sdk-rust` (PandelisZ; Tokio-native, control requests)
  c. Hand-rolled: spawn `claude -p --input-format stream-json --output-format stream-json`
     and implement the protocol surface directly (serde types + a session task)

## Scope decisions already made

- **TUI first: ratatui.** No GUI framework in this spike. All the risk lives in the
  protocol and process layer, which is identical under any shell. (Confirmed by Bill.)
- **Fresh repository.** This is a spike → parallel universe. Create a new repo
  (suggested: `epik-app-spike`). Do NOT touch the Epik repo, its issues, or any clone
  of it. No Epik issues are created for this work.
- **Rust stable, tokio, serde, ratatui.** clippy + rustfmt clean at every checkpoint.
  Tests only where they buy confidence — protocol parsing and session state, not UI.
- **Epik conventions apply**: work on branches, small commits, commit messages name the
  milestone (e.g. `M1: handle can_use_tool control request`).

## Milestones (ordered by risk, not by user value)

- **M0 — Handshake.** From Rust: spawn `claude -p` one-shot with stream-json output;
  parse the message stream into typed structs; print the assistant text. Proves the
  subprocess + parse layer. (Expected: hours.)
- **M1 — Session.** Bidirectional stream-json: multi-turn interactive session,
  streaming deltas, interrupts, and — critically — the control protocol: tool
  permission requests answered programmatically from Rust. This is the load-bearing
  joint. Evaluate crates (a)/(b) against hand-rolling (c) here; pick and record.
- **M2 — Chat.** Minimal ratatui chat: input box, streaming assistant output, visible
  tool-use events, session status line. Ugly is fine; legible is required.
- **M3 — Epik-ness.** On launch, the app summons Epik (persona loaded, "Hello, I'm
  Epik."). EpikMCP attached via CLI MCP config. From inside the app: create an issue,
  `feature_launch` it, and monitor the run to completion. This is the success demo.
- **M4 — Distribution.** Single release binary. The app locates the `claude` CLI,
  version-checks it, and reports clearly when missing; investigate whether the app can
  fetch/manage its own engine (the uv-manages-Python pattern applied to `claude`).
  Write the would-be install story and compare it honestly to the MCPB path.
- **M5 (stretch, only if M0–M4 land with time to spare) — polish**: dependency-graph
  pane fed from GitHub state, build-status colors per the V2 dashboard sketch.

## Kill criteria (fire early, record evidence, stop)

- **K1**: M1's control protocol (permissions, interrupts, session control) cannot be
  made reliable within ~1 focused day. Record the exact failure: which control request,
  which CLI version, what the crates do wrong.
- **K2**: The stream-json surface is both undocumented AND the community crates are
  broken against the current CLI, with drift too large to vendor. Record versions and
  the specific incompatibilities.
- **K3**: Budget cap reached (see constraints). Report whatever milestone was reached.

A kill is an answer. Write it up with the same care as a success.

## Constraints

- **Budget**: log estimated spend at every checkpoint; hard stop and report at $200.
- **Time limit**: two days from kickoff. Record the kickoff timestamp as the first
  LOG.md entry; when the limit expires, stop and report exactly as with the budget cap.
- **Autonomy**: do not wait for Bill. The only legitimate stops are: goal complete,
  a kill criterion fired, budget cap, or a credential you genuinely do not have.
- **Secrets**: never commit keys; the app inherits auth from the `claude` CLI — that
  is a feature, note it in FINDINGS.

## Deliverables

1. The prototype, at whatever milestone was reached, on `main` of the spike repo.
2. **`FINDINGS.md`** — the answer. Verdict (GREEN / YELLOW / RED) with one-paragraph
   justification up top, then: protocol notes (what the stream-json surface actually
   is, how stable it looks), crate evaluation and choice, what broke and why,
   distribution story vs MCPB, and a recommendation for Epik (adopt / adopt-with-
   conditions / drop, and what V-next would look like if adopted).
3. **`LOG.md`** — timestamped running log (see /loop prompt). This file is also the
   session memory: it must always be sufficient for a fresh session to resume.
4. If verdict is GREEN or YELLOW: a draft ADR in the spike repo
   (`docs/2026-XX-XX-rust-app-spike.md`, Epik design-history format) ready for Bill
   to move into Epik's design-history if he adopts.
