use std::collections::VecDeque;

/// Bounded undo stack that forgets the oldest entry once full.
pub struct UndoHistory<T> {
    entries: VecDeque<T>,
    capacity: usize,
}

impl<T> UndoHistory<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn remember(&mut self, entry: T) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn latest(&self) -> Option<&T> {
        self.entries.back()
    }

    pub fn discard_latest(&mut self) -> Option<T> {
        self.entries.pop_back()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_most_recent_entries() {
        let mut history = UndoHistory::new(2);
        for entry in 1..=3 {
            history.remember(entry);
        }
        assert_eq!(history.discard_latest(), Some(3));
        assert_eq!(history.discard_latest(), Some(2));
        assert_eq!(history.discard_latest(), None);
    }
}
