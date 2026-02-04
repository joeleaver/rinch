/// Trait for operations that can be inverted for undo.
pub trait Invertible: Clone {
    fn inverse(&self) -> Self;
}

/// Generic undo/redo stack.
#[derive(Debug)]
pub struct UndoStack<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    max_size: usize,
}

impl<T: Invertible> UndoStack<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_size,
        }
    }

    pub fn push(&mut self, op: T) {
        self.undo.push(op);
        self.redo.clear(); // Clear redo stack on new operation

        // Limit size
        while self.undo.len() > self.max_size {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self) -> Option<T> {
        let op = self.undo.pop()?;
        let inverse = op.inverse();
        self.redo.push(op);
        Some(inverse)
    }

    pub fn redo(&mut self) -> Option<T> {
        let op = self.redo.pop()?;
        self.undo.push(op.clone());
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
            undo: Vec::new(),
            redo: Vec::new(),
            max_size: 1000,
        }
    }
}
