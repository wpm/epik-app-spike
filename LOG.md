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
