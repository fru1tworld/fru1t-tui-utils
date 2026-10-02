mod app;
mod appearance;
mod diff_view;
mod file_tree;
mod git;
#[cfg(test)]
mod git_tests;
mod matching;
mod refresh;
mod settings;
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
    app::{App, KeyOutcome, TestFiles},
    diff_view::ViewMode,
    git::{Mode, Repository, escape_controls},
    settings::Settings,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const REFRESH_CHECK_WHILE_READING: Duration = Duration::from_millis(16);
const MAX_INPUT_WAIT: Duration = Duration::from_millis(250);

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Review Git diffs with test files hidden by default",
    after_help = "Examples:\n  diff-tui                     Side-by-side working changes against HEAD\n  diff-tui main                Compare main with the current HEAD\n  diff-tui main feature        Compare two branch tips without checkout\n  diff-tui --unified           Start in unified view\n  diff-tui --unstaged          Same comparison as git diff\n  diff-tui --staged            Same comparison as git diff --cached\n  diff-tui -C /path/to/repo     Review another repository\n\nIn the TUI: v switches views, b selects branches, w returns to working changes, t toggles tests, T toggles dark/light theme."
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

impl Cli {
    fn mode(&self) -> Mode {
        match self.revisions.as_slice() {
            [from] => Mode::Branches {
                from: from.clone(),
                to: "HEAD".into(),
            },
            [from, to] => Mode::Branches {
                from: from.clone(),
                to: to.clone(),
            },
            _ if self.staged => Mode::Staged,
            _ if self.unstaged => Mode::Unstaged,
            _ => Mode::Working,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let repo = Repository::open(&cli.repo)?;
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "diff-tui needs an interactive terminal. Run diff-tui --help for usage."
    );
    let test_files = if cli.show_tests {
        TestFiles::Shown
    } else {
        TestFiles::Hidden
    };
    let mut app = App::new(repo, cli.mode(), test_files)?;
    if cli.unified {
        app.display.mode = ViewMode::Unified;
    }
    let settings = Settings::open_default().and_then(|settings| {
        if let Some(appearance) = settings.appearance()? {
            app.set_appearance(appearance);
        }
        Ok(settings)
    });
    let settings = match settings {
        Ok(settings) => Some(settings),
        Err(error) => {
            app.error = Some(escape_controls(&format!("{error:#}")));
            None
        }
    };
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal, &mut app, settings.as_ref());
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    settings: Option<&Settings>,
) -> Result<()> {
    let mut poller = refresh::Poller::new(app.repo.root.clone())?;
    let mut next_poll = Instant::now() + POLL_INTERVAL;
    loop {
        if let Some((request, update)) = poller.take_ready()? {
            app.apply_refresh(request, update);
        }
        if Instant::now() >= next_poll && !poller.is_reading() {
            poller.request(app.refresh_request())?;
            next_poll = Instant::now() + POLL_INTERVAL;
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        let input_wait = if poller.is_reading() {
            REFRESH_CHECK_WHILE_READING
        } else {
            next_poll
                .saturating_duration_since(Instant::now())
                .min(MAX_INPUT_WAIT)
        };
        if event::poll(input_wait)?
            && let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            let appearance = app.display.appearance;
            match app.handle(key) {
                Ok(KeyOutcome::Quit) => return Ok(()),
                Ok(KeyOutcome::Continue) => {}
                Err(error) => app.error = Some(escape_controls(&format!("{error:#}"))),
            }
            if app.display.appearance != appearance
                && let Some(settings) = settings
                && let Err(error) = settings.save_appearance(app.display.appearance)
            {
                app.error = Some(escape_controls(&format!("Cannot save theme: {error:#}")));
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
