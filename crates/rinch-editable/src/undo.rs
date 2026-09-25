/// Trait for operations that can be inverted for undo.
pub trait Invertible: Clone {
    fn inverse(&self) -> Self;
}

use std::collections::VecDeque;

/// Generic undo/redo stack.
#[derive(Debug)]
pub struct UndoStack<T> {
    undo: VecDeque<T>,
    redo: VecDeque<T>,
    max_size: usize,
    /// Entries pushed and not taken back by [`Self::take_since`]: what an
    /// [`UndoMark`] records. Counted net so that a group folded inside another
    /// (a command's own ops, then the command and a host's rewrite) leaves the
    /// outer mark counting the one entry it became.
    pushes: u64,
}

/// A point in an [`UndoStack`]'s history, taken with [`UndoStack::mark`], so
/// that everything pushed after it can be taken back with
/// [`UndoStack::take_since`] and pushed again as one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UndoMark(u64);

impl<T: Invertible> UndoStack<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            max_size,
            pushes: 0,
        }
    }

    pub fn push(&mut self, op: T) {
        self.pushes += 1;
        self.undo.push_back(op);
        self.redo.clear(); // Clear redo stack on new operation

        // Limit size
        while self.undo.len() > self.max_size {
            self.undo.pop_front();
        }
    }

    pub fn undo(&mut self) -> Option<T> {
        let op = self.undo.pop_back()?;
        let inverse = op.inverse();
        self.redo.push_back(op);
        Some(inverse)
    }

    pub fn redo(&mut self) -> Option<T> {
        let op = self.redo.pop_back()?;
        self.undo.push_back(op.clone());
        Some(op)
    }

    /// The current point in the history. Only [`Self::push`] and
    /// [`Self::take_since`] move it — undo and redo do not.
    pub fn mark(&self) -> UndoMark {
        UndoMark(self.pushes)
    }

    /// Remove and return, oldest first, the entries pushed since `mark` — at
    /// most as many as the stack still holds, since the size limit may have
    /// dropped some from the front. Empty when nothing was pushed.
    pub fn take_since(&mut self, mark: UndoMark) -> Vec<T> {
        let pushed = self.pushes.saturating_sub(mark.0);
        let n = usize::try_from(pushed)
            .unwrap_or(usize::MAX)
            .min(self.undo.len());
        self.pushes -= n as u64;
        self.undo.split_off(self.undo.len() - n).into()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

impl<T> Default for UndoStack<T> {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            max_size: 1000,
            pushes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Op(u32);
    impl Invertible for Op {
        fn inverse(&self) -> Self {
            Op(self.0 + 100)
        }
    }

    #[test]
    fn take_since_returns_what_was_pushed_oldest_first() {
        let mut s = UndoStack::new(10);
        s.push(Op(1));
        let mark = s.mark();
        s.push(Op(2));
        s.push(Op(3));
        assert_eq!(s.take_since(mark), [Op(2), Op(3)]);
        assert_eq!(s.undo(), Some(Op(101)));
        assert_eq!(s.take_since(s.mark()), []);
    }

    /// The size limit may drop entries from the front while a group is open;
    /// the take is bounded by what the stack still holds.
    #[test]
    fn take_since_is_bounded_by_the_size_limit() {
        let mut s = UndoStack::new(2);
        let mark = s.mark();
        s.push(Op(1));
        s.push(Op(2));
        s.push(Op(3));
        assert_eq!(s.take_since(mark), [Op(2), Op(3)]);
        assert!(!s.can_undo());
    }
}
