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
    about = "테스트 파일을 기본으로 숨기고 Git diff를 리뷰합니다",
    after_help = "예시:\n  diff-tui                     HEAD 대비 작업 중인 변경을 나란히 보기\n  diff-tui main                main과 현재 HEAD 비교\n  diff-tui main feature        체크아웃 없이 두 브랜치 끝 비교\n  diff-tui --unified           통합 보기로 시작\n  diff-tui --unstaged          git diff와 같은 비교\n  diff-tui --staged            git diff --cached와 같은 비교\n  diff-tui -C /path/to/repo    다른 저장소 리뷰\n\nTUI 안에서: v 보기 전환, b 브랜치 선택, w 작업 중인 변경으로 복귀, t 테스트 표시 전환, T 다크/라이트 테마."
)]
struct Cli {
    /// 비교할 브랜치, 태그 또는 커밋 (TO 기본값은 HEAD)
    #[arg(value_names = ["FROM", "TO"], num_args = 1..=2)]
    revisions: Vec<String>,
    /// 저장소 디렉터리 (기본값은 현재 디렉터리)
    #[arg(short = 'C', long, default_value = ".")]
    repo: PathBuf,
    /// HEAD와 index 비교
    #[arg(long, conflicts_with_all = ["unstaged", "revisions"])]
    staged: bool,
    /// index와 작업 트리 비교
    #[arg(long, conflicts_with = "revisions")]
    unstaged: bool,
    /// 테스트 파일을 보이게 하고 시작
    #[arg(short = 't', long)]
    show_tests: bool,
    /// 기본 나란히 보기 대신 통합 보기로 시작
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
        "diff-tui는 대화형 터미널이 필요합니다. 사용법은 diff-tui --help를 참고하세요."
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
                app.error = Some(escape_controls(&format!(
                    "테마를 저장할 수 없습니다: {error:#}"
                )));
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
