//! M2 — minimal ratatui chat over a [`Session`].
//!
//! Layout: transcript (scrolls, follows tail) / status line / input box.
//! Keys: Enter send · Esc interrupt · Ctrl+C quit · y/n answer a pending
//! permission ask (input typing is suspended while an ask is pending).

use std::io::Stdout;

use anyhow::Context;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::protocol::{CanUseToolRequest, PermissionDecision};
use crate::session::{Session, SessionEvent};

/// One transcript entry. `Assistant` accumulates streaming deltas until the
/// complete block arrives and replaces it.
enum Entry {
    User(String),
    Assistant(String),
    Tool {
        label: String,
        outcome: Option<bool>,
    },
    Info(String),
}

pub struct App {
    session: Session,
    transcript: Vec<Entry>,
    input: String,
    /// Streaming text for the assistant turn in flight, if any.
    streaming: Option<String>,
    pending_ask: Option<(String, CanUseToolRequest)>,
    session_id: String,
    model: String,
    cost_total: f64,
    busy: bool,
    quit: bool,
}

impl App {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            transcript: vec![Entry::Info(
                "Enter: send · Esc: interrupt · Ctrl+C: quit".to_owned(),
            )],
            input: String::new(),
            streaming: None,
            pending_ask: None,
            session_id: String::new(),
            model: String::new(),
            cost_total: 0.0,
            busy: false,
            quit: false,
        }
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> anyhow::Result<()> {
        let mut term_events = EventStream::new();
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            tokio::select! {
                ev = term_events.next() => {
                    match ev {
                        Some(Ok(Event::Key(key))) => self.on_key(key).await?,
                        Some(Ok(_)) => {} // resize etc: redraw on next loop
                        Some(Err(err)) => return Err(err).context("terminal event stream"),
                        None => break,
                    }
                }
                ev = self.session.events.recv() => {
                    match ev {
                        Some(event) => self.on_session_event(event),
                        None => break,
                    }
                }
            }
            if self.quit {
                break;
            }
        }
        Ok(())
    }

    async fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        // Answer a pending permission ask first; it captures y/n.
        if let Some((request_id, request)) = self.pending_ask.take() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.session
                        .handle()
                        .respond_permission(
                            &request_id,
                            &PermissionDecision::Allow {
                                updated_input: None,
                            },
                        )
                        .await?;
                    self.transcript
                        .push(Entry::Info(format!("allowed {}", request.tool_name)));
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.session
                        .handle()
                        .respond_permission(
                            &request_id,
                            &PermissionDecision::Deny {
                                message: "Denied by the user in epik-app.".to_owned(),
                            },
                        )
                        .await?;
                    self.transcript
                        .push(Entry::Info(format!("denied {}", request.tool_name)));
                    return Ok(());
                }
                _ => {
                    // Not an answer: keep the ask pending, fall through so
                    // Esc/Ctrl+C still work.
                    self.pending_ask = Some((request_id, request));
                }
            }
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.session.handle().shutdown().await.ok();
                self.quit = true;
            }
            (KeyCode::Esc, _) => {
                if self.busy {
                    self.session.handle().interrupt().await?;
                    self.transcript
                        .push(Entry::Info("interrupt sent".to_owned()));
                }
            }
            (KeyCode::Enter, _) => {
                let text = self.input.trim().to_owned();
                if !text.is_empty() && !self.busy && self.pending_ask.is_none() {
                    self.session.handle().send_user(&text).await?;
                    self.transcript.push(Entry::User(text));
                    self.input.clear();
                    self.busy = true;
                }
            }
            (KeyCode::Backspace, _) => {
                if self.pending_ask.is_none() {
                    self.input.pop();
                }
            }
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT)
                if self.pending_ask.is_none() =>
            {
                self.input.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Init {
                session_id, model, ..
            } => {
                self.session_id = session_id;
                self.model = model;
            }
            SessionEvent::TextDelta(delta) => {
                self.streaming.get_or_insert_default().push_str(&delta);
            }
            SessionEvent::AssistantText(text) => {
                self.streaming = None;
                self.transcript.push(Entry::Assistant(text));
            }
            SessionEvent::ToolUse { name, input, .. } => {
                let detail = input
                    .get("command")
                    .or_else(|| input.get("file_path"))
                    .or_else(|| input.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut label = format!("{name} {detail}");
                label.truncate(120);
                self.transcript.push(Entry::Tool {
                    label,
                    outcome: None,
                });
            }
            SessionEvent::ToolResult { is_error, .. } => {
                if let Some(Entry::Tool { outcome, .. }) = self
                    .transcript
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, Entry::Tool { outcome: None, .. }))
                {
                    *outcome = Some(!is_error);
                }
            }
            SessionEvent::PermissionRequest {
                request_id,
                request,
            } => {
                self.pending_ask = Some((request_id, request));
            }
            SessionEvent::ControlAck { .. } => {}
            SessionEvent::TurnComplete {
                subtype,
                is_error,
                total_cost_usd,
                ..
            } => {
                self.busy = false;
                self.streaming = None;
                self.cost_total += total_cost_usd.unwrap_or(0.0);
                if is_error {
                    self.transcript
                        .push(Entry::Info(format!("turn ended: {subtype}")));
                }
            }
            SessionEvent::Unknown(_) => {}
            SessionEvent::Closed { .. } => {
                self.transcript
                    .push(Entry::Info("session closed".to_owned()));
                self.quit = true;
            }
        }
    }

    fn draw(&self, frame: &mut ratatui::Frame) {
        let [chat_area, status_area, input_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .areas(frame.area());

        // Transcript, following the tail.
        let mut lines: Vec<Line> = Vec::new();
        for entry in &self.transcript {
            match entry {
                Entry::User(t) => {
                    for (i, l) in t.lines().enumerate() {
                        let prefix = if i == 0 { "you ❯ " } else { "      " };
                        lines.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(Color::Cyan)),
                            Span::raw(l.to_owned()),
                        ]));
                    }
                }
                Entry::Assistant(t) => {
                    for l in t.lines() {
                        lines.push(Line::from(l.to_owned()));
                    }
                    lines.push(Line::default());
                }
                Entry::Tool { label, outcome } => {
                    let (mark, color) = match outcome {
                        None => ("⚙ ", Color::Yellow),
                        Some(true) => ("✓ ", Color::Green),
                        Some(false) => ("✗ ", Color::Red),
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{mark}{label}"),
                        Style::default().fg(color),
                    )));
                }
                Entry::Info(t) => lines.push(Line::from(Span::styled(
                    format!("· {t}"),
                    Style::default().fg(Color::DarkGray),
                ))),
            }
        }
        if let Some(streaming) = &self.streaming {
            for l in streaming.lines() {
                lines.push(Line::from(Span::styled(
                    l.to_owned(),
                    Style::default().add_modifier(Modifier::ITALIC),
                )));
            }
        }
        // Rough tail-follow: count wrapped rows at the current width.
        let width = chat_area.width.saturating_sub(2).max(1) as usize;
        let total_rows: usize = lines
            .iter()
            .map(|l| (l.width().max(1)).div_ceil(width))
            .sum();
        let viewport = chat_area.height.saturating_sub(2) as usize;
        let scroll = total_rows.saturating_sub(viewport) as u16;
        frame.render_widget(
            Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title("epik-app"))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            chat_area,
        );

        // Status line.
        let status = if let Some((_, ask)) = &self.pending_ask {
            let detail = ask
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Line::from(Span::styled(
                format!(" permission: {} {} — allow? [y/n]", ask.tool_name, detail),
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ))
        } else {
            let state = if self.busy { "thinking…" } else { "idle" };
            Line::from(Span::styled(
                format!(
                    " {} · {} · ${:.4} · {}",
                    self.model,
                    &self.session_id.chars().take(8).collect::<String>(),
                    self.cost_total,
                    state,
                ),
                Style::default().fg(Color::DarkGray),
            ))
        };
        frame.render_widget(Paragraph::new(status), status_area);

        // Input box.
        frame.render_widget(
            Paragraph::new(self.input.as_str())
                .block(Block::default().borders(Borders::ALL).title("message")),
            input_area,
        );
        let cursor_x = input_area.x + 1 + self.input.chars().count() as u16;
        frame.set_cursor_position((cursor_x.min(input_area.right() - 2), input_area.y + 1));
    }
}
