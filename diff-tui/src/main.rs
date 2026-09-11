mod app;
mod diff_view;
mod file_tree;
mod git;
#[cfg(test)]
mod git_tests;
mod matching;
mod refresh;
mod syntax;
mod ui;
mod wrap;

use anyhow::{Result, ensure};
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    app::App,
    git::{Mode, Repository},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Review Git diffs with test files hidden by default",
    after_help = "Examples:\n  diff-tui                     Side-by-side working changes against HEAD\n  diff-tui main                Compare main with the current HEAD\n  diff-tui main feature        Compare two branch tips without checkout\n  diff-tui --unified           Start in unified view\n  diff-tui --unstaged          Same comparison as git diff\n  diff-tui --staged            Same comparison as git diff --cached\n  diff-tui -C /path/to/repo     Review another repository\n\nIn the TUI: v switches views, b selects branches, w returns to working changes, t toggles tests."
)]
struct Cli {
    /// Branches, tags, or commits to compare; TO defaults to HEAD
    #[arg(value_names = ["FROM", "TO"], num_args = 1..=2)]
    revisions: Vec<String>,
    /// Repository directory; defaults to the current directory
    #[arg(short = 'C', long, default_value = ".")]
    repo: PathBuf,
    /// Compare HEAD with the index
    #[arg(long, conflicts_with_all = ["unstaged", "revisions"])]
    staged: bool,
    /// Compare the index with the working tree
    #[arg(long, conflicts_with = "revisions")]
    unstaged: bool,
    /// Start with test files visible
    #[arg(short = 't', long)]
    show_tests: bool,
    /// Start in unified view instead of the default side-by-side view
    #[arg(long)]
    unified: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mode = match cli.revisions.as_slice() {
        [from] => Mode::Branches(from.clone(), "HEAD".into()),
        [from, to] => Mode::Branches(from.clone(), to.clone()),
        _ if cli.staged => Mode::Staged,
        _ if cli.unstaged => Mode::Unstaged,
        _ => Mode::Working,
    };
    let repo = Repository::open(&cli.repo)?;
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "diff-tui needs an interactive terminal. Run diff-tui --help for usage."
    );
    let mut app = App::new(repo, mode, cli.show_tests)?;
    if cli.unified {
        app.display.mode = diff_view::ViewMode::Unified;
    }
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut poller = refresh::Poller::new(app.repo.root.clone())?;
    let interval = Duration::from_secs(1);
    let mut next_poll = Instant::now() + interval;
    loop {
        if let Some((request, update)) = poller.take_ready()? {
            app.apply_refresh(request, update);
        }
        if Instant::now() >= next_poll && !poller.running {
            poller.request(app.refresh_request())?;
            next_poll = Instant::now() + interval;
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => match app.handle(key) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(error) => app.error = Some(git::clean(&format!("{error:#}"))),
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_arguments_are_unambiguous() {
        assert!(Cli::try_parse_from(["diff-tui"]).is_ok());
        assert!(Cli::try_parse_from(["diff-tui", "main", "feature"]).is_ok());
        assert!(Cli::try_parse_from(["diff-tui", "main"]).is_ok());
        assert!(Cli::try_parse_from(["diff-tui", "main", "feature", "third"]).is_err());
        assert!(Cli::try_parse_from(["diff-tui", "--staged", "main", "feature"]).is_err());
        assert!(Cli::try_parse_from(["diff-tui", "--staged", "--unstaged"]).is_err());
    }
}
