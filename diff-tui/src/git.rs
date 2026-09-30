use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Working,
    Unstaged,
    Staged,
    Branches { from: String, to: String },
}

impl Mode {
    pub fn label(&self) -> String {
        match self {
            Self::Working => "HEAD -> working tree (staged + unstaged + untracked)".into(),
            Self::Unstaged => "index -> working tree (unstaged)".into(),
            Self::Staged => "HEAD -> index (staged)".into(),
            Self::Branches { from, to } => format!("{from} -> {to}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    Unmerged,
    Untracked,
    Other(char),
}

impl ChangeStatus {
    fn from_git(code: u8) -> Self {
        match code {
            b'A' => Self::Added,
            b'C' => Self::Copied,
            b'D' => Self::Deleted,
            b'M' => Self::Modified,
            b'R' => Self::Renamed,
            b'U' => Self::Unmerged,
            other => Self::Other(char::from(other)),
        }
    }

    pub fn marker(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Copied => 'C',
            Self::Deleted => 'D',
            Self::Modified => 'M',
            Self::Renamed => 'R',
            Self::Unmerged => 'U',
            Self::Untracked => '?',
            Self::Other(code) => code,
        }
    }

    fn has_old_side(self) -> bool {
        !matches!(self, Self::Added | Self::Unmerged | Self::Untracked)
    }

    fn has_new_side(self) -> bool {
        self != Self::Deleted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub status: ChangeStatus,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

impl Change {
    pub fn is_test(&self) -> bool {
        is_test_path(&self.path) && self.old_path.as_ref().is_none_or(|path| is_test_path(path))
    }

    pub fn label(&self) -> String {
        let path = escape_controls(&self.path.to_string_lossy());
        match &self.old_path {
            Some(old) => format!("{} -> {path}", escape_controls(&old.to_string_lossy())),
            None => path,
        }
    }

    pub fn move_summary(&self, similarity: &str) -> Option<String> {
        let old = self.old_path.as_ref()?;
        let verb = if self.status == ChangeStatus::Copied {
            "Copied"
        } else {
            "Moved"
        };
        let detail = if similarity == "100%" {
            "content unchanged".to_owned()
        } else {
            format!("{similarity} similar")
        };
        let paths = collapse_move(
            &escape_controls(&old.to_string_lossy()),
            &escape_controls(&self.path.to_string_lossy()),
        );
        Some(format!("{verb}, {detail}: {paths}"))
    }
}

fn collapse_move(old: &str, new: &str) -> String {
    let old: Vec<_> = old.split('/').collect();
    let new: Vec<_> = new.split('/').collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let moved = format!(
        "{{{} -> {}}}",
        old[prefix..old.len() - suffix].join("/"),
        new[prefix..new.len() - suffix].join("/")
    );
    [
        old[..prefix].join("/"),
        moved,
        old[old.len() - suffix..].join("/"),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("/")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeIndex(pub usize);

pub struct Snapshot {
    pub comparison: Comparison,
    pub changes: Vec<Change>,
}

impl Snapshot {
    pub fn change(&self, index: ChangeIndex) -> &Change {
        &self.changes[index.0]
    }
}

pub enum Comparison {
    Working { base: String },
    Unstaged,
    Staged { base: Option<String> },
    Between { from: String, to: String },
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Whitespace {
    #[default]
    Compared,
    Ignored,
}

impl Whitespace {
    pub fn toggled(self) -> Self {
        match self {
            Self::Compared => Self::Ignored,
            Self::Ignored => Self::Compared,
        }
    }

    fn diff_arg(self) -> Option<&'static str> {
        (self == Self::Ignored).then_some("--ignore-all-space")
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub old: Option<String>,
    pub new: Option<String>,
}

enum Revision<'a> {
    Index,
    Commit(&'a str),
}

const PLAIN_DIFF_ARGS: [&str; 4] = [
    "--no-ext-diff",
    "--no-textconv",
    "--color=never",
    "--no-relative",
];

pub struct Repository {
    pub root: PathBuf,
}

impl Repository {
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Self {
            root: path.to_owned(),
        };
        let output = repo.run(&["rev-parse", "--show-toplevel"])?;
        let bytes = output.strip_suffix(b"\n").unwrap_or(&output);
        Ok(Self {
            root: PathBuf::from(decode_path(bytes)?),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        command
            .current_dir(&self.root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args([
                "--no-pager",
                "--literal-pathspecs",
                "-c",
                "core.quotePath=false",
            ])
            .stdin(Stdio::null());
        command
    }

    fn checked(output: Output) -> Result<Vec<u8>> {
        ensure!(output.status.success(), "{}", stderr_message(&output));
        Ok(output.stdout)
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        Self::checked(
            self.command()
                .args(args)
                .output()
                .context("Could not run git")?,
        )
    }

    fn resolve(&self, revision: &str) -> Result<String> {
        let bytes = self
            .run(&[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{commit}}"),
            ])
            .with_context(|| format!("Cannot resolve revision {}", escape_controls(revision)))?;
        trimmed_text(bytes)
    }

    fn head(&self) -> Result<Option<String>> {
        let output = self
            .command()
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        trimmed_text(Self::checked(output)?).map(Some)
    }

    fn empty_tree(&self) -> Result<String> {
        trimmed_text(self.run(&["hash-object", "-t", "tree", "--stdin"])?)
    }

    fn resolve_mode(&self, mode: &Mode) -> Result<Comparison> {
        Ok(match mode {
            Mode::Working => Comparison::Working {
                base: match self.head()? {
                    Some(head) => head,
                    None => self.empty_tree()?,
                },
            },
            Mode::Unstaged => Comparison::Unstaged,
            Mode::Staged => Comparison::Staged { base: self.head()? },
            Mode::Branches { from, to } => Comparison::Between {
                from: self.resolve(from)?,
                to: self.resolve(to)?,
            },
        })
    }

    pub fn snapshot(&self, mode: &Mode) -> Result<Snapshot> {
        let comparison = self.resolve_mode(mode)?;
        let bytes = Self::checked(
            self.diff_command(&comparison)
                .args(["--name-status", "-z", "--"])
                .output()?,
        )?;
        let mut changes = parse_changes(&bytes)?;
        if *mode == Mode::Working {
            self.add_untracked(&mut changes)?;
            changes.sort_by(|left, right| left.path.cmp(&right.path));
        }
        Ok(Snapshot {
            comparison,
            changes,
        })
    }

    fn add_untracked(&self, changes: &mut Vec<Change>) -> Result<()> {
        let untracked = self.run(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in untracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = PathBuf::from(decode_path(path)?);
            if let Some(staged_deletion) = changes.iter_mut().find(|change| change.path == path) {
                staged_deletion.status = ChangeStatus::Untracked;
                staged_deletion.old_path = None;
            } else {
                changes.push(Change {
                    status: ChangeStatus::Untracked,
                    path,
                    old_path: None,
                });
            }
        }
        Ok(())
    }

    fn diff_command(&self, comparison: &Comparison) -> Command {
        let mut command = self.command();
        command.arg("diff").args(PLAIN_DIFF_ARGS).args([
            "--find-renames",
            "--ignore-submodules=none",
            "--submodule=short",
        ]);
        match comparison {
            Comparison::Working { base } => {
                command.arg(base);
            }
            Comparison::Unstaged => {}
            Comparison::Staged { base } => {
                command.arg("--cached").args(base);
            }
            Comparison::Between { from, to } => {
                command.args([from, to]);
            }
        }
        command
    }

    pub fn patch(
        &self,
        snapshot: &Snapshot,
        change: &Change,
        whitespace: Whitespace,
    ) -> Result<Vec<DiffLine>> {
        if change.status == ChangeStatus::Untracked {
            return self.untracked_patch(change, whitespace);
        }
        let mut command = self.diff_command(&snapshot.comparison);
        command
            .args(whitespace.diff_arg())
            .args(["--patch", "--unified=3", "--"])
            .args(&change.old_path)
            .arg(&change.path);
        let output = Self::checked(command.output()?)?;
        Ok(parse_patch(&String::from_utf8_lossy(&output)))
    }

    fn untracked_patch(&self, change: &Change, whitespace: Whitespace) -> Result<Vec<DiffLine>> {
        let output = self
            .command()
            .args(["diff", "--no-index"])
            .args(PLAIN_DIFF_ARGS)
            .args(["--patch", "--unified=3"])
            .args(whitespace.diff_arg())
            .args(["--", "/dev/null"])
            .arg(&change.path)
            .output()
            .context("Could not read untracked file diff")?;
        let identical_or_different = matches!(output.status.code(), Some(0 | 1));
        ensure!(
            identical_or_different,
            "Could not read untracked file {}: {}",
            change.label(),
            stderr_message(&output)
        );
        Ok(parse_patch(&String::from_utf8_lossy(&output.stdout)))
    }

    pub fn sources(&self, snapshot: &Snapshot, change: &Change) -> Result<Sources> {
        let old_path = change.old_path.as_deref().unwrap_or(&change.path);
        let old = if change.status.has_old_side() {
            match &snapshot.comparison {
                Comparison::Working { base }
                | Comparison::Between { from: base, .. }
                | Comparison::Staged { base: Some(base) } => {
                    self.blob(Revision::Commit(base), old_path)?
                }
                Comparison::Staged { base: None } => None,
                Comparison::Unstaged => self.blob(Revision::Index, old_path)?,
            }
        } else {
            None
        };
        let new = if change.status.has_new_side() {
            match &snapshot.comparison {
                Comparison::Between { to, .. } => self.blob(Revision::Commit(to), &change.path)?,
                Comparison::Staged { .. } => self.blob(Revision::Index, &change.path)?,
                Comparison::Working { .. } | Comparison::Unstaged => {
                    self.working_source(&change.path)?
                }
            }
        } else {
            None
        };
        Ok(Sources { old, new })
    }

    fn blob(&self, revision: Revision<'_>, path: &Path) -> Result<Option<String>> {
        let mut object = OsString::from(match revision {
            Revision::Index => ":".to_owned(),
            Revision::Commit(commit) => format!("{commit}:"),
        });
        object.push(path);
        let object_type = Self::checked(
            self.command()
                .args(["cat-file", "-t"])
                .arg(&object)
                .output()?,
        )?;
        let is_submodule_commit_or_tree = object_type != b"blob\n";
        if is_submodule_commit_or_tree {
            return Ok(None);
        }
        let output = Self::checked(
            self.command()
                .args(["cat-file", "blob"])
                .arg(object)
                .output()?,
        )?;
        Ok(Some(String::from_utf8_lossy(&output).into_owned()))
    }

    fn working_source(&self, path: &Path) -> Result<Option<String>> {
        let path = self.root.join(path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.is_dir() {
            return Ok(None);
        }
        if metadata.file_type().is_symlink() {
            return Ok(Some(fs::read_link(path)?.to_string_lossy().into_owned()));
        }
        Ok(Some(String::from_utf8_lossy(&fs::read(path)?).into_owned()))
    }

    pub fn branches(&self) -> Result<Vec<String>> {
        let bytes = self.run(&[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/",
            "refs/remotes/",
        ])?;
        Ok(String::from_utf8(bytes)?
            .lines()
            .filter(|line| !line.ends_with("/HEAD"))
            .map(str::to_owned)
            .collect())
    }
}

fn stderr_message(output: &Output) -> String {
    escape_controls(String::from_utf8_lossy(&output.stderr).trim())
}

fn trimmed_text(bytes: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(bytes)?.trim().to_owned())
}

#[cfg(unix)]
fn decode_path(bytes: &[u8]) -> Result<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Ok(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn decode_path(bytes: &[u8]) -> Result<OsString> {
    Ok(String::from_utf8(bytes.to_vec())
        .context("Git path is not valid UTF-8")?
        .into())
}

fn parse_changes(bytes: &[u8]) -> Result<Vec<Change>> {
    let mut fields = bytes.split(|byte| *byte == 0).peekable();
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() && fields.peek().is_none() {
            break;
        }
        let Some(&code) = status.first() else {
            bail!("Empty git diff status");
        };
        let status = ChangeStatus::from_git(code);
        let first = next_path(&mut fields, "Missing git diff path")?;
        let (old_path, path) = if matches!(status, ChangeStatus::Renamed | ChangeStatus::Copied) {
            let destination = next_path(&mut fields, "Missing rename destination")?;
            (Some(first), destination)
        } else {
            (None, first)
        };
        changes.push(Change {
            status,
            path,
            old_path,
        });
    }
    Ok(changes)
}

fn next_path<'a>(
    fields: &mut impl Iterator<Item = &'a [u8]>,
    missing: &'static str,
) -> Result<PathBuf> {
    let field = fields
        .next()
        .filter(|field| !field.is_empty())
        .context(missing)?;
    Ok(PathBuf::from(decode_path(field)?))
}

const TEST_DIRECTORIES: [&str; 21] = [
    "test",
    "tests",
    "__tests__",
    "__snapshots__",
    "testdata",
    "testfixtures",
    "test-fixtures",
    "androidtest",
    "unittest",
    "integrationtest",
    "integrationtests",
    "integration-test",
    "integration-tests",
    "commontest",
    "jvmtest",
    "jstest",
    "nativetest",
    "iostest",
    "e2e",
    "spec",
    "specs",
];

pub fn is_test_path(path: &Path) -> bool {
    let in_test_directory = path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        TEST_DIRECTORIES.contains(&name.as_str())
    });
    if in_test_directory {
        return true;
    }
    let Some(name) = path.file_name() else {
        return false;
    };
    let lower = name.to_string_lossy().to_ascii_lowercase();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let is_jvm_source = [".java", ".kt", ".scala", ".groovy"]
        .iter()
        .any(|extension| lower.ends_with(extension));
    lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.go")
        || (lower.ends_with(".py") && (lower.starts_with("test_") || lower.ends_with("_test.py")))
        || (is_jvm_source
            && ["Test", "Tests", "Spec"]
                .iter()
                .any(|suffix| stem.ends_with(suffix)))
}

pub fn escape_controls(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\t' {
            result.push_str("    ");
        } else if ch.is_control() {
            result.extend(ch.escape_default());
        } else {
            result.push(ch);
        }
    }
    result
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

impl LineKind {
    pub fn is_code(self) -> bool {
        matches!(self, Self::Context | Self::Added | Self::Removed)
    }

    pub fn is_change(self) -> bool {
        matches!(self, Self::Added | Self::Removed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub kind: LineKind,
}

pub(crate) fn parse_patch(patch: &str) -> Vec<DiffLine> {
    let (mut old_number, mut new_number) = (0, 0);
    let mut in_hunk = false;
    patch
        .lines()
        .map(|text| {
            let mut line = DiffLine {
                text: escape_controls(text),
                old: None,
                new: None,
                kind: LineKind::Header,
            };
            if text.starts_with("diff --git ") {
                in_hunk = false;
            } else if text.starts_with("@@ ") {
                line.kind = LineKind::Hunk;
                if let Some((old_start, new_start)) = hunk_starts(text) {
                    old_number = old_start;
                    new_number = new_start;
                    in_hunk = true;
                }
            } else if in_hunk {
                match text.as_bytes().first() {
                    Some(b'+') => {
                        line.kind = LineKind::Added;
                        line.new = Some(new_number);
                        new_number += 1;
                    }
                    Some(b'-') => {
                        line.kind = LineKind::Removed;
                        line.old = Some(old_number);
                        old_number += 1;
                    }
                    Some(b' ') => {
                        line.kind = LineKind::Context;
                        line.old = Some(old_number);
                        line.new = Some(new_number);
                        old_number += 1;
                        new_number += 1;
                    }
                    _ => {}
                }
            }
            line
        })
        .collect()
}

fn hunk_starts(header: &str) -> Option<(usize, usize)> {
    let mut ranges = header.split_whitespace().skip(1);
    let mut start =
        || -> Option<usize> { ranges.next()?.get(1..)?.split(',').next()?.parse().ok() };
    Some((start()?, start()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_tests_without_matching_production_substrings() {
        for path in [
            "module/src/test/kotlin/Helper.kt",
            "src/androidTest/Test.kt",
            "src/testFixtures/data.json",
            "a/__tests__/Button.tsx",
            "a/foo.spec.ts",
            "test_calc.py",
            "a/foo_test.go",
            "a/ParserSpec.scala",
        ] {
            assert!(is_test_path(Path::new(path)), "{path}");
        }
        for path in [
            "src/main/kotlin/Contest.kt",
            "src/main/Latest.java",
            "src/testimony.ts",
            "README.md",
            "src/testing/Client.kt",
            "src/main.rs",
            "build.gradle.kts",
        ] {
            assert!(!is_test_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn keeps_renames_across_test_boundary_visible() {
        let change = Change {
            status: ChangeStatus::Renamed,
            path: "tests/a.rs".into(),
            old_path: Some("src/a.rs".into()),
        };
        assert!(!change.is_test());
    }

    #[test]
    fn collapses_shared_path_parts_of_moves() {
        assert_eq!(
            collapse_move(
                "service/common/domain/src/Action.kt",
                "service/agent/src/Action.kt"
            ),
            "service/{common/domain -> agent}/src/Action.kt"
        );
        assert_eq!(collapse_move("src/a.rs", "src/b.rs"), "src/{a.rs -> b.rs}");
        assert_eq!(collapse_move("a.rs", "lib/a.rs"), "{ -> lib}/a.rs");
        let change = Change {
            status: ChangeStatus::Renamed,
            path: "src/b.rs".into(),
            old_path: Some("src/a.rs".into()),
        };
        assert_eq!(
            change.move_summary("100%").as_deref(),
            Some("Moved, content unchanged: src/{a.rs -> b.rs}")
        );
        assert_eq!(
            change.move_summary("96%").as_deref(),
            Some("Moved, 96% similar: src/{a.rs -> b.rs}")
        );
    }

    #[test]
    fn does_not_emit_terminal_controls() {
        assert_eq!(
            escape_controls("\x1b[31mhello\tworld"),
            "\\u{1b}[31mhello    world"
        );
    }
}
