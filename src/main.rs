//! epik-app: a terminal chat interface to Claude Code.
//!
//! Usage: epik-app [--model MODEL] [--settings JSON] [--mcp-config PATH]
//!                 [--append-system-prompt TEXT] [--permission-mode MODE]
//!                 [--persona-file PATH] [--greet TEXT]
//!        epik-app doctor    # locate + version-check the claude engine

use std::io::stdout;

use anyhow::bail;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use epik_app::session::{Session, SessionConfig};
use epik_app::tui::App;

fn parse_args() -> anyhow::Result<(SessionConfig, Option<String>)> {
    let mut config = SessionConfig::default();
    let mut greet = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))
        };
        match arg.as_str() {
            "--model" => config.model = Some(value("--model")?),
            "--settings" => config.settings_json = Some(value("--settings")?),
            "--mcp-config" => config.mcp_config = Some(value("--mcp-config")?),
            "--append-system-prompt" => {
                config.append_system_prompt = Some(value("--append-system-prompt")?);
            }
            "--permission-mode" => config.permission_mode = Some(value("--permission-mode")?),
            "--persona-file" => {
                let path = value("--persona-file")?;
                let persona = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("cannot read persona file {path}: {e}"))?;
                config.append_system_prompt = Some(match config.append_system_prompt.take() {
                    Some(existing) => format!("{existing}\n\n{persona}"),
                    None => persona,
                });
            }
            "--greet" => greet = Some(value("--greet")?),
            other => bail!("unknown argument: {other}"),
        }
    }
    Ok((config, greet))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("doctor") {
        return epik_app::doctor::run_doctor();
    }

    let (mut config, greet) = parse_args()?;
    // Resolve the engine up front so a missing/old CLI fails with the doctor
    // message instead of a bare spawn error.
    let engine = epik_app::doctor::inspect();
    match engine.path {
        Some(path) if engine.supported => config.claude_bin = path.display().to_string(),
        _ => return epik_app::doctor::run_doctor(),
    }
    let session = Session::spawn(config).await?;
    let greeted = if let Some(text) = greet {
        session.handle().send_user(&text).await?;
        true
    } else {
        false
    };

    terminal::enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = App::new(session, greeted).run(&mut terminal).await;

    terminal::disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    result
}
