#!/usr/bin/env python3
"""A scripted stand-in for the `claude` CLI, for looking at the interface.

Speaks enough of the stream-json protocol to drive every part of the window:
streamed markdown, a tool call that succeeds, a tool call that fails, a permission
ask answered by hand, a second ask for the same tool (which "always allow" should
suppress), and a result line carrying a cost. It answers `--version` with a
supported version so the doctor accepts it.

Why this exists: the interface is much easier to check against a fixed script than
against a live model. The same conversation every time means a screenshot diff
means something, no API tokens are spent, and states that are awkward to provoke
on purpose — a failing tool, two asks in a row — arrive on cue.

Usage — put it on PATH ahead of the real engine, under the name it looks for:

    mkdir -p /tmp/fake-bin && cp demo/fake-engine.py /tmp/fake-bin/claude
    chmod +x /tmp/fake-bin/claude
    PATH=/tmp/fake-bin:$PATH cargo run -p epik-app

This is a development fixture. The automated tests use their own stubs inline
(`crates/epik-app/tests/ipc_stream.rs`) and do not depend on this file.
"""

import json
import sys
import time

VERSION = "2.1.220 (Claude Code)"


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def delta(text):
    emit(
        {
            "type": "stream_event",
            "session_id": "fake-session",
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": text},
            },
        }
    )


def stream(text, chunk=12, pause=0.02):
    """Emit `text` in small pieces, the way the real engine does."""
    for i in range(0, len(text), chunk):
        delta(text[i : i + chunk])
        time.sleep(pause)


def assistant_text(text):
    emit(
        {
            "type": "assistant",
            "session_id": "fake-session",
            "message": {"role": "assistant", "content": [{"type": "text", "text": text}]},
        }
    )


def tool_use(tool_id, name, tool_input):
    emit(
        {
            "type": "assistant",
            "session_id": "fake-session",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": tool_id, "name": name, "input": tool_input}
                ],
            },
        }
    )


def tool_result(tool_id, content, is_error=False):
    emit(
        {
            "type": "user",
            "session_id": "fake-session",
            "message": {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": tool_id,
                        "content": content,
                        "is_error": is_error,
                    }
                ],
            },
        }
    )


def ask(request_id, tool_name, tool_input, description):
    emit(
        {
            "type": "control_request",
            "request_id": request_id,
            "request": {
                "subtype": "can_use_tool",
                "tool_name": tool_name,
                "display_name": tool_name,
                "input": tool_input,
                "description": description,
                "decision_reason_type": "rule",
                "decision_reason": "no matching allow rule for this tool",
            },
        }
    )


def read_line():
    line = sys.stdin.readline()
    return line.strip() if line else None


def wait_for_permission_answer():
    """Read until a permission answer arrives; return True if it allowed."""
    while True:
        line = read_line()
        if line is None:
            return None
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        response = message.get("response", {}).get("response")
        if isinstance(response, dict) and "behavior" in response:
            return response["behavior"] == "allow"


REPLY_ONE = """I can see the repository. Here is what stands out:

- **`crates/epik-core`** — the session layer, no interface attached
- **`crates/epik-app`** — the Tauri host
- **`crates/epik-ui`** — this window

The interesting part is the permission boundary:

```rust
let action = policy.decide(tool_name);
```

Let me check the tree."""


def turn_one():
    stream(REPLY_ONE)
    assistant_text(REPLY_ONE)

    # An ask the user answers by hand.
    ask(
        "ask-1",
        "Bash",
        {"command": "git status --short", "description": "Check the working tree"},
        "Check the working tree",
    )
    if wait_for_permission_answer():
        tool_use("toolu_1", "Bash", {"command": "git status --short", "description": "Check the working tree"})
        tool_result("toolu_1", " M crates/epik-ui/src/app.rs\n?? crates/epik-ui/src/components/\n")
        stream("\n\nTwo changes: a modified `app.rs` and a new components directory.")
        assistant_text("Two changes: a modified `app.rs` and a new components directory.")
    else:
        stream("\n\nUnderstood — I won't run that. ")
        stream("Tell me what you would like me to look at instead.")
        assistant_text(
            "Understood — I won't run that. Tell me what you would like me to "
            "look at instead."
        )

    emit(
        {
            "type": "result",
            "subtype": "success",
            "is_error": False,
            "result": "done",
            "total_cost_usd": 0.0184,
            "num_turns": 2,
            "duration_ms": 3120,
            "session_id": "fake-session",
        }
    )


def turn_two():
    stream("Running the tests now.")
    assistant_text("Running the tests now.")

    # Two asks for the same tool: "always allow" on the first should mean the
    # second never surfaces, because the core's policy answers it.
    ask("ask-2", "Bash", {"command": "cargo test --workspace"}, "Run the test suite")
    if wait_for_permission_answer():
        tool_use("toolu_2", "Bash", {"command": "cargo test --workspace"})
        tool_result("toolu_2", "test result: ok. 45 passed; 0 failed")

        ask("ask-3", "Bash", {"command": "cargo clippy --workspace"}, "Run clippy")
        allowed = wait_for_permission_answer()
        if allowed is None:
            return
        if allowed:
            tool_use("toolu_3", "Bash", {"command": "cargo clippy --workspace"})
            # A failing tool call, so the error card has something to render.
            tool_result(
                "toolu_3",
                "error: unused variable `client`\nerror: could not compile `epik-ui`",
                is_error=True,
            )
            stream("\n\nClippy found an unused variable. I'll fix it.")
            assistant_text("Clippy found an unused variable. I'll fix it.")

    emit(
        {
            "type": "result",
            "subtype": "success",
            "is_error": False,
            "total_cost_usd": 0.0231,
            "num_turns": 3,
            "session_id": "fake-session",
        }
    )


def main():
    if "--version" in sys.argv:
        print(VERSION)
        return 0

    emit(
        {
            "type": "system",
            "subtype": "init",
            "session_id": "fake-session",
            "model": "claude-sonnet-5",
            "claude_code_version": VERSION,
            "tools": ["Bash", "Read", "Write", "Edit", "Glob", "Grep"],
        }
    )

    turns = [turn_one, turn_two]
    taken = 0
    while True:
        line = read_line()
        if line is None:
            return 0
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue

        if message.get("type") == "control_request":
            # The host's `initialize`; acknowledge it and carry on.
            emit(
                {
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": message.get("request_id", "unknown"),
                        "response": {},
                    },
                }
            )
        elif message.get("type") == "user":
            handler = turns[taken] if taken < len(turns) else turns[-1]
            taken += 1
            handler()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (BrokenPipeError, KeyboardInterrupt):
        sys.exit(0)
