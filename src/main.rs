//! epik-app: a terminal chat interface to Claude Code.
//!
//! Usage: epik-app [--model MODEL] [--settings JSON] [--mcp-config PATH]
//!                 [--append-system-prompt TEXT] [--permission-mode MODE]

use std::io::stdout;

use anyhow::bail;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use epik_app::session::{Session, SessionConfig};
use epik_app::tui::App;

fn parse_args() -> anyhow::Result<SessionConfig> {
    let mut config = SessionConfig::default();
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
            other => bail!("unknown argument: {other}"),
        }
    }
    Ok(config)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = parse_args()?;
    let session = Session::spawn(config).await?;

    terminal::enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = App::new(session).run(&mut terminal).await;

    terminal::disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    result
}
