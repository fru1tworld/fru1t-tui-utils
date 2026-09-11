use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use ratatui::widgets::ListState;

use crate::git::{Change, clean};

pub struct Entry {
    pub path: PathBuf,
    pub label: String,
    pub depth: usize,
    pub change: Option<usize>,
    pub collapsed: bool,
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<OsString, Directory>,
    files: BTreeMap<OsString, usize>,
}

#[derive(Default)]
pub struct FileTree {
    pub entries: Vec<Entry>,
    pub state: ListState,
    closed: BTreeSet<PathBuf>,
}

impl FileTree {
    pub fn current(&self) -> Option<&Entry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    pub fn rebuild(
        &mut self,
        changes: &[Change],
        visible: &[usize],
        searching: bool,
        selected: Option<&Path>,
        reveal: bool,
    ) {
        let keep = if reveal {
            if let Some(selected) = selected {
                self.closed.retain(|path| !selected.starts_with(path));
            }
            selected.map(Path::to_owned)
        } else {
            self.current()
                .map(|entry| entry.path.clone())
                .or_else(|| selected.map(Path::to_owned))
        };
        let mut root = Directory::default();
        for &index in visible {
            let path = &changes[index].path;
            let mut directory = &mut root;
            if let Some(parent) = path.parent() {
                for part in parent.components() {
                    directory = directory
                        .directories
                        .entry(part.as_os_str().to_owned())
                        .or_default();
                }
            }
            if let Some(name) = path.file_name() {
                directory.files.insert(name.to_owned(), index);
            }
        }
        self.entries.clear();
        self.append(&root, Path::new(""), 0, searching);
        let index = keep
            .as_ref()
            .and_then(|path| {
                self.entries
                    .iter()
                    .position(|entry| entry.path == *path)
                    .or_else(|| {
                        self.entries
                            .iter()
                            .rposition(|entry| path.starts_with(&entry.path))
                    })
            })
            .or_else(|| self.state.selected())
            .unwrap_or(0)
            .min(self.entries.len().saturating_sub(1));
        self.state
            .select((!self.entries.is_empty()).then_some(index));
    }

    fn append(&mut self, directory: &Directory, parent: &Path, depth: usize, searching: bool) {
        for (name, child) in &directory.directories {
            let mut path = parent.join(name);
            let mut child = child;
            while child.files.is_empty() && child.directories.len() == 1 {
                let (name, next) = child.directories.first_key_value().unwrap();
                path.push(name);
                child = next;
            }
            let collapsed = !searching && self.closed.contains(&path);
            self.entries.push(Entry {
                label: format!(
                    "{}/",
                    clean(&path.strip_prefix(parent).unwrap().to_string_lossy())
                ),
                path: path.clone(),
                depth,
                change: None,
                collapsed,
            });
            if !collapsed {
                self.append(child, &path, depth + 1, searching);
            }
        }
        for (name, &change) in &directory.files {
            self.entries.push(Entry {
                path: parent.join(name),
                label: clean(&name.to_string_lossy()),
                depth,
                change: Some(change),
                collapsed: false,
            });
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if !self.entries.is_empty() {
            self.state.select(Some(
                self.state
                    .selected()
                    .unwrap_or(0)
                    .saturating_add_signed(delta)
                    .min(self.entries.len() - 1),
            ));
        }
    }

    pub fn expand(&mut self) {
        if let Some(entry) = self.current() {
            self.closed.remove(&entry.path.clone());
        }
    }

    pub fn collapse_or_parent(&mut self) {
        if let Some(entry) = self.current() {
            if entry.change.is_none() && !entry.collapsed {
                self.closed.insert(entry.path.clone());
            } else {
                let parent = self.entries.iter().rposition(|candidate| {
                    candidate.change.is_none()
                        && candidate.path != entry.path
                        && entry.path.starts_with(&candidate.path)
                });
                if let Some(parent) = parent {
                    self.state.select(Some(parent));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_directories_keep_full_paths_and_preserve_collapsed_state() {
        let changes: Vec<_> = [
            "subproject/src/main/checkout/Service.kt",
            "subproject/src/main/billing/Service.kt",
        ]
        .into_iter()
        .map(|path| Change {
            path: path.into(),
            status: 'M',
            old_path: None,
        })
        .collect();
        let mut tree = FileTree::default();
        tree.rebuild(&changes, &[0, 1], false, None, false);
        assert_eq!(tree.entries[0].label, "subproject/src/main/");
        assert_eq!(
            tree.entries
                .iter()
                .filter(|entry| entry.change.is_some())
                .count(),
            2
        );
        tree.collapse_or_parent();
        tree.rebuild(&changes, &[0, 1], false, None, false);
        assert_eq!(tree.entries.len(), 1);
        assert!(tree.current().unwrap().collapsed);
        tree.rebuild(&changes, &[0], true, Some(&changes[0].path), true);
        assert_eq!(tree.current().unwrap().path, changes[0].path);
        tree.collapse_or_parent();
        assert_eq!(
            tree.current().unwrap().label,
            "subproject/src/main/checkout/"
        );
    }
}
