# LOG — epik-app Rust interface feasibility spike

Running log, newest entries at the bottom. This file is the session memory: it must
always be sufficient for a fresh session to resume the spike.

## 2026-07-31 23:54 CDT — Kickoff

- **Time limit**: two days from now → hard stop 2026-08-02 ~23:54 CDT.
- **Budget**: hard stop at $200 estimated spend. Current estimate: ~$0.
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
    $0.271. Test runs will pin `--model claude-haiku-4-5` (~$0.03/turn). The real app
    will likely want config isolation flags evaluated later.
  - Final `result` message carries `total_cost_usd`, `num_turns`, usage — the app
    gets budget telemetry for free.
- **Budget**: CLI spend so far ≈ $0.30. Session spend not precisely known; rough
  estimate ≤ $5 total. Well under cap.
- Next: M1 — bidirectional stream-json + control protocol. Start by probing
  `--input-format stream-json` and `can_use_tool` control requests in the shell.
