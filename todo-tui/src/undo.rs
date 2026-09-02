use std::collections::VecDeque;

use crate::db::{Project, Todo};

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) projects: Vec<Project>,
    pub(crate) todos: Vec<Todo>,
    pub(crate) data_version: i64,
}

pub(crate) struct UndoHistory {
    snapshots: VecDeque<Snapshot>,
    capacity: usize,
}

impl UndoHistory {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            snapshots: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub(crate) fn remember(&mut self, snapshot: Snapshot) {
        if self.snapshots.len() >= self.capacity {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    pub(crate) fn latest(&self) -> Option<&Snapshot> {
        self.snapshots.back()
    }

    pub(crate) fn discard_latest(&mut self) {
        self.snapshots.pop_back();
    }

    pub(crate) fn clear(&mut self) {
        self.snapshots.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.snapshots.len()
    }
}
