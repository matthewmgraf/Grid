//! Undo history: where windows were before Grid snapped them.

use crate::geometry::Rect;

/// How many snaps Undo can step back through.
pub const UNDO_LIMIT: usize = 10;

/// The most recent snaps, newest last. `W` identifies a window (a handle on Windows).
#[derive(Debug, Clone)]
pub struct History<W> {
    entries: Vec<(W, Rect)>,
}

impl<W> Default for History<W> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<W: Copy> History<W> {
    /// Records that `window` was at `before` until it was snapped. The oldest entry is dropped
    /// once there are more than `UNDO_LIMIT`.
    pub fn push(&mut self, window: W, before: Rect) {
        self.entries.push((window, before));
        if self.entries.len() > UNDO_LIMIT {
            self.entries.remove(0);
        }
    }

    /// The newest entry whose window `exists`, dropping entries for windows that are gone.
    pub fn pop(&mut self, exists: impl Fn(W) -> bool) -> Option<(W, Rect)> {
        while let Some(entry) = self.entries.pop() {
            if exists(entry.0) {
                return Some(entry);
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_newest_first_and_skips_closed_windows() {
        let mut h = History::default();
        h.push(1, Rect::new(0, 0, 10, 10));
        h.push(2, Rect::new(5, 5, 10, 10));
        h.push(3, Rect::new(9, 9, 10, 10));
        assert_eq!(h.pop(|w| w != 3), Some((2, Rect::new(5, 5, 10, 10))));
        assert_eq!(h.pop(|_| true), Some((1, Rect::new(0, 0, 10, 10))));
        assert!(h.is_empty());
    }

    #[test]
    fn keeps_only_the_last_snaps() {
        let mut h = History::default();
        for i in 0..UNDO_LIMIT + 5 {
            h.push(i, Rect::default());
        }
        let mut popped = Vec::new();
        while let Some((w, _)) = h.pop(|_| true) {
            popped.push(w);
        }
        assert_eq!(popped.len(), UNDO_LIMIT);
        assert_eq!(popped.last(), Some(&5));
    }
}
