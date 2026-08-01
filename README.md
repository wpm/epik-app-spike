# epik-app-spike

Feasibility spike: can a single locally-running Rust application serve as Epik's
interface, with Claude Code as the agent engine?

**The answer is in [FINDINGS.md](FINDINGS.md) (verdict: GREEN).** The running
narrative, sufficient to resume the spike cold, is in [LOG.md](LOG.md). A draft ADR
in Epik's design-history format is at
[docs/2026-08-01-rust-app-spike.md](docs/2026-08-01-rust-app-spike.md).

## What's here

The repository is a Cargo workspace. `crates/epik-core` is the session layer with
no interface attached — the crate every host embeds:

- `crates/epik-core/src/protocol.rs` — typed serde view of the CLI's stream-json wire
  format, both directions, unknown-tolerant by construction. The protocol tests are
  captures from a live CLI (2.1.220).
- `crates/epik-core/src/session.rs` — `Session::spawn` runs `claude -p`
  bidirectionally and turns the stream into `SessionEvent`s on a channel;
  `SessionHandle` sends user turns, permission decisions (`can_use_tool`
  allow/deny), and interrupts.
- `crates/epik-core/src/doctor.rs` — engine resolution: locate `claude`,
  version-check, actionable failure messages.
- `crates/epik-core/examples/m0_handshake.rs`,
  `crates/epik-core/examples/m1_demo.rs` — the M0/M1 milestone proofs, runnable
  against a live CLI.
- `demo/run-epik.sh` — the M3 demo: Epik persona + EpikMCP attached, mutating tools
  gated by the app's permission prompt.

The spike's ratatui TUI is gone. It proved the session layer drives a real
interface, which is all it was for; the interface 0.2.0 ships is the Tauri
application, and keeping a terminal dependency in the tree would have contradicted
the point of extracting a UI-free core.

## Running

```sh
cargo test --workspace                          # the protocol and session tests
cargo run -p epik-core --example m0_handshake   # M0: one handshake against a live CLI
cargo run -p epik-core --example m1_demo        # M1: a scripted turn with tool use
```

Hosts inherit Claude auth from the `claude` CLI and GitHub auth from `gh` — nothing
here touches credentials.
