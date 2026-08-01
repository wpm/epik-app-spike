# Manual checks

Things that cannot be asserted in `cargo test` because they are properties of a
packaged application on a particular operating system. Each one says what to do,
what to expect, and why an automated test is not enough.

## Engine resolution from a Finder launch (macOS)

**Why this is manual.** The automated test
(`epik-core::doctor::tests::resolves_with_no_path_at_all`) proves the resolution
*logic* does not depend on `PATH`: it runs a search with `path_env: None` against
a fake install and finds it. What it cannot prove is that the packaged app
actually inherits the environment we think it does. `launchd` gives a
Finder-launched process roughly `/usr/bin:/bin:/usr/sbin:/sbin`, and the only way
to be sure about that is to double-click the app.

**Setup.** Claude Code installed in `~/.local/bin` (the official installer's
location) and *not* symlinked into any directory on the default `launchd` path.
Confirm with:

```sh
which -a claude                 # should be ~/.local/bin/claude
ls /usr/local/bin/claude        # should not exist
ls /opt/homebrew/bin/claude     # should not exist
```

**Steps.**

1. `cargo tauri build` in `crates/epik-app`.
2. Open `target/release/bundle/macos/` in Finder and double-click `Epik.app`.
   Double-click it — do not run the binary from a terminal, which would give it
   the shell's `PATH` and defeat the check.
3. The window must show the chat interface, and the right panel must show the
   engine path as `~/.local/bin/claude` (expanded) and the CLI version.

**Failure looks like** the doctor screen saying "No `claude` binary found", with
`~/.local/bin` listed under `searched`. That means `known_dirs()` did not include
the directory the engine is in — `$HOME` was not set as expected, or the install
is somewhere new. Add the directory to `doctor::known_dirs`.

**Also worth doing from Finder**, since they share the same cause:

- Quit with ⌘Q, then `pgrep -fl claude`. Nothing may remain. The automated test
  (`session::tests::end_session_kills_an_engine_that_ignores_stdin_closing`)
  proves `end_session` reaps even a wedged engine, but only a real quit proves
  the app's exit handler calls it.
- Close the window with ⌘W and check `pgrep -fl claude` again.

## What the automated tests already cover

Do not repeat these by hand:

| Property | Test |
| --- | --- |
| Resolution works with no `PATH` at all | `doctor::tests::resolves_with_no_path_at_all` |
| Resolution works with `launchd`'s `PATH` | `doctor::tests::resolves_with_the_path_a_gui_launch_actually_gets` |
| A too-old engine is distinguished from a missing one | `doctor::tests::an_engine_below_the_floor_is_too_old_not_missing` |
| A broken engine is distinguished from a missing one | `doctor::tests::an_engine_that_will_not_run_is_unusable_not_missing` |
| A clean quit reaps the engine | `session::tests::end_session_reaps_an_engine_that_exits_on_its_own` |
| A wedged engine is killed rather than orphaned | `session::tests::end_session_kills_an_engine_that_ignores_stdin_closing` |
| Quitting twice does not stall | `session::tests::end_session_is_idempotent` |

## Linux equivalent, for CI-adjacent checking

The window and both screens can be verified headlessly on Linux, which is how the
screenshots in `docs/screenshots/` were produced:

```sh
Xvfb :99 -screen 0 1280x900x24 &
DISPLAY=:99 \
  WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 \
  WEBKIT_DISABLE_DMABUF_RENDERER=1 \
  WEBKIT_DISABLE_COMPOSITING_MODE=1 \
  ./target/debug/epik-app &
sleep 10 && DISPLAY=:99 import -window root /tmp/epik.png
```

Scrubbing the environment (`HOME=/tmp/empty PATH=/usr/bin:/bin`) produces the
doctor screen instead. The three `WEBKIT_DISABLE_*` variables are container
workarounds — WebKit's sandbox and DMA-BUF renderer need kernel features a
container does not have — and are not needed on a normal desktop.

## Looking at the interface: the scripted fake engine

`demo/fake-engine.py` is a stand-in for the `claude` CLI that speaks enough
stream-json to drive every part of the window: streamed markdown, a tool call that
succeeds, one that fails, a permission ask answered by hand, and a second ask for
the same tool so "always allow" has something to suppress. It answers `--version`
with a supported version, so the doctor accepts it.

```sh
mkdir -p /tmp/fake-bin
cp demo/fake-engine.py /tmp/fake-bin/claude && chmod +x /tmp/fake-bin/claude
PATH=/tmp/fake-bin:$PATH cargo run -p epik-app
```

The same conversation every time, no API tokens spent, and the awkward states —
a failing tool, two asks in a row — arrive on cue. The screenshots in
`docs/screenshots/` were taken this way, under Xvfb, driving the window with
`xdotool`. Note that with no window manager running, X focus follows the pointer:
click into the composer with `xdotool mousemove X Y click 1` before typing, or the
keystrokes go nowhere.

The automated tests do not use this file — they carry their own stubs inline in
`crates/epik-app/tests/ipc_stream.rs`.
