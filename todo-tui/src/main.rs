mod action;
mod app;
mod cli;
mod clipboard;
mod db;
mod dialog;
mod error;
mod tree;
mod ui;
mod undo;

use std::time::Duration;

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        return cli::run(cmd);
    }

    let store = Store::open_default()?;
    let mut app = App::new(store)?;

    let mut terminal = ratatui::init();

    let keyboard_enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhanced {
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let _ = execute!(std::io::stdout(), SetCursorStyle::BlinkingBar);

    let result = run_tui(&mut terminal, &mut app);

    let _ = execute!(std::io::stdout(), SetCursorStyle::DefaultUserShape);
    if keyboard_enhanced {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

fn run_tui(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    const EXTERNAL_CHANGE_POLL_INTERVAL: Duration = Duration::from_secs(1);

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if !event::poll(EXTERNAL_CHANGE_POLL_INTERVAL)? {
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
            app.hold_tab();
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
