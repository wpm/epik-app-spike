# epik-app-spike

Feasibility spike: can a single locally-running Rust application serve as Epik's
interface, with Claude Code as the agent engine?

**The answer is in [FINDINGS.md](FINDINGS.md) (verdict: GREEN).** The running
narrative, sufficient to resume the spike cold, is in [LOG.md](LOG.md). A draft ADR
in Epik's design-history format is at
[docs/2026-08-01-rust-app-spike.md](docs/2026-08-01-rust-app-spike.md).

## What's here

- `src/protocol.rs` — typed serde view of the CLI's stream-json wire format, both
  directions, unknown-tolerant by construction. The protocol tests are captures from
  a live CLI (2.1.220).
- `src/session.rs` — `Session::spawn` runs `claude -p` bidirectionally and turns the
  stream into `SessionEvent`s on a channel; `SessionHandle` sends user turns,
  permission decisions (`can_use_tool` allow/deny), and interrupts.
- `src/tui.rs` — minimal ratatui chat: streaming output, tool events, permission
  banner (y/n), Esc interrupt, cost in the status line.
- `src/doctor.rs` — engine resolution: locate `claude`, version-check, actionable
  failure messages (`epik-app doctor`).
- `examples/m0_handshake.rs`, `examples/m1_demo.rs` — the M0/M1 milestone proofs,
  runnable against a live CLI.
- `demo/run-epik.sh` — the M3 demo: Epik persona + EpikMCP attached, mutating tools
  gated by the app's permission banner.

## Running

```sh
cargo run                         # plain chat with your default model
cargo run -- doctor               # engine report
./demo/run-epik.sh                # the Epik demo (needs a local Epik checkout)
```

The app inherits Claude auth from the `claude` CLI and GitHub auth from `gh` — it
never touches credentials.
