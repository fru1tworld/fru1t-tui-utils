mod action;
mod app;
mod cli;
mod clipboard;
mod db;
mod error;
mod ui;

use clap::Parser;
use ratatui::{
    DefaultTerminal,
    crossterm::{
        cursor::SetCursorStyle,
        event::{
            self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags,
            PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::supports_keyboard_enhancement,
    },
};

use action::{Flow, map_key};
use app::App;
use cli::Cli;
use db::Store;
use ui::ui;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        return cli::run(cmd);
    }

    let store = Store::open_default()?;
    let mut app = App::new(store)?;

    let mut terminal = ratatui::init();

    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBar);

    let result = run(&mut terminal, &mut app);

    let _ = execute!(std::io::stdout(), SetCursorStyle::DefaultUserShape);
    if enhanced {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    use std::time::Duration;

    let poll_interval = Duration::from_secs(1);

    loop {
        terminal.draw(|f| ui(f, app))?;

        if !event::poll(poll_interval)? {
            app.sync()?;
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Tab {
            app.tab_held = true;
            app.status = "←→ 메모를 옆 탭으로 보내기 · 다른 키를 누르면 해제".to_string();
            continue;
        }
        if app.tab_held && !matches!(key.code, KeyCode::Left | KeyCode::Right) {
            app.tab_held = false;
        }
        if let Some(action) = map_key(app, key)
            && let Flow::Quit = app.apply(action)?
        {
            return Ok(());
        }
    }
}
