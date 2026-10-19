mod action;
mod app;
mod cli;
mod db;
mod dialog;
mod domain;
mod error;
mod tree;
mod ui;

use std::time::Duration;

use clap::Parser;
use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEventKind},
};

use action::Flow;
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
    tui_kit::terminal::run(|terminal| run_tui(terminal, &mut app))?
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
        if let Flow::Quit = app.handle_key(key)? {
            return Ok(());
        }
    }
}
