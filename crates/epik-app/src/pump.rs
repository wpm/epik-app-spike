//! Forwarding session events to the frontend, in order, with text coalesced.
//!
//! Streaming text arrives as a great many tiny `TextDelta`s — one per few
//! characters. Handing each one to the frontend individually means a wasm call
//! and a signal update per fragment, which is work the display cannot use: the
//! screen refreshes far less often than the deltas arrive. So deltas accumulate
//! for [`DELTA_FLUSH`] and go across as one.
//!
//! The property that matters more than the batching is **order**. A coalescer
//! that buffers text while letting other events past would reorder the stream:
//! a `ToolUse` could overtake the sentence that introduced it, and the transcript
//! would read wrongly. So every non-delta event flushes the pending text *first*,
//! and only then goes out. Text is therefore delayed relative to real time, never
//! relative to the events around it.
//!
//! [`EventSink`] is the seam. The app's sink is a `tauri::ipc::Channel`; the
//! tests' sink is a recording buffer. Both drive the same [`pump`], so the
//! ordering and coalescing tests exercise the real thing rather than a copy of it.

use std::time::Duration;

use epik_core::SessionEvent;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// How long deltas accumulate before being flushed as one event.
///
/// ~30 ms is about two frames at 60 Hz: short enough that typing still looks
/// live, long enough to collapse a burst of fragments into one update.
pub const DELTA_FLUSH: Duration = Duration::from_millis(30);

/// Where events go. Implemented by `tauri::ipc::Channel` in the app and by a
/// recording buffer in tests.
pub trait EventSink: Send + 'static {
    /// Deliver one event. An error means the far side is gone, which ends the
    /// pump — there is no point translating a stream nobody is reading.
    fn send(&self, event: SessionEvent) -> Result<(), String>;
}

impl EventSink for tauri::ipc::Channel<SessionEvent> {
    fn send(&self, event: SessionEvent) -> Result<(), String> {
        tauri::ipc::Channel::send(self, event).map_err(|e| e.to_string())
    }
}

/// Drain `events`, coalescing text, until the session's stream ends or the sink
/// goes away.
pub async fn pump<S: EventSink>(mut events: mpsc::Receiver<SessionEvent>, sink: S) {
    let mut pending = String::new();
    // `Some` exactly when there is buffered text waiting on a deadline.
    let mut deadline: Option<Instant> = None;

    loop {
        let flush_due = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                // Nothing buffered: this arm must never win, so it never
                // resolves. Cheaper and clearer than restructuring the select.
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            received = events.recv() => match received {
                // Stream ended. Whatever text is buffered is still real text, so
                // it goes out; a sink that has already gone away is nothing left
                // to report to, hence the discard.
                None => {
                    let _ = flush(&mut pending, &sink);
                    break;
                }
                Some(SessionEvent::TextDelta(text)) => {
                    if deadline.is_none() {
                        // The window starts at the *first* delta of a burst, so a
                        // steady stream flushes every DELTA_FLUSH rather than
                        // being deferred indefinitely by each new fragment.
                        deadline = Some(Instant::now() + DELTA_FLUSH);
                    }
                    pending.push_str(&text);
                }
                Some(event) => {
                    // Order before latency: buffered text precedes the event that
                    // interrupted it.
                    if flush(&mut pending, &sink).is_err() {
                        break;
                    }
                    deadline = None;
                    if sink.send(event).is_err() {
                        break;
                    }
                }
            },
            () = flush_due => {
                if flush(&mut pending, &sink).is_err() {
                    break;
                }
                deadline = None;
            }
        }
    }
}

/// Send buffered text as one `TextDelta` and clear the buffer. A no-op when
/// there is nothing pending, so callers can flush unconditionally.
fn flush<S: EventSink>(pending: &mut String, sink: &S) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let text = std::mem::take(pending);
    sink.send(SessionEvent::TextDelta(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records what it is given, in the order it was given.
    #[derive(Clone, Default)]
    pub struct Recorder(Arc<Mutex<Vec<SessionEvent>>>);

    impl Recorder {
        pub fn events(&self) -> Vec<SessionEvent> {
            self.0.lock().unwrap().clone()
        }
    }

    impl EventSink for Recorder {
        fn send(&self, event: SessionEvent) -> Result<(), String> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn text(s: &str) -> SessionEvent {
        SessionEvent::TextDelta(s.to_owned())
    }

    fn turn_complete() -> SessionEvent {
        SessionEvent::TurnComplete {
            subtype: "success".into(),
            is_error: false,
            total_cost_usd: Some(0.01),
            num_turns: Some(1),
        }
    }

    #[tokio::test]
    async fn deltas_inside_one_window_arrive_as_a_single_event() {
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        for fragment in ["Hel", "lo, ", "wor", "ld"] {
            tx.send(text(fragment)).await.unwrap();
        }
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(
            sink.events(),
            vec![text("Hello, world")],
            "four fragments should have been coalesced into one"
        );
    }

    #[tokio::test]
    async fn deltas_spanning_windows_flush_more_than_once() {
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        tx.send(text("first")).await.unwrap();
        tokio::time::sleep(DELTA_FLUSH * 3).await;
        tx.send(text("second")).await.unwrap();
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(sink.events(), vec![text("first"), text("second")]);
    }

    #[tokio::test]
    async fn a_non_delta_event_flushes_pending_text_before_itself() {
        // The ordering property. If this fails the transcript reads wrongly:
        // the tool card would appear above the sentence that introduced it.
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        tx.send(text("Let me check")).await.unwrap();
        let tool = SessionEvent::ToolUse {
            id: "toolu_01".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        tx.send(tool.clone()).await.unwrap();
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(sink.events(), vec![text("Let me check"), tool]);
    }

    #[tokio::test]
    async fn non_delta_events_are_never_batched_together() {
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        let init = SessionEvent::Init {
            session_id: "s".into(),
            model: "m".into(),
            cli_version: "2.1.220".into(),
            tools: vec![],
        };
        let final_text = SessionEvent::AssistantText("done".into());
        for event in [init.clone(), final_text.clone(), turn_complete()] {
            tx.send(event).await.unwrap();
        }
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(sink.events(), vec![init, final_text, turn_complete()]);
    }

    #[tokio::test]
    async fn text_buffered_when_the_stream_ends_is_not_dropped() {
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        tx.send(text("trailing")).await.unwrap();
        // Close immediately, well inside the flush window.
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(sink.events(), vec![text("trailing")]);
    }

    #[tokio::test]
    async fn interleaved_text_and_events_keep_their_relative_order() {
        let (tx, rx) = mpsc::channel(64);
        let sink = Recorder::default();
        let pumping = tokio::spawn(pump(rx, sink.clone()));

        let tool = SessionEvent::ToolUse {
            id: "t".into(),
            name: "Read".into(),
            input: serde_json::json!({}),
        };
        let result = SessionEvent::ToolResult {
            tool_use_id: "t".into(),
            is_error: false,
            content: serde_json::json!("ok"),
        };
        tx.send(text("a")).await.unwrap();
        tx.send(text("b")).await.unwrap();
        tx.send(tool.clone()).await.unwrap();
        tx.send(text("c")).await.unwrap();
        tx.send(result.clone()).await.unwrap();
        tx.send(text("d")).await.unwrap();
        tx.send(turn_complete()).await.unwrap();
        drop(tx);
        pumping.await.unwrap();

        assert_eq!(
            sink.events(),
            vec![
                text("ab"),
                tool,
                text("c"),
                result,
                text("d"),
                turn_complete(),
            ]
        );
    }
}
