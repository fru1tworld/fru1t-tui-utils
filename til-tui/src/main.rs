mod action;
mod app;
mod cli;
mod clipboard;
mod db;
mod domain;
mod error;
mod output;
mod ui;

use std::io::{self, Write};
use std::time::{Duration, Instant};

use action::{Flow, map_key};
use app::App;
use clap::Parser;
use db::SqliteTilRepository;
use ratatui::{
    DefaultTerminal,
    crossterm::{
        cursor::SetCursorStyle,
        event::{
            self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
            PushKeyboardEnhancementFlags,
        },
        execute,
    },
};

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    if let Some(command) = cli.command {
        return cli::run(command);
    }

    let repository = SqliteTilRepository::open_default()?;
    let mut app = App::new(repository)?;
    let mut terminal = ratatui::init();
    let _ = execute!(
        io::stdout(),
        SetCursorStyle::BlinkingBar,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );

    let result = run(&mut terminal, &mut app);

    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        SetCursorStyle::DefaultUserShape
    );
    ratatui::restore();

    if let Some(text) = result? {
        print!("{text}");
        io::stdout().flush()?;
    }
    Ok(())
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<Option<String>> {
    let sync_interval = Duration::from_secs(1);
    let mut last_sync = Instant::now();

    loop {
        if last_sync.elapsed() >= sync_interval {
            app.sync_external_changes()?;
            last_sync = Instant::now();
        }
        terminal.draw(|frame| ui::ui(frame, app))?;

        let wait = sync_interval.saturating_sub(last_sync.elapsed());
        if !event::poll(wait)? {
            app.sync_external_changes()?;
            last_sync = Instant::now();
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if let Some(action) = map_key(app, key) {
            match app.apply(action)? {
                Flow::Continue => {}
                Flow::Quit => return Ok(None),
                Flow::Output(text) => return Ok(Some(text)),
            }
        }
    }
}
