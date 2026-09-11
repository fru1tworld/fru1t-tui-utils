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
    Branches(String, String),
}

impl Mode {
    pub fn label(&self) -> String {
        match self {
            Self::Working => "HEAD -> working tree (staged + unstaged)".into(),
            Self::Unstaged => "index -> working tree (unstaged)".into(),
            Self::Staged => "HEAD -> index (staged)".into(),
            Self::Branches(from, to) => format!("{from} -> {to}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub status: char,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

impl Change {
    pub fn is_test(&self) -> bool {
        // Keep moves across the production/test boundary visible for review.
        is_test_path(&self.path) && self.old_path.as_ref().is_none_or(|path| is_test_path(path))
    }

    pub fn label(&self) -> String {
        let path = clean(&self.path.to_string_lossy());
        match &self.old_path {
            Some(old) => format!("{} -> {path}", clean(&old.to_string_lossy())),
            None => path,
        }
    }
}

pub struct Snapshot {
    pub comparison: Comparison,
    pub changes: Vec<Change>,
}

pub enum Comparison {
    Working(String),
    Unstaged,
    Staged(Option<String>),
    Between(String, String),
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub old: Option<String>,
    pub new: Option<String>,
}

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
        ensure!(
            output.status.success(),
            "{}",
            clean(String::from_utf8_lossy(&output.stderr).trim())
        );
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
            .with_context(|| format!("Cannot resolve revision {}", clean(revision)))?;
        Ok(String::from_utf8(bytes)?.trim().to_owned())
    }

    pub fn snapshot(&self, mode: &Mode) -> Result<Snapshot> {
        let comparison = match mode {
            Mode::Working => {
                let hash = if let Some(head) = self.head()? {
                    head
                } else {
                    // hash-object without -w computes the empty tree without writing an object.
                    String::from_utf8(self.run(&["hash-object", "-t", "tree", "--stdin"])?)?
                        .trim()
                        .to_owned()
                };
                Comparison::Working(hash)
            }
            Mode::Unstaged => Comparison::Unstaged,
            Mode::Staged => Comparison::Staged(self.head()?),
            Mode::Branches(from, to) => Comparison::Between(self.resolve(from)?, self.resolve(to)?),
        };
        let bytes = Self::checked(
            self.diff_command(&comparison)
                .args(["--name-status", "-z", "--"])
                .output()?,
        )?;
        Ok(Snapshot {
            comparison,
            changes: parse_changes(&bytes)?,
        })
    }

    fn head(&self) -> Result<Option<String>> {
        let output = self
            .command()
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8(Self::checked(output)?)?.trim().to_owned(),
        ))
    }

    fn diff_command(&self, comparison: &Comparison) -> Command {
        let mut command = self.command();
        command.args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--color=never",
            "--no-relative",
            "--find-renames",
            "--ignore-submodules=none",
            "--submodule=short",
        ]);
        match comparison {
            Comparison::Working(base) => {
                command.arg(base);
            }
            Comparison::Unstaged => {}
            Comparison::Staged(base) => {
                command.arg("--cached");
                if let Some(base) = base {
                    command.arg(base);
                }
            }
            Comparison::Between(from, to) => {
                command.args([from, to]);
            }
        }
        command
    }

    pub fn patch(
        &self,
        snapshot: &Snapshot,
        change: &Change,
        ignore_whitespace: bool,
    ) -> Result<Vec<DiffLine>> {
        let mut command = self.diff_command(&snapshot.comparison);
        if ignore_whitespace {
            command.arg("--ignore-all-space");
        }
        command.args(["--patch", "--unified=3", "--"]);
        if let Some(old) = &change.old_path {
            command.arg(old);
        }
        command.arg(&change.path);
        let output = Self::checked(command.output()?)?;
        Ok(parse_patch(&String::from_utf8_lossy(&output)))
    }

    pub fn sources(&self, snapshot: &Snapshot, change: &Change) -> Result<Sources> {
        let old_path = change.old_path.as_deref().unwrap_or(&change.path);
        let old = if change.status == 'A' || change.status == 'U' {
            None
        } else {
            match &snapshot.comparison {
                Comparison::Working(base) | Comparison::Between(base, _) => {
                    self.blob(base, old_path)?
                }
                Comparison::Staged(Some(base)) => self.blob(base, old_path)?,
                Comparison::Staged(None) => None,
                Comparison::Unstaged => self.blob("", old_path)?,
            }
        };
        let new = if change.status == 'D' {
            None
        } else {
            match &snapshot.comparison {
                Comparison::Between(_, to) => self.blob(to, &change.path)?,
                Comparison::Staged(_) => self.blob("", &change.path)?,
                Comparison::Working(_) | Comparison::Unstaged => {
                    self.working_source(&change.path)?
                }
            }
        };
        Ok(Sources { old, new })
    }

    fn blob(&self, revision: &str, path: &Path) -> Result<Option<String>> {
        let mut object = OsString::from(format!("{revision}:"));
        object.push(path);
        let kind = Self::checked(
            self.command()
                .args(["cat-file", "-t"])
                .arg(&object)
                .output()?,
        )?;
        if kind != b"blob\n" {
            // Submodule gitlinks reference commits, not source files.
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
        let Some(status) = status.first().copied() else {
            bail!("Empty git diff status");
        };
        let first = fields
            .next()
            .filter(|field| !field.is_empty())
            .context("Missing git diff path")?;
        let (old_path, path) = if matches!(status, b'R' | b'C') {
            let second = fields
                .next()
                .filter(|field| !field.is_empty())
                .context("Missing rename destination")?;
            (
                Some(PathBuf::from(decode_path(first)?)),
                PathBuf::from(decode_path(second)?),
            )
        } else {
            (None, PathBuf::from(decode_path(first)?))
        };
        changes.push(Change {
            status: char::from(status),
            path,
            old_path,
        });
    }
    Ok(changes)
}

pub fn is_test_path(path: &Path) -> bool {
    for component in path.components() {
        let part = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        if matches!(
            part.as_str(),
            "test"
                | "tests"
                | "__tests__"
                | "__snapshots__"
                | "testdata"
                | "testfixtures"
                | "test-fixtures"
                | "androidtest"
                | "unittest"
                | "integrationtest"
                | "integrationtests"
                | "integration-test"
                | "integration-tests"
                | "commontest"
                | "jvmtest"
                | "jstest"
                | "nativetest"
                | "iostest"
                | "e2e"
                | "spec"
                | "specs"
        ) {
            return true;
        }
    }
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.go")
        || (lower.ends_with(".py") && (lower.starts_with("test_") || lower.ends_with("_test.py")))
        || (["java", "kt", "scala", "groovy"]
            .iter()
            .any(|ext| lower.ends_with(&format!(".{ext}")))
            && ["Test", "Tests", "Spec"]
                .iter()
                .any(|suffix| stem.ends_with(suffix)))
}

// Treat repository content as text, never terminal escape sequences.
pub fn clean(text: &str) -> String {
    let mut result = String::new();
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub kind: LineKind,
}

pub(crate) fn parse_patch(patch: &str) -> Vec<DiffLine> {
    let (mut old, mut new) = (0, 0);
    let mut in_hunk = false;
    patch
        .lines()
        .map(|line| {
            let mut result = DiffLine {
                text: clean(line),
                old: None,
                new: None,
                kind: LineKind::Header,
            };
            if line.starts_with("diff --git ") {
                in_hunk = false;
            } else if line.starts_with("@@ ") {
                let mut fields = line.split_whitespace().skip(1);
                let parse_start = |field: Option<&str>| {
                    field
                        .and_then(|value| value.get(1..))
                        .and_then(|value| value.split(',').next())
                        .and_then(|value| value.parse().ok())
                };
                if let (Some(a), Some(b)) = (parse_start(fields.next()), parse_start(fields.next()))
                {
                    old = a;
                    new = b;
                    in_hunk = true;
                }
                result.kind = LineKind::Hunk;
            } else if in_hunk {
                match line.as_bytes().first() {
                    Some(b'+') => {
                        result.kind = LineKind::Added;
                        result.new = Some(new);
                        new += 1;
                    }
                    Some(b'-') => {
                        result.kind = LineKind::Removed;
                        result.old = Some(old);
                        old += 1;
                    }
                    Some(b' ') => {
                        result.kind = LineKind::Context;
                        result.old = Some(old);
                        result.new = Some(new);
                        old += 1;
                        new += 1;
                    }
                    _ => {}
                }
            }
            result
        })
        .collect()
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
            "a/PaymentSpec.scala",
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
    fn parses_nul_delimited_renames_and_unusual_paths() {
        let changes =
            parse_changes(b"R100\0src/old name.rs\0src/new\nname.rs\0D\0src/test/gone.rs\0")
                .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(
            changes[0].old_path.as_deref(),
            Some(Path::new("src/old name.rs"))
        );
        assert_eq!(changes[0].path, Path::new("src/new\nname.rs"));
        assert_eq!(changes[1].status, 'D');
        assert!(changes[1].is_test());
        assert!(parse_changes(b"R100\0old\0").is_err());
    }

    #[test]
    fn keeps_renames_across_test_boundary_visible() {
        let change = Change {
            status: 'R',
            path: "tests/a.rs".into(),
            old_path: Some("src/a.rs".into()),
        };
        assert!(!change.is_test());
    }

    #[test]
    fn numbers_hunks_and_does_not_emit_terminal_controls() {
        let lines = parse_patch("--- a/x\n+++ b/x\n@@ -7,2 +9,2 @@\n-old\n+new\n same\n");
        assert_eq!((lines[3].old, lines[3].new), (Some(7), None));
        assert_eq!((lines[4].old, lines[4].new), (None, Some(9)));
        assert_eq!((lines[5].old, lines[5].new), (Some(8), Some(10)));
        assert_eq!(clean("\x1b[31mhello\tworld"), "\\u{1b}[31mhello    world");
    }
}
