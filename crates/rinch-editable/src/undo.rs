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
}

impl<T: Invertible> UndoStack<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            max_size,
        }
    }

    pub fn push(&mut self, op: T) {
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
        }
    }
}
