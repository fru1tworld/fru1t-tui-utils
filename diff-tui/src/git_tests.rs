use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    app::{App, Dialog},
    git::{LineKind, Mode, Repository},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "diff-tui-{}-{suffix}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        fixture.git(&["init", "-b", "main"]);
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.root)
            .args([
                "-c",
                "user.name=diff-tui tests",
                "-c",
                "user.email=diff-tui@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-m", "fixture"]);
    }

    fn repository(&self) -> Repository {
        Repository::open(&self.root).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn working_staged_and_unstaged_are_distinct() {
    let fixture = Fixture::new();
    fixture.write("src/main.rs", "base\n");
    fixture.write("src/other.rs", "old\n");
    fixture.write("tests/payment.rs", "old test\n");
    fixture.commit();
    fixture.write("src/main.rs", "staged\n");
    fixture.git(&["add", "src/main.rs"]);
    fixture.write("src/other.rs", "unstaged\n");
    fixture.write("tests/payment.rs", "new test\n");
    fixture.write("untracked.rs", "untracked\n");
    let repo = fixture.repository();
    let all = repo.snapshot(&Mode::Working).unwrap();
    assert_eq!(all.changes.len(), 3);
    let staged = repo.snapshot(&Mode::Staged).unwrap();
    assert_eq!(staged.changes.len(), 1);
    assert_eq!(staged.changes[0].path, PathBuf::from("src/main.rs"));
    let unstaged = repo.snapshot(&Mode::Unstaged).unwrap();
    assert_eq!(unstaged.changes.len(), 2);
    assert!(
        unstaged
            .changes
            .iter()
            .all(|change| change.path != Path::new("src/main.rs"))
    );
    let mut app = App::new(repo, Mode::Working, false).unwrap();
    assert_eq!(app.visible.len(), 2);
    assert_eq!(app.hidden_count(), 1);
    app.handle(key('t')).unwrap();
    assert_eq!(app.visible.len(), 3);
    app.handle(key('/')).unwrap();
    for ch in "payment".chars() {
        app.handle(key(ch)).unwrap();
    }
    app.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.visible.len(), 1);
    app.handle(key('t')).unwrap();
    assert!(app.visible.is_empty());
    assert!(app.preview.patch.is_empty());
    app.handle(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.visible.len(), 2);
}

#[test]
fn branch_comparison_uses_both_tips_and_preserves_worktree() {
    let fixture = Fixture::new();
    fixture.write("src/main.rs", "old\n");
    fixture.write("tests/payment.rs", "old test\n");
    fixture.commit();
    fixture.git(&["checkout", "-b", "feature"]);
    fixture.write("src/main.rs", "new\n");
    fixture.write("tests/payment.rs", "new test\n");
    fixture.commit();
    fixture.git(&["checkout", "main"]);
    fixture.write("main-only.txt", "main has diverged\n");
    fixture.commit();
    fixture.write("src/main.rs", "uncommitted, must survive\n");
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let status = fixture.git(&["status", "--porcelain=v1"]);
    let index = fs::read(fixture.root.join(".git/index")).unwrap();
    let repo = fixture.repository();
    let snapshot = repo
        .snapshot(&Mode::Branches("main".into(), "feature".into()))
        .unwrap();
    assert!(
        snapshot
            .changes
            .iter()
            .any(|change| change.path == Path::new("main-only.txt") && change.status == 'D')
    );
    let change = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("src/main.rs"))
        .unwrap();
    let patch = repo.patch(&snapshot, change, false).unwrap();
    assert!(
        patch
            .iter()
            .any(|line| line.kind == LineKind::Added && line.text == "+new")
    );
    assert!(!patch.iter().any(|line| line.text.contains("uncommitted")));
    assert!(
        repo.snapshot(&Mode::Branches("missing-branch".into(), "main".into()))
            .is_err()
    );
    assert!(
        repo.snapshot(&Mode::Branches("--output=bad".into(), "main".into()))
            .is_err()
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fixture.git(&["status", "--porcelain=v1"]), status);
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/main.rs")).unwrap(),
        "uncommitted, must survive\n"
    );

    let mut app = App::new(repo, Mode::Working, false).unwrap();
    app.handle(key('b')).unwrap();
    assert!(matches!(app.dialog, Some(Dialog::Branches(_))));
    for ch in "main".chars() {
        app.handle(key(ch)).unwrap();
    }
    app.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    for ch in "feature".chars() {
        app.handle(key(ch)).unwrap();
    }
    app.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(&app.mode, Mode::Branches(from, to) if from == "main" && to == "feature"));
    assert!(app.dialog.is_none());
    assert_eq!(app.hidden_count(), 1);
    app.handle(key('w')).unwrap();
    assert!(matches!(app.mode, Mode::Working));
}

#[test]
fn unborn_head_uses_empty_tree_without_writing_it() {
    let fixture = Fixture::new();
    let repo = fixture.repository();
    assert!(repo.snapshot(&Mode::Working).unwrap().changes.is_empty());
    fixture.write("main.rs", "staged\n");
    fixture.git(&["add", "main.rs"]);
    fixture.write("main.rs", "latest working content\n");
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    assert_eq!(snapshot.changes.len(), 1);
    let patch = repo.patch(&snapshot, &snapshot.changes[0], false).unwrap();
    assert!(
        patch
            .iter()
            .any(|line| line.text == "+latest working content")
    );
    let index = repo.snapshot(&Mode::Staged).unwrap();
    assert_eq!(index.changes.len(), 1);
}

#[test]
fn renamed_deleted_binary_and_literal_paths_have_previews() {
    let fixture = Fixture::new();
    fixture.write("src/old.rs", "unchanged\n");
    fixture.write("src/deleted.rs", "gone\n");
    fixture.write("src/data.bin", b"\0old");
    fixture.write("src/[literal].rs", "old literal\n");
    fixture.write("src/l.rs", "must not appear in literal preview\n");
    fixture.commit();
    fixture.git(&["mv", "src/old.rs", "src/new name.rs"]);
    fs::remove_file(fixture.root.join("src/deleted.rs")).unwrap();
    fixture.write("src/data.bin", b"\0new");
    fixture.write("src/[literal].rs", "new literal\n");
    fixture.write("src/l.rs", "other change\n");
    let repo = Repository::open(&fixture.root.join("src")).unwrap();
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    let renamed = snapshot
        .changes
        .iter()
        .find(|change| change.status == 'R')
        .unwrap();
    assert_eq!(renamed.path, PathBuf::from("src/new name.rs"));
    assert!(
        repo.patch(&snapshot, renamed, false)
            .unwrap()
            .iter()
            .any(|line| line.text.starts_with("rename to"))
    );
    let binary = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("src/data.bin"))
        .unwrap();
    assert!(
        repo.patch(&snapshot, binary, false)
            .unwrap()
            .iter()
            .any(|line| line.text.contains("Binary files"))
    );
    let literal = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("src/[literal].rs"))
        .unwrap();
    let patch = repo.patch(&snapshot, literal, false).unwrap();
    assert!(patch.iter().any(|line| line.text == "+new literal"));
    assert!(!patch.iter().any(|line| line.text.contains("other change")));
    assert!(snapshot.changes.iter().any(|change| change.status == 'D'));
}

fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

#[test]
fn highlighting_sources_follow_each_comparison_and_rename() {
    let fixture = Fixture::new();
    fixture.write("src/main.rs", "base\n");
    fixture.commit();
    fixture.write("src/main.rs", "staged\n");
    fixture.git(&["add", "src/main.rs"]);
    fixture.write("src/main.rs", "working\n");
    let repo = fixture.repository();
    for (mode, old, new) in [
        (Mode::Working, "base\n", "working\n"),
        (Mode::Staged, "base\n", "staged\n"),
        (Mode::Unstaged, "staged\n", "working\n"),
    ] {
        let snapshot = repo.snapshot(&mode).unwrap();
        let sources = repo.sources(&snapshot, &snapshot.changes[0]).unwrap();
        assert_eq!(sources.old.as_deref(), Some(old));
        assert_eq!(sources.new.as_deref(), Some(new));
    }
    fixture.commit();
    fixture.git(&["mv", "src/main.rs", "src/renamed.rs"]);
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    let sources = repo.sources(&snapshot, &snapshot.changes[0]).unwrap();
    assert_eq!(sources.old.as_deref(), Some("working\n"));
    assert_eq!(sources.new.as_deref(), Some("working\n"));
}

fn refresh(app: &mut App) {
    let request = app.refresh_request();
    let update = request.read(&app.repo);
    app.apply_refresh(request, update);
    assert!(app.refresh_error.is_none(), "{:?}", app.refresh_error);
}

#[test]
fn polling_preserves_review_position_and_reuses_unchanged_layout() {
    use crate::app::Focus;

    let fixture = Fixture::new();
    let content = |prefix: &str| {
        (0..80)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    fixture.write("src/a.rs", "base\n");
    fixture.write("src/b.rs", content("base"));
    fixture.commit();
    fixture.write("src/b.rs", content("first"));
    let mut app = App::new(fixture.repository(), Mode::Working, false).unwrap();
    app.layout_diff(120, 10);
    app.scroll = 25;
    app.horizontal = 8;
    app.focus = Focus::Diff;
    app.query = "src/".into();
    app.searching = true;
    app.dialog = Some(Dialog::Help);
    let rows = app.display.rows.as_ptr();
    let anchor = app.display.anchor(app.scroll);
    let old_line = app.preview.patch[anchor].old;
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let index = fs::read(fixture.root.join(".git/index")).unwrap();

    refresh(&mut app);
    assert_eq!(app.display.rows.as_ptr(), rows);
    assert_eq!(app.scroll, 25);
    fixture.write("src/a.rs", "new change before selected file\n");
    fixture.write("src/b.rs", content("second"));
    refresh(&mut app);
    app.layout_diff(120, 10);
    assert_eq!(app.selected().unwrap().path, Path::new("src/b.rs"));
    assert_eq!(app.selected_file, Some(1));
    assert_eq!(app.scroll, 25);
    assert_eq!(
        app.preview.patch[app.display.anchor(app.scroll)].old,
        old_line
    );
    assert_eq!(app.horizontal, 8);
    assert!(app.focus == Focus::Diff);
    assert_eq!(app.query, "src/");
    assert!(app.searching);
    assert!(matches!(app.dialog, Some(Dialog::Help)));
    assert!(
        app.preview
            .patch
            .iter()
            .any(|line| line.text == "+second 40")
    );

    let rows = app.display.rows.as_ptr();
    app.handle(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.searching = false;
    app.handle(key('r')).unwrap();
    assert_eq!(app.display.rows.as_ptr(), rows);
    assert_eq!(app.scroll, 25);
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/b.rs")).unwrap(),
        content("second")
    );
}

#[test]
fn polling_tracks_new_and_disappearing_changes_with_test_filter() {
    let fixture = Fixture::new();
    fixture.write("src/a.rs", "base\n");
    fixture.write("src/b.rs", "base\n");
    fixture.write("tests/a.rs", "base\n");
    fixture.commit();
    let mut app = App::new(fixture.repository(), Mode::Working, false).unwrap();
    assert!(app.visible.is_empty());
    fixture.write("tests/a.rs", "test change\n");
    fixture.write("src/a.rs", "new a\n");
    fixture.write("src/b.rs", "new b\n");
    refresh(&mut app);
    assert_eq!(app.visible.len(), 2);
    assert_eq!(app.hidden_count(), 1);
    assert_eq!(app.selected().unwrap().path, Path::new("src/a.rs"));
    fixture.write("src/a.rs", "base\n");
    refresh(&mut app);
    assert_eq!(app.selected().unwrap().path, Path::new("src/b.rs"));
    assert!(app.preview.patch.iter().any(|line| line.text == "+new b"));
    fixture.write("src/b.rs", "base\n");
    refresh(&mut app);
    assert!(app.visible.is_empty());
    assert!(app.preview.patch.is_empty());
    assert!(app.display.rows.is_empty());
}

#[test]
fn polling_tracks_index_updates_and_discards_superseded_results() {
    let fixture = Fixture::new();
    fixture.write("src/a.rs", "base\n");
    fixture.write("src/b.rs", "base\n");
    fixture.commit();
    fixture.write("src/a.rs", "staged a\n");
    fixture.write("src/b.rs", "staged b\n");
    fixture.git(&["add", "--all"]);
    fixture.write("src/a.rs", "working a\n");
    let mut app = App::new(fixture.repository(), Mode::Staged, false).unwrap();
    fixture.git(&["add", "src/a.rs"]);
    refresh(&mut app);
    assert!(
        app.preview
            .patch
            .iter()
            .any(|line| line.text == "+working a")
    );

    let request = app.refresh_request();
    let update = request.read(&app.repo);
    app.handle(key('n')).unwrap();
    app.apply_refresh(request, update);
    assert_eq!(app.selected().unwrap().path, Path::new("src/b.rs"));
    assert!(
        app.preview
            .patch
            .iter()
            .any(|line| line.text == "+staged b")
    );

    let request = app.refresh_request();
    let update = request.read(&app.repo);
    app.handle(key('u')).unwrap();
    app.apply_refresh(request, update);
    assert!(matches!(app.mode, Mode::Unstaged));
    assert!(app.visible.is_empty());
    fixture.write("src/a.rs", "latest working a\n");
    refresh(&mut app);
    assert!(
        app.preview
            .patch
            .iter()
            .any(|line| line.text == "-working a")
    );
    assert!(
        app.preview
            .patch
            .iter()
            .any(|line| line.text == "+latest working a")
    );
}

#[test]
fn polling_follows_branch_tips_and_recovers_from_missing_ref() {
    let fixture = Fixture::new();
    fixture.write("src/a.rs", "base\n");
    fixture.commit();
    fixture.git(&["checkout", "-b", "feature"]);
    fixture.write("src/a.rs", "first\n");
    fixture.commit();
    let mut app = App::new(
        fixture.repository(),
        Mode::Branches("main".into(), "feature".into()),
        false,
    )
    .unwrap();
    fixture.write("src/a.rs", "second\n");
    fixture.commit();
    refresh(&mut app);
    assert!(app.preview.patch.iter().any(|line| line.text == "+second"));
    fixture.git(&["checkout", "main"]);
    let tip = fixture.git(&["rev-parse", "feature"]);
    fixture.git(&["branch", "-D", "feature"]);
    let request = app.refresh_request();
    let update = request.read(&app.repo);
    app.apply_refresh(request, update);
    assert!(app.refresh_error.is_some());
    assert!(app.preview.patch.iter().any(|line| line.text == "+second"));
    fixture.git(&["branch", "feature", tip.trim()]);
    refresh(&mut app);
    assert!(app.preview.patch.iter().any(|line| line.text == "+second"));
}

#[test]
fn tab_switches_preview_and_full_screen_and_keeps_directory_navigation() {
    use crate::{app::Focus, ui};
    use ratatui::{Terminal, backend::TestBackend};

    let fixture = Fixture::new();
    let path = "subproject/application/src/main/kotlin/billing/Service.kt";
    let other = "subproject/application/src/main/kotlin/checkout/Service.kt";
    let content = |prefix: &str| {
        (0..60)
            .map(|i| format!("{prefix} {i}\n"))
            .collect::<String>()
    };
    fixture.write(path, content("base"));
    fixture.write(other, "base\n");
    fixture.commit();
    fixture.write(path, content("changed"));
    fixture.write(other, "changed\n");
    let mut app = App::new(fixture.repository(), Mode::Working, false).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    let render = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let text = render(&mut app, &mut terminal);
    assert!(text.contains(" Files "));
    assert!(text.contains("subproject/application/src/main/kotlin/"));
    assert!(text.contains("Before | Kotlin"));
    assert!(!text.contains("..."));
    let selected = app.tree.current().unwrap().path.clone();
    app.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let text = render(&mut app, &mut terminal);
    assert!(text.contains("Before | Kotlin"));
    assert!(text.contains("After | Kotlin"));
    assert!(!text.contains(" Files "));
    assert!(!text.contains("diff --git"));
    assert!(!text.contains("--- a/"));
    assert!(!text.contains("+++ b/"));
    app.scroll = 8;
    app.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.focus == Focus::Files);
    assert_eq!(app.tree.current().unwrap().path, selected);
    app.handle(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.tree.current().unwrap().path,
        Path::new(path).parent().unwrap()
    );
    app.handle(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert!(app.tree.current().unwrap().collapsed);
    refresh(&mut app);
    assert!(app.tree.current().unwrap().collapsed);
    app.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    render(&mut app, &mut terminal);
    assert_eq!(app.scroll, 8);
    app.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.tree.current().unwrap().collapsed);
    app.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.tree.current().unwrap().collapsed);
}
