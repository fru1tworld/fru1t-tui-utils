use std::iter;

use crate::db::{Todo, TodoId};

pub(crate) const MAX_DEPTH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildProgress {
    pub(crate) done: usize,
    pub(crate) total: usize,
}

pub(crate) fn find(todos: &[Todo], id: TodoId) -> Option<&Todo> {
    todos.iter().find(|t| t.id == id)
}

pub(crate) fn children(todos: &[Todo], id: TodoId) -> impl Iterator<Item = &Todo> {
    todos.iter().filter(move |c| c.parent_id == Some(id))
}

pub(crate) fn ancestors(todos: &[Todo], id: TodoId) -> impl Iterator<Item = &Todo> {
    iter::successors(find(todos, id), |t| {
        t.parent_id.and_then(|parent| find(todos, parent))
    })
    .skip(1)
}

pub(crate) fn depth(todos: &[Todo], id: TodoId) -> usize {
    ancestors(todos, id).count()
}

pub(crate) fn accepts_child(todos: &[Todo], id: TodoId) -> bool {
    depth(todos, id) + 1 < MAX_DEPTH
}

pub(crate) fn descendant_ids(todos: &[Todo], id: TodoId) -> Vec<TodoId> {
    children(todos, id)
        .flat_map(|c| iter::once(c.id).chain(descendant_ids(todos, c.id)))
        .collect()
}

pub(crate) fn subtree_height(todos: &[Todo], id: TodoId) -> usize {
    1 + children(todos, id)
        .map(|c| subtree_height(todos, c.id))
        .max()
        .unwrap_or(0)
}

pub(crate) fn child_progress(todos: &[Todo], id: TodoId) -> ChildProgress {
    children(todos, id).fold(ChildProgress { done: 0, total: 0 }, |p, c| ChildProgress {
        done: p.done + usize::from(c.done),
        total: p.total + 1,
    })
}

pub(crate) fn completion_updates(todos: &[Todo], id: TodoId, done: bool) -> Vec<(TodoId, bool)> {
    let reopened_ancestors = (!done).then(|| ancestors(todos, id).map(|a| (a.id, false)));
    iter::once((id, done))
        .chain(descendant_ids(todos, id).into_iter().map(|d| (d, done)))
        .chain(reopened_ancestors.into_iter().flatten())
        .collect()
}
