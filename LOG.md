# LOG — epik-app Rust interface feasibility spike

Running log, newest entries at the bottom. This file is the session memory: it must
always be sufficient for a fresh session to resume the spike.

## 2026-07-31 23:54 CDT — Kickoff

- **Time limit**: two days from now → hard stop 2026-08-02 23:54 CDT.
- **Budget**: hard stop at \$200 estimated spend. Current estimate: \$0.
- Environment verified:
  - `claude` CLI 2.1.220 at `/opt/homebrew/bin/claude`
  - Rust 1.96.0 stable, cargo, clippy 0.1.96, rustfmt 1.9.0
  - Repo `epik-app-spike` clean on `main`, scaffolding only (`.claude/` prompts).
- Plan: M0 (handshake) on branch `m0-handshake`. First step: probe the actual
  stream-json output shape from the CLI in the shell, then build typed parse in Rust.

## 2026-08-01 00:05 CDT — M0 complete

- Branch `m0-handshake`, commit `4803b7d`. Cargo project `epik-app`: tokio, serde,
  serde_json, anyhow.
- `src/protocol.rs`: `StreamMessage` tagged enum (system / assistant / user / result /
  stream_event) with `#[serde(untagged)] Unknown(Value)` fallbacks at both the message
  and content-block level — unknown types degrade, never fail the stream. 6 unit tests
  built from real wire captures (probe saved in scratchpad `m0-probe.jsonl`).
- `src/main.rs`: spawns `claude -p <prompt> --output-format stream-json --verbose`,
  parses lines, prints assistant text; init/tool-use/result go to stderr.
- Live verification: session opened, `handshake` echoed, result parsed. clippy
  `-D warnings` clean, rustfmt clean, 6/6 tests.
- **Protocol observations (for FINDINGS):**
  - `-p --output-format stream-json` requires `--verbose`.
  - Every stdout line is one JSON object tagged `type`. `system/init` opens the
    session and reports session_id, model, tools, CLI version, apiKeySource.
  - User-level config bleeds in: hooks fire (`system/hook_started`), plugins load,
    global CLAUDE.md creates a ~24.5k-token cache write → one cold Fable turn cost
    \$0.271. Test runs will pin `--model claude-haiku-4-5` (~\$0.03/turn). The real app
    will likely want config isolation flags evaluated later.
  - Final `result` message carries `total_cost_usd`, `num_turns`, usage — the app
    gets budget telemetry for free.
- **Budget**: CLI spend so far ≈ \$0.30. Session spend not precisely known; rough
  estimate ≤ \$5 total. Well under cap.
- Next: M1 — bidirectional stream-json + control protocol. Start by probing
  `--input-format stream-json` and `can_use_tool` control requests in the shell.

## 2026-08-01 00:55 CDT — M1 wire protocol proven, session layer built

- Branch `m1-session`, commit `5e1cde4`.
- **Protocol reference found**: the official TS SDK's `sdk.d.ts` (npm
  `@anthropic-ai/claude-agent-sdk@0.3.220`, unpacked in scratchpad `package/`) is a
  ~300KB richly documented type spec of the whole stream-json + control surface.
  The protocol is *documented*, just not as prose. Kills half of K2's premise.
- **Spawn recipe** (verbatim from sdk.mjs): `claude -p --output-format stream-json
  --verbose --input-format stream-json` + `--permission-prompt-tool stdio` when the
  client wants permission asks (the flag is hidden from --help but present in the
  binary and functional).
- **Live probes (python, scratchpad m1-control-probe.py / m1-probe2.py), all against
  CLI 2.1.220:**
  - `initialize` control request → success response with commands/models/account.
  - `can_use_tool` control_request arrives on ask-gated tools; answered with
    `{"behavior":"allow","updatedInput":...}` → tool runs; `{"behavior":"deny",
    "message":...}` → tool_result is_error=true with our message, model continues.
  - Gotcha: asks only fire when the permission system would prompt. Bill's global
    settings allow all Bash, so probes force `--settings '{"permissions":{"ask":
    ["Bash"]}}'` + `--permission-mode default`. The real app must think about
    settings isolation vs inheritance.
  - `interrupt` control request mid-stream → success ack (`still_queued:[]`),
    truncated turn, result subtype `error_during_execution`, is_error=true.
  - Multi-turn: same process, same session_id; each turn re-emits `system/init` and
    ends with a `result`. CLI exits cleanly on stdin EOF.
  - Streaming: `--include-partial-messages` emits `stream_event` lines wrapping raw
    API events; text deltas = `content_block_delta`/`text_delta`.
- **Rust session layer** (`src/session.rs`): `Session::spawn(SessionConfig)` →
  events channel + cloneable `SessionHandle` (send_user / respond_permission /
  interrupt / shutdown). Restructured to lib+bin; `examples/m1_demo.rs` scripts
  allow→deny→interrupt live: all three behaved exactly as the probes predicted
  (interrupt stopped a 1-to-300 count at 32). 11 protocol unit tests. clippy/fmt
  clean.
- **K1 verdict: does not fire.** Control protocol was reliable within ~1 hour of
  focused work, not a day.
- Crate evaluation (claude-agent-sdk v0.1.1, claude-code-sdk-rust v0.4.1) delegated
  to a subagent; report pending. Decision pending its report, but the hand-rolled
  layer is ~450 lines total and matches the SDK recipe exactly.
- **Budget**: CLI probe spend this session ≈ \$0.45 cumulative. Est. total incl.
  session ≤ $15. Next: M2 ratatui chat on top of SessionEvent.

## 2026-08-01 01:25 CDT — M2 complete

- Branch `m2-chat`, commit `2606529`. ratatui 0.29 + crossterm event-stream +
  futures. `src/tui.rs` (~350 lines): transcript with tail-follow, streaming
  deltas shown italic then replaced by the final block, tool entries ⚙/✓/✗,
  permission banner answered y/n, Esc interrupt, cost accumulator. `src/main.rs`
  is now the TUI binary; M0 one-shot lives on as `examples/m0_handshake.rs`.
- Verified live in tmux (100x30): plain chat turn; forced-ask run
  (`--permission-mode default --settings '{"permissions":{"ask":["Bash"]}}'`)
  showed the yellow ask banner, `y` allowed it, tool ran, ✓ and output rendered.
- M0→M2 took ~1.5h wall clock total. Crate-eval subagent still out; M1 decision
  paragraph in FINDINGS waits for it. Next: M3 — Epik persona + EpikMCP.

## 2026-08-01 02:20 CDT — M3 demo in flight

- Scouted the Epik repo (read-only, via subagent). Key facts: EpikMCP is a Python
  MCP server run by `uv run --project <Epik>/mcp epik-mcp`, zero env vars (auth
  delegated to ambient `gh`); persona is self-contained at
  `plugin/skills/summon/persona.md`; tools of interest: `issue_create`,
  `feature_launch(repo, feature_issue_number, base_branch, target_branch)`,
  `run_list/run_get/run_logs`. ADR convention: `docs/design-history/YYYY-MM-DD-
  topic.md`, H1 + Status/Date/Source bullets. NOTE for FINDINGS: ADR-0001
  decision 8 says "GitHub is the dashboard — no custom dashboard", and
  2026-07-28-presence-not-a-connector constrains "no chat UI, no owned infra";
  this app is the first non-GitHub Epik surface, so a GREEN verdict needs an ADR
  that squarely supersedes those.
- Demo target: `wpm/small-project` ("Test Epik", Bill's Feb sandbox) — the goal
  forbids touching epik-agent/Epik. Prepped it per Epik's own init flow:
  copied `.github/workflows/epik-build.yml` from the public epik-agent/Epik
  (pushed via SSH; keyring gh token lacks `workflow` scope — API PUT 404s, git
  push works) and set the `ANTHROPIC_API_KEY` secret from env.
- App: added `--persona-file` and `--greet` (branch `m3-epik`, commit `5deb56a`).
- Demo (running in tmux session `m3`): launched via demo/run-epik.sh (sonnet-5,
  persona loaded, EpikMCP attached, read-only tools pre-allowed). Observed:
  - Persona works: "Hello, I'm Epik" greeting; unprompted, it read LOG.md (cwd
    is the spike repo) and correctly analyzed the spike's own state — the app
    hosting Epik analyzing the app is a nice recursion.
  - Permission banner exercised repeatedly on MCP tools (gh_raw ×3,
    issue_create, feature_launch), each answered y from the keyboard.
  - Epik created issue #9 (multiply(a,b) in ops.py) and dispatched
    feature_launch → run 30685585463, then scheduled its own 2-min polling.
- Run monitor armed on this side too (gh run view loop). Cost of the demo
  session so far: \$0.76 (visible in the app's own status line).
- Next while build runs: M4 distribution work + FINDINGS skeleton.

## 2026-08-01 03:35 CDT — Checkpoint: M4 done, deliverables drafted, build rerun in flight

- M4 complete (branch `m4-distribution`, merged): `src/doctor.rs` locates the CLI
  (PATH + known locations), version-checks with a 2.1 floor, actionable errors;
  app resolves engine at startup; `epik-app doctor` subcommand. Release binary
  1.9MB. Engine self-fetch verified feasible: official installer = thin shell over
  `downloads.claude.ai/claude-code-releases/{stable|latest|<ver>/manifest.json|
  <ver>/<platform>/claude}` with per-platform sha256 — uv pattern applies directly.
- Deliverables drafted and committed on main: FINDINGS.md (verdict GREEN),
  docs/2026-08-01-rust-app-spike.md (ADR draft), README.md.
- Crate verdict recorded from my own source review (both crates downloaded in
  scratchpad): both implement the control protocol; claude-agent-sdk 0.1.1 is 10
  months stale; claude-code-sdk-rust 0.4.1 is current and credible but owns the
  joints Epik cares about. **Hand-roll chosen; claude-code-sdk-rust is the named
  fallback.** [decision made on own authority] Crate-eval subagent report still
  outstanding; will fold in if it arrives, decision does not depend on it.
- M3 build run 1 (30685585463): success as a run, but could not open PRs — repo
  setting "GitHub Actions is not permitted to create or approve pull requests".
  The build narrated this precisely onto issue #9 (failure-reporting design
  working as intended). Fixed via
  `PUT repos/wpm/small-project/actions/permissions/workflow` (write +
  can_approve_pull_request_reviews). Epik relaunched from inside the app →
  run 30685743921, in progress, monitored both by Epik (its own scheduled
  polling) and by a local monitor.
- **Budget**: probes+demos ≈ \$2.8 CLI-side (Epik session at \$2.04 on its own
  status line). Session total est. ≤ \$40. Time: ~3.7h elapsed of 48h limit.
- Next: on rerun completion — verify review PR, capture Epik's final report,
  finalize FINDINGS/LOG, push.

## 2026-08-01 04:15 CDT — GOAL COMPLETE

- Rerun 30685743921: **success**. Review PR #10 open (`feature/9-multiply` →
  `main`, `Closes #9`), main untouched, multiply() implemented with tests on the
  feature branch. Epik confirmed completion from inside the app on its own
  scheduled poll and flagged that `feature_status` is empty for standalone issues
  (V-next input, recorded in FINDINGS). Final frame of the demo saved to
  docs/m3-demo-transcript.txt; Epik session closed cleanly (\$2.81 total on its
  status line).
- M3 done end-to-end → per the loop's stop conditions, goal complete (M4 also
  done). All tasks closed. FINDINGS.md finalized GREEN; ADR draft ready for
  Bill's design-history if adopted.
- Crate-eval subagent report arrived after a nudge and was folded into FINDINGS.
  Headline: `claude-agent-sdk` 0.1.1 falsely claims Anthropic authorship (404
  anthropics/ repo URL, unaffiliated publisher) and its control protocol is
  fabricated — interrupt silently ignored by the real CLI, can_use_tool requests
  discarded. `claude-code-sdk-rust` 0.4.1 verified correct end-to-end (141 tests,
  full control protocol, forward-compatible parsing, multi-turn + interrupt
  smoke-tested). Hand-roll decision stands, now with empirical backing; the
  crate's wire_parity.rs noted as a conformance corpus to port at V-next.
- **Final spend estimate**: CLI-side (probes + haiku runs + sonnet demo + the two
  Opus builds billed to the same key via Actions) ≈ \$10-15; session overhead
  brings the total comfortably under \$50 of the \$200 cap. Elapsed: ~4.3h of the
  48h limit.
- Everything pushed to origin (main + all milestone branches).
