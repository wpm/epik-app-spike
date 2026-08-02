# epik-app

A Tauri GUI for Epik: a native desktop interface driving Claude Code over
stream-json.

This began as a feasibility spike — *can a single locally-running Rust application
serve as Epik's interface, with Claude Code as the agent engine?* The answer is in
[FINDINGS.md](FINDINGS.md) (verdict: GREEN), and 0.2.0 is that answer built out
into an application.

## What's here

A Cargo workspace, plus one crate deliberately outside it.

| Crate | What it is |
| --- | --- |
| `crates/epik-core` | The session layer, with no interface attached. Everything about driving Claude Code and nothing about displaying it. |
| `crates/epik-app` | The Tauri 2 host: one window, at most one session, the command surface the frontend uses. |
| `crates/epik-ui` | The Leptos frontend, compiled to wasm by Trunk. Not a workspace member — it targets `wasm32-unknown-unknown`, so `cargo build --workspace` has no business building it. |

Inside the core:

- `protocol.rs` — typed serde view of the CLI's stream-json wire format, both
  directions, unknown-tolerant by construction. The protocol tests are captures
  from a live CLI (2.1.220).
- `event.rs` — the types that cross the IPC boundary. Behind no feature flag, so
  the wasm frontend deserializes *these* types rather than a mirrored copy.
- `session.rs` — `Session::spawn` runs `claude -p` bidirectionally and turns the
  stream into `SessionEvent`s on one channel; `SessionHandle` sends user turns,
  permission decisions, interrupts, and policy edits.
- `permission.rs` — `PermissionPolicy`: ordered rules over exact / prefix /
  wildcard tool matchers, first match wins, no match means ask.
- `profile.rs` — named session configurations. The Epik profile lives here.
- `doctor.rs` — engine resolution that does not depend on `PATH`, and the report
  the doctor screen renders.

The spike's ratatui TUI is gone. It proved the session layer drives a real
interface, which is all it was for.

## Running it

Needs Claude Code ≥ 2.1 installed and authenticated, a Rust toolchain, the
`wasm32-unknown-unknown` target, and [Trunk](https://trunkrs.dev).

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

trunk build --config crates/epik-ui/Trunk.toml   # build the frontend first
cargo run -p epik-app                            # then the window
```

The frontend has to be built before the app, because Tauri embeds it at compile
time. If it has not been, the window shows a "Frontend not built" page instead of
a blank screen.

For development, `trunk serve` in `crates/epik-ui` gives hot reload — but only
inside the Tauri window; opened in a browser it shows a "No Epik host" screen,
because there is no `invoke` to call.

On Linux, Tauri needs its system libraries first:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

### Building a bundle

```sh
cargo install tauri-cli --version "^2" --locked
cd crates/epik-app
trunk build --config ../epik-ui/Trunk.toml
cargo tauri build                 # or --bundles deb / dmg / app
```

On macOS this produces an **unsigned** `.app` and `.dmg` under
`target/release/bundle/`. Unsigned means Gatekeeper will refuse it on first open;
right-click → Open, or `xattr -dr com.apple.quarantine Epik.app`. Signing is not
set up in 0.2.0.

## The doctor screen

Claude Code is treated the way uv treats Python: an engine the app finds rather
than a dependency the user must pre-assemble. On startup the app resolves it, and
when it cannot the window renders the doctor report **instead of** the chat
interface — what was looked for, what was found, and what to do about it, with a
retry that re-resolves so installing Claude Code does not require restarting the
app.

Resolution does not depend on `PATH`, deliberately. An app launched from Finder,
the Dock, or Spotlight inherits `launchd`'s environment — roughly
`/usr/bin:/bin:/usr/sbin:/sbin` — so a `claude` in `~/.local/bin` is invisible to
a `PATH` search even though it works perfectly in a terminal. The app searches
`PATH` first (so a deliberately-placed binary wins) and then the locations Claude
Code actually installs to: `~/.local/bin`, `~/.claude/local`,
`/opt/homebrew/bin`, `/usr/local/bin`.

Four outcomes are distinguished, because they need four different sentences:
found and supported; found but older than the floor (2.1); found but it would not
run; not found at all.

## The Epik profile

The default session. A profile is a named `SessionConfig` — what kind of session
this is — and this one is what makes the window an Epik interface rather than a
generic chat client.

| Part | Value |
| --- | --- |
| Model | `claude-sonnet-5` |
| Persona | `plugin/skills/summon/persona.md` from the Epik checkout, passed as `--append-system-prompt` |
| MCP | EpikMCP, run as `uv run --project <checkout>/mcp epik-mcp` |
| Settings isolation | `--settings '{}'` plus `--strict-mcp-config` |
| Pre-allowed | read-only EpikMCP tools, and `Read` / `Glob` / `Grep` / `NotebookRead` / `TodoWrite` |
| Gated | everything else, including `Bash`, `Write`, `Edit`, and every mutating EpikMCP tool |

**Settings isolation** means the session does not inherit the user's project or
user `settings.json`, so nothing they allowed elsewhere silently pre-approves a
tool this profile means to gate: the app's `PermissionPolicy` becomes the only
policy. `--strict-mcp-config` means only the MCP servers named above are attached,
because a profile that inherited unknown servers would be gating a tool list it
cannot predict. **Auth is unaffected** — it is not part of settings, so the
session inherits whatever the `claude` CLI is logged in as, and the app never
touches credentials. GitHub auth comes from `gh` the same way.

Read-only tools are pre-allowed because an agent monitoring a build polls them
every few seconds, and a prompt for each would teach the user to click Allow
without reading — worse than not asking. `Bash` is gated however innocent the
command looks, because it can do anything.

The checkout is found via `$EPIK_CHECKOUT`, then `~/Projects/Epik/Epik`,
`~/Epik/Epik`, `~/Epik`. Without one the session still starts, in reduced form,
and says what it could not find rather than leaving you to infer it from tools
that are absent.

The `Plain` profile is the other one: the CLI's defaults, nothing pre-decided,
every ask surfaced. Reach for it when the question is what the engine will try to
do.

**One thing worth knowing:** the app's policy is the second gate, not the only
one. Claude Code decides for itself which tool calls to route to the permission
prompt, and it does not route every one — so the guarantee is that *of the calls
Claude Code asks about*, only pre-allowed read-only ones are answered without a
human. See [docs/0.2.0-gui-demo.md](docs/0.2.0-gui-demo.md).

## The interface

Two panels and a status bar. Chat on the left: user turns, assistant output as
markdown, tool activity as collapsible cards that open pending and complete
success or error, and permission cards that block the turn until answered by hand
— allow, deny with an optional message the model reads, or always allow this tool
for this session. The right panel carries session metadata and the engine's
stderr, which opens itself when a session closes badly. The status bar carries
state, model, CLI version, turn count, cumulative cost, and the exit code if the
session ended.

Screenshots and the demo transcript are in
[docs/0.2.0-gui-demo.md](docs/0.2.0-gui-demo.md).

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# the frontend is a separate workspace on a different target
cargo clippy --manifest-path crates/epik-ui/Cargo.toml --all-targets \
  --target wasm32-unknown-unknown -- -D warnings
cargo test --manifest-path crates/epik-ui/Cargo.toml
```

The frontend's tests run on the host target because the state it renders is a pure
fold over the event stream, with no Leptos or IPC in it — so the properties worth
pinning need no browser.

Examples run against a live CLI:

```sh
cargo run -p epik-core --example m0_handshake   # one handshake
cargo run -p epik-core --example m1_demo        # a scripted turn with tool use
```

`demo/fake-engine.py` is a scripted stand-in for the CLI that drives every state
the window can be in, for looking at the interface without spending tokens. See
[docs/manual-checks.md](docs/manual-checks.md).

## Reading order

- [FINDINGS.md](FINDINGS.md) — the spike's verdict.
- [LOG.md](LOG.md) — the running narrative, enough to resume cold.
- [docs/design-history/](docs/design-history/) — design decisions, newest last.
- [docs/2026-08-01-rust-app-spike.md](docs/2026-08-01-rust-app-spike.md) — the
  draft ADR.
- [docs/0.2.0-gui-demo.md](docs/0.2.0-gui-demo.md) — the demo, and what it does
  not cover.
- [docs/manual-checks.md](docs/manual-checks.md) — what cannot be asserted in
  `cargo test`, and what already is.
