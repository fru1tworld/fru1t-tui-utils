use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    app::{App, Dialog, TestFiles},
    diff_view::{DiffDisplay, Indentation},
    git::{Change, ChangeStatus, LineKind, Mode, Repository, Snapshot, Whitespace},
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
    fixture.write("tests/example.rs", "old test\n");
    fixture.commit();
    fixture.write("src/main.rs", "staged\n");
    fixture.git(&["add", "src/main.rs"]);
    fixture.write("src/other.rs", "unstaged\n");
    fixture.write("tests/example.rs", "new test\n");
    fixture.write("untracked.rs", "untracked\n");
    let repo = fixture.repository();
    let all = repo.snapshot(&Mode::Working).unwrap();
    assert_eq!(all.changes.len(), 4);
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
    let mut app = App::new(repo, Mode::Working, TestFiles::Hidden).unwrap();
    assert_eq!(app.visible.len(), 3);
    assert_eq!(app.hidden_test_count(), 1);
    app.handle(key('t')).unwrap();
    assert_eq!(app.visible.len(), 4);
    app.handle(key('/')).unwrap();
    for ch in "example".chars() {
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
    assert_eq!(app.visible.len(), 3);
}

#[test]
fn untracked_files_render_without_changing_the_index() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "ignored/\n");
    fixture.commit();
    fixture.write("ignored/secret.txt", "ignored\n");
    fixture.write("src/new file.rs", "first\nsecond");
    fixture.write("empty.txt", "");
    fixture.write("binary.dat", b"\0binary");
    let index = fs::read(fixture.root.join(".git/index")).unwrap();
    let repo = fixture.repository();
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    assert_eq!(snapshot.changes.len(), 3);
    for change in &snapshot.changes {
        assert_eq!(change.status, ChangeStatus::Untracked);
        let patch = repo.patch(&snapshot, change, Whitespace::Compared).unwrap();
        match change.path.to_str().unwrap() {
            "src/new file.rs" => {
                let added: Vec<_> = patch
                    .iter()
                    .filter(|line| line.kind == LineKind::Added)
                    .collect();
                assert_eq!(added.len(), 2);
                assert_eq!(added[0].text, "+first");
                assert_eq!(added[1].new, Some(2));
                let sources = repo.sources(&snapshot, change).unwrap();
                assert!(sources.old.is_none());
                assert_eq!(sources.new.as_deref(), Some("first\nsecond"));
            }
            "binary.dat" => assert!(patch.iter().any(|line| line.text.contains("Binary files"))),
            "empty.txt" => assert!(!patch.iter().any(|line| line.kind == LineKind::Added)),
            path => panic!("Unexpected file: {path}"),
        }
    }
    assert!(
        repo.snapshot(&Mode::Branches {
            from: "HEAD".into(),
            to: "HEAD".into(),
        })
        .unwrap()
        .changes
        .is_empty()
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
}

#[test]
fn polling_tracks_untracked_files_and_staging_with_test_filter() {
    let fixture = Fixture::new();
    let mut app = App::new(fixture.repository(), Mode::Working, TestFiles::Hidden).unwrap();
    fixture.write("src/new.rs", "first\n");
    fixture.write("tests/new.rs", "test\n");
    refresh(&mut app);
    assert_eq!(app.visible.len(), 1);
    assert_eq!(app.hidden_test_count(), 1);
    assert_eq!(app.selected().unwrap().status, ChangeStatus::Untracked);
    assert!(app.preview.patch.iter().any(|line| line.text == "+first"));
    fixture.write("src/new.rs", "second\n");
    refresh(&mut app);
    assert!(app.preview.patch.iter().any(|line| line.text == "+second"));
    fixture.git(&["add", "src/new.rs"]);
    refresh(&mut app);
    assert_eq!(app.visible.len(), 1);
    assert_eq!(app.selected().unwrap().status, ChangeStatus::Added);
    assert!(app.preview.patch.iter().any(|line| line.text == "+second"));
    fixture.git(&["rm", "--cached", "src/new.rs"]);
    fs::remove_file(fixture.root.join("src/new.rs")).unwrap();
    refresh(&mut app);
    assert!(app.visible.is_empty());
    assert!(app.preview.patch.is_empty());
}

#[test]
fn staged_deletion_with_recreated_file_is_not_duplicated() {
    let fixture = Fixture::new();
    fixture.write("file.txt", "base\n");
    fixture.commit();
    fixture.git(&["rm", "--cached", "file.txt"]);
    fixture.write("file.txt", "replacement\n");
    let repo = fixture.repository();
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    assert_eq!(snapshot.changes.len(), 1);
    let patch = repo
        .patch(&snapshot, &snapshot.changes[0], Whitespace::Compared)
        .unwrap();
    assert!(patch.iter().any(|line| line.text == "+replacement"));
}

#[test]
fn hunk_headers_find_language_specific_declarations() {
    let fixture = Fixture::new();
    let cases = [
        (
            "client.kt",
            "class Client {\n    private fun approve(\n        amount: Int,\n    ) = effect {",
            "        send(\"before\")",
            "    }\n}",
            "private fun approve(",
        ),
        (
            "client.kts",
            "suspend fun Client.approve() {",
            "    send(\"before\")",
            "}",
            "suspend fun Client.approve() {",
        ),
        (
            "client.rs",
            "impl Client {\n    pub(crate) async unsafe fn approve(&self) {",
            "        send(\"before\");",
            "    }\n}",
            "pub(crate) async unsafe fn approve(&self) {",
        ),
        (
            "client.scala",
            "class Client {\n  private[core] def approve(amount: Int) = {",
            "    send(\"before\")",
            "  }\n}",
            "private[core] def approve(amount: Int) = {",
        ),
        (
            "Client.java",
            "class Client {\n    public void approve(\n        int amount\n    ) {",
            "        send(\"before\");",
            "    }\n}",
            "public void approve(",
        ),
        (
            "Client.groovy",
            "class Client {\n    def approve(amount) {",
            "        send(\"before\")",
            "    }\n}",
            "def approve(amount) {",
        ),
        (
            "client.py",
            "class Client:\n    async def approve(self):",
            "        await send(\"before\")",
            "",
            "async def approve(self):",
        ),
        (
            "client.go",
            "package client\nfunc (c *Client) Approve() {",
            "    send(\"before\")",
            "}",
            "func (c *Client) Approve() {",
        ),
        (
            "function.js",
            "export async function approve(amount) {",
            "    send(\"before\");",
            "}",
            "export async function approve(amount) {",
        ),
        (
            "arrow.ts",
            "export const approve = async (amount: number) => {",
            "    send(\"before\");",
            "};",
            "export const approve = async (amount: number) => {",
        ),
        (
            "method.ts",
            "class Client {\n    public async approve(amount: number): Promise<void> {",
            "        send(\"before\");",
            "    }\n}",
            "public async approve(amount: number): Promise<void> {",
        ),
        (
            "client.tsx",
            "export function Client() {",
            "    send(\"before\");",
            "    return <div />;\n}",
            "export function Client() {",
        ),
        (
            "client.rb",
            "class Client\n  def approve(amount)",
            "    send(\"before\")",
            "  end\nend",
            "def approve(amount)",
        ),
        (
            "client.php",
            "<?php\nclass Client {\n    public function approve($amount) {",
            "        send(\"before\");",
            "    }\n}",
            "public function approve($amount) {",
        ),
        (
            "client.swift",
            "class Client {\n    public func approve(amount: Int) {",
            "        send(\"before\")",
            "    }\n}",
            "public func approve(amount: Int) {",
        ),
        (
            "client.c",
            "static void approve(int amount)\n{",
            "    send(\"before\");",
            "}",
            "static void approve(int amount)",
        ),
        (
            "client.cpp",
            "class Client {\npublic:\n    void approve(int amount) {",
            "        send(\"before\");",
            "    }\n};",
            "void approve(int amount) {",
        ),
        (
            "Client.cs",
            "class Client {\n    public void Approve(int amount) {",
            "        Send(\"before\");",
            "    }\n}",
            "public void Approve(int amount) {",
        ),
        (
            "client.sh",
            "approve() {",
            "    echo \"before\"",
            "}",
            "approve() {",
        ),
        (
            "client.zsh",
            "function approve {",
            "    echo \"before\"",
            "}",
            "function approve {",
        ),
        (
            "client.lua",
            "local function approve(amount)",
            "    send(\"before\")",
            "end",
            "local function approve(amount)",
        ),
        (
            "client.ex",
            "defmodule Client do\n  def approve(amount) do",
            "    send(\"before\")",
            "  end\nend",
            "def approve(amount) do",
        ),
    ];
    for (path, declaration, statement, end, _) in cases {
        let context = format!("{}\n", statement.replace("before", "unchanged")).repeat(8);
        fixture.write(
            path,
            format!("{declaration}\n{context}{statement}\n{end}\n"),
        );
    }
    fixture.commit();
    for (path, _, _, _, _) in cases {
        let source = fs::read_to_string(fixture.root.join(path)).unwrap();
        fixture.write(path, source.replace("before", "after"));
    }
    let repo = fixture.repository();
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    for (path, _, _, _, declaration) in cases {
        let change = snapshot
            .changes
            .iter()
            .find(|change| change.path == Path::new(path))
            .unwrap();
        let headers = displayed_hunk_headers(&repo, &snapshot, change);
        assert_eq!(headers.len(), 1, "{path}: {headers:?}");
        assert!(headers[0].ends_with(declaration), "{path}: {}", headers[0]);
    }
}

#[test]
fn hunk_headers_follow_comparison_sources_and_renames() {
    let fixture = Fixture::new();
    let source = format!(
        "class Client {{\n    private fun approve() {{\n{}        send(\"before\")\n    }}\n}}\n",
        "        send(\"unchanged\")\n".repeat(8)
    );
    fixture.write("client.kt", &source);
    fixture.commit();
    fixture.write("client.kt", source.replace("before", "staged"));
    fixture.git(&["add", "client.kt"]);
    fixture.write(
        "client.kt",
        "class Client:\n    def unrelated(self):\n        pass\n",
    );
    let repo = fixture.repository();
    let staged = repo.snapshot(&Mode::Staged).unwrap();
    let headers = displayed_hunk_headers(&repo, &staged, &staged.changes[0]);
    assert!(
        headers[0].ends_with("private fun approve() {"),
        "{headers:?}"
    );

    fixture.write("client.kt", source.replace("before", "after"));
    fixture.git(&["mv", "client.kt", "client.txt"]);
    fixture.commit();
    let branches = repo
        .snapshot(&Mode::Branches {
            from: "HEAD~1".into(),
            to: "HEAD".into(),
        })
        .unwrap();
    assert_eq!(
        branches.changes[0].old_path.as_deref(),
        Some(Path::new("client.kt"))
    );
    let headers = displayed_hunk_headers(&repo, &branches, &branches.changes[0]);
    assert!(
        headers[0].ends_with("private fun approve() {"),
        "{headers:?}"
    );
}

fn displayed_hunk_headers(repo: &Repository, snapshot: &Snapshot, change: &Change) -> Vec<String> {
    let patch = repo.patch(snapshot, change, Whitespace::Ignored).unwrap();
    let sources = repo.sources(snapshot, change).unwrap();
    let mut display = DiffDisplay::default();
    display.indentation = Indentation::Full;
    assert_eq!(display.load(&patch, change, &sources), None);
    display.layout(500);
    display
        .rows
        .iter()
        .filter(|row| row.starts_hunk)
        .map(|row| row.left.as_ref().unwrap().code.to_string())
        .collect()
}

#[test]
fn hunk_header_fallback_preserves_attribute_drivers_binary_and_unknown_files() {
    let fixture = Fixture::new();
    fixture.write(
        ".gitattributes",
        "custom.kt diff=custom\nbuiltin.py diff=python\nbinary.kt -diff\n",
    );
    fixture.git(&["config", "diff.custom.xfuncname", "^(CUSTOM CONTEXT)$"]);
    let context = "    unchanged\n".repeat(8);
    for (path, declaration) in [
        ("custom.kt", "CUSTOM CONTEXT"),
        ("builtin.py", "def approve():"),
        ("binary.kt", "private fun approve() {"),
        ("unknown.xyz", "Original context"),
    ] {
        fixture.write(path, format!("{declaration}\n{context}    before\n"));
    }
    fixture.commit();
    for path in ["custom.kt", "builtin.py", "binary.kt", "unknown.xyz"] {
        let source = fs::read_to_string(fixture.root.join(path)).unwrap();
        fixture.write(path, source.replace("before", "after"));
    }
    let config_before = fs::read(fixture.root.join(".git/config")).unwrap();
    let attributes_before = fs::read(fixture.root.join(".gitattributes")).unwrap();
    let repo = fixture.repository();
    let snapshot = repo.snapshot(&Mode::Working).unwrap();
    for (path, expected) in [
        ("custom.kt", "@@ CUSTOM CONTEXT"),
        ("builtin.py", "@@ def approve():"),
        ("unknown.xyz", "@@ Original context"),
    ] {
        let change = snapshot
            .changes
            .iter()
            .find(|change| change.path == Path::new(path))
            .unwrap();
        let patch = repo.patch(&snapshot, change, Whitespace::Compared).unwrap();
        assert!(
            patch
                .iter()
                .any(|line| line.kind == LineKind::Hunk && line.text.ends_with(expected)),
            "{path}: {patch:?}"
        );
    }
    let binary = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("binary.kt"))
        .unwrap();
    let patch = repo.patch(&snapshot, binary, Whitespace::Compared).unwrap();
    assert!(
        patch
            .iter()
            .any(|line| line.text.starts_with("Binary files "))
    );
    assert!(!patch.iter().any(|line| line.kind == LineKind::Hunk));
    assert_eq!(
        fs::read(fixture.root.join(".git/config")).unwrap(),
        config_before
    );
    assert_eq!(
        fs::read(fixture.root.join(".gitattributes")).unwrap(),
        attributes_before
    );
}

#[test]
fn branch_comparison_uses_both_tips_and_preserves_worktree() {
    let fixture = Fixture::new();
    fixture.write("src/main.rs", "old\n");
    fixture.write("tests/example.rs", "old test\n");
    fixture.commit();
    fixture.git(&["checkout", "-b", "feature"]);
    fixture.write("src/main.rs", "new\n");
    fixture.write("tests/example.rs", "new test\n");
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
        .snapshot(&Mode::Branches {
            from: "main".into(),
            to: "feature".into(),
        })
        .unwrap();
    assert!(
        snapshot
            .changes
            .iter()
            .any(|change| change.path == Path::new("main-only.txt")
                && change.status == ChangeStatus::Deleted)
    );
    let change = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("src/main.rs"))
        .unwrap();
    let patch = repo.patch(&snapshot, change, Whitespace::Compared).unwrap();
    assert!(
        patch
            .iter()
            .any(|line| line.kind == LineKind::Added && line.text == "+new")
    );
    assert!(!patch.iter().any(|line| line.text.contains("uncommitted")));
    assert!(
        repo.snapshot(&Mode::Branches {
            from: "missing-branch".into(),
            to: "main".into(),
        })
        .is_err()
    );
    assert!(
        repo.snapshot(&Mode::Branches {
            from: "--output=bad".into(),
            to: "main".into(),
        })
        .is_err()
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fixture.git(&["status", "--porcelain=v1"]), status);
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/main.rs")).unwrap(),
        "uncommitted, must survive\n"
    );

    let mut app = App::new(repo, Mode::Working, TestFiles::Hidden).unwrap();
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
    assert!(matches!(&app.mode, Mode::Branches { from, to } if from == "main" && to == "feature"));
    assert!(app.dialog.is_none());
    assert_eq!(app.hidden_test_count(), 1);
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
    let patch = repo
        .patch(&snapshot, &snapshot.changes[0], Whitespace::Compared)
        .unwrap();
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
        .find(|change| change.status == ChangeStatus::Renamed)
        .unwrap();
    assert_eq!(renamed.path, PathBuf::from("src/new name.rs"));
    assert!(
        repo.patch(&snapshot, renamed, Whitespace::Compared)
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
        repo.patch(&snapshot, binary, Whitespace::Compared)
            .unwrap()
            .iter()
            .any(|line| line.text.contains("Binary files"))
    );
    let literal = snapshot
        .changes
        .iter()
        .find(|change| change.path == Path::new("src/[literal].rs"))
        .unwrap();
    let patch = repo
        .patch(&snapshot, literal, Whitespace::Compared)
        .unwrap();
    assert!(patch.iter().any(|line| line.text == "+new literal"));
    assert!(!patch.iter().any(|line| line.text.contains("other change")));
    assert!(
        snapshot
            .changes
            .iter()
            .any(|change| change.status == ChangeStatus::Deleted)
    );
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
    let mut app = App::new(fixture.repository(), Mode::Working, TestFiles::Hidden).unwrap();
    app.layout_diff(120, 10);
    app.scroll = 25;
    app.horizontal_scroll = 8;
    app.focus = Focus::Diff;
    app.query = "src/".into();
    app.searching = true;
    app.dialog = Some(Dialog::Help);
    let rows = app.display.rows.as_ptr();
    let line = app.display.line_at_row(app.scroll);
    let old_line = app.preview.patch[line].old;
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
        app.preview.patch[app.display.line_at_row(app.scroll)].old,
        old_line
    );
    assert_eq!(app.horizontal_scroll, 8);
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
    let mut app = App::new(fixture.repository(), Mode::Working, TestFiles::Hidden).unwrap();
    assert!(app.visible.is_empty());
    fixture.write("tests/a.rs", "test change\n");
    fixture.write("src/a.rs", "new a\n");
    fixture.write("src/b.rs", "new b\n");
    refresh(&mut app);
    assert_eq!(app.visible.len(), 2);
    assert_eq!(app.hidden_test_count(), 1);
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
    let mut app = App::new(fixture.repository(), Mode::Staged, TestFiles::Hidden).unwrap();
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
        Mode::Branches {
            from: "main".into(),
            to: "feature".into(),
        },
        TestFiles::Hidden,
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
    use crate::{app::Focus, file_tree::EntryKind, ui};
    const COLLAPSED: EntryKind = EntryKind::Directory { collapsed: true };
    const EXPANDED: EntryKind = EntryKind::Directory { collapsed: false };
    use ratatui::{Terminal, backend::TestBackend};

    let fixture = Fixture::new();
    let path = "project/application/src/main/kotlin/api/Service.kt";
    let other = "project/application/src/main/kotlin/web/Service.kt";
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
    let mut app = App::new(fixture.repository(), Mode::Working, TestFiles::Hidden).unwrap();
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
    assert!(text.contains("project/application/src/main/kotlin/"));
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
        Path::new(path).parent().unwrap().parent().unwrap()
    );
    app.handle(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.tree.current().unwrap().kind, COLLAPSED);
    refresh(&mut app);
    assert_eq!(app.tree.current().unwrap().kind, COLLAPSED);
    app.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    render(&mut app, &mut terminal);
    assert_eq!(app.scroll, 8);
    app.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.tree.current().unwrap().kind, COLLAPSED);
    app.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.tree.current().unwrap().kind, EXPANDED);
}
