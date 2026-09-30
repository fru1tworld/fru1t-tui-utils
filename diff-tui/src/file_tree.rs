use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use ratatui::widgets::ListState;

use crate::git::{Change, ChangeIndex, escape_controls};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File(ChangeIndex),
    Directory { collapsed: bool },
}

pub struct Entry {
    pub path: PathBuf,
    pub label: String,
    pub depth: usize,
    pub kind: EntryKind,
}

impl Entry {
    pub fn change(&self) -> Option<ChangeIndex> {
        match self.kind {
            EntryKind::File(change) => Some(change),
            EntryKind::Directory { .. } => None,
        }
    }

    fn is_directory(&self) -> bool {
        matches!(self.kind, EntryKind::Directory { .. })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FolderState {
    Remembered,
    AllExpanded,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TreeCursor {
    RevealSelected,
    KeepCurrent,
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<OsString, Directory>,
    files: BTreeMap<OsString, ChangeIndex>,
}

#[derive(Default)]
pub struct FileTree {
    pub entries: Vec<Entry>,
    pub state: ListState,
    collapsed_directories: BTreeSet<PathBuf>,
}

impl FileTree {
    pub fn current(&self) -> Option<&Entry> {
        self.state
            .selected()
            .and_then(|index| self.entries.get(index))
    }

    pub fn rebuild(
        &mut self,
        changes: &[Change],
        visible: &[ChangeIndex],
        folders: FolderState,
        selected: Option<&Path>,
        cursor: TreeCursor,
    ) {
        let keep = match cursor {
            TreeCursor::RevealSelected => {
                if let Some(selected) = selected {
                    self.collapsed_directories
                        .retain(|directory| !selected.starts_with(directory));
                }
                selected.map(Path::to_owned)
            }
            TreeCursor::KeepCurrent => self
                .current()
                .map(|entry| entry.path.clone())
                .or_else(|| selected.map(Path::to_owned)),
        };
        self.entries.clear();
        self.append_directory(&directory_tree(changes, visible), Path::new(""), 0, folders);
        let index = keep
            .as_deref()
            .and_then(|path| self.position_of(path))
            .or_else(|| self.state.selected())
            .unwrap_or(0)
            .min(self.entries.len().saturating_sub(1));
        self.state
            .select((!self.entries.is_empty()).then_some(index));
    }

    fn position_of(&self, path: &Path) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.path == path)
            .or_else(|| {
                self.entries
                    .iter()
                    .rposition(|entry| path.starts_with(&entry.path))
            })
    }

    fn append_directory(
        &mut self,
        directory: &Directory,
        parent: &Path,
        depth: usize,
        folders: FolderState,
    ) {
        for (name, child) in &directory.directories {
            let mut label = PathBuf::from(name);
            let mut child = child;
            while child.files.is_empty()
                && child.directories.len() == 1
                && let Some((name, only_directory)) = child.directories.first_key_value()
            {
                label.push(name);
                child = only_directory;
            }
            if child.directories.is_empty()
                && child.files.len() == 1
                && let Some((name, &change)) = child.files.first_key_value()
            {
                label.push(name);
                self.entries.push(Entry {
                    path: parent.join(&label),
                    label: escape_controls(&label.to_string_lossy()),
                    depth,
                    kind: EntryKind::File(change),
                });
                continue;
            }
            let path = parent.join(&label);
            let collapsed =
                folders == FolderState::Remembered && self.collapsed_directories.contains(&path);
            self.entries.push(Entry {
                path: path.clone(),
                label: format!("{}/", escape_controls(&label.to_string_lossy())),
                depth,
                kind: EntryKind::Directory { collapsed },
            });
            if !collapsed {
                self.append_directory(child, &path, depth + 1, folders);
            }
        }
        for (name, &change) in &directory.files {
            self.entries.push(Entry {
                path: parent.join(name),
                label: escape_controls(&name.to_string_lossy()),
                depth,
                kind: EntryKind::File(change),
            });
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if let Some(last) = self.entries.len().checked_sub(1) {
            self.state.select(Some(
                self.state
                    .selected()
                    .unwrap_or(0)
                    .saturating_add_signed(delta)
                    .min(last),
            ));
        }
    }

    pub fn expand(&mut self) {
        if let Some(entry) = self
            .state
            .selected()
            .and_then(|index| self.entries.get(index))
        {
            self.collapsed_directories.remove(&entry.path);
        }
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(entry) = self
            .state
            .selected()
            .and_then(|index| self.entries.get(index))
        else {
            return;
        };
        if entry.kind == (EntryKind::Directory { collapsed: false }) {
            self.collapsed_directories.insert(entry.path.clone());
        } else if let Some(parent) = self.entries.iter().rposition(|candidate| {
            candidate.is_directory()
                && candidate.path != entry.path
                && entry.path.starts_with(&candidate.path)
        }) {
            self.state.select(Some(parent));
        }
    }
}

fn directory_tree(changes: &[Change], visible: &[ChangeIndex]) -> Directory {
    let mut root = Directory::default();
    for &index in visible {
        let path = &changes[index.0].path;
        let mut directory = &mut root;
        for part in path.parent().into_iter().flat_map(Path::components) {
            directory = directory
                .directories
                .entry(part.as_os_str().to_owned())
                .or_default();
        }
        if let Some(name) = path.file_name() {
            directory.files.insert(name.to_owned(), index);
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeStatus;

    fn modified(paths: &[&str]) -> (Vec<Change>, Vec<ChangeIndex>) {
        let changes = paths
            .iter()
            .map(|path| Change {
                path: path.into(),
                status: ChangeStatus::Modified,
                old_path: None,
            })
            .collect();
        (changes, (0..paths.len()).map(ChangeIndex).collect())
    }

    #[test]
    fn compact_directories_keep_full_paths_and_preserve_collapsed_state() {
        let (changes, visible) = modified(&[
            "project/src/main/web/Service.kt",
            "project/src/main/api/Service.kt",
        ]);
        let mut tree = FileTree::default();
        let (remembered, keep) = (FolderState::Remembered, TreeCursor::KeepCurrent);
        tree.rebuild(&changes, &visible, remembered, None, keep);
        assert_eq!(tree.entries[0].label, "project/src/main/");
        assert_eq!(
            tree.entries
                .iter()
                .filter(|entry| entry.change().is_some())
                .count(),
            2
        );
        tree.collapse_or_parent();
        tree.rebuild(&changes, &visible, remembered, None, keep);
        assert_eq!(tree.entries.len(), 1);
        assert_eq!(
            tree.current().unwrap().kind,
            EntryKind::Directory { collapsed: true }
        );
        tree.rebuild(
            &changes,
            &visible,
            FolderState::AllExpanded,
            Some(&changes[0].path),
            TreeCursor::RevealSelected,
        );
        assert_eq!(tree.current().unwrap().path, changes[0].path);
        tree.collapse_or_parent();
        assert_eq!(tree.current().unwrap().label, "project/src/main/");
    }

    #[test]
    fn directory_with_single_file_is_merged_into_file_entry() {
        let (changes, visible) = modified(&[
            "src/Main.kt",
            "src/api/Client.kt",
            "src/api/Service.kt",
            "src/web/Service.kt",
        ]);
        let mut tree = FileTree::default();
        tree.rebuild(
            &changes,
            &visible,
            FolderState::Remembered,
            None,
            TreeCursor::KeepCurrent,
        );
        let labels: Vec<_> = tree
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "src/",
                "api/",
                "Client.kt",
                "Service.kt",
                "web/Service.kt",
                "Main.kt",
            ]
        );
        assert_eq!(tree.entries[4].path, changes[3].path);
        assert_eq!(tree.entries[4].change(), Some(ChangeIndex(3)));
    }
}
