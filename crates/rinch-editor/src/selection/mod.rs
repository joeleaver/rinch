//! Selection management.

mod state;

pub use state::SelectionState;

use crate::document::{Position, Range};

/// Selection represents the current cursor position and/or selected range.
#[derive(Debug, Clone)]
pub struct Selection {
    /// The anchor position (where selection started)
    pub anchor: Position,
    /// The head position (current cursor position)
    pub head: Position,
}

impl Selection {
    /// Create a new selection.
    pub fn new(anchor: Position, head: Position) -> Self {
        Self { anchor, head }
    }

    /// Create a cursor (zero-width selection).
    pub fn cursor(pos: Position) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// Get the selection as a range.
    pub fn range(&self) -> Range {
        Range::new(self.start(), self.end())
    }

    /// Get the start position (min of anchor and head).
    pub fn start(&self) -> Position {
        self.anchor.min(self.head)
    }

    /// Get the end position (max of anchor and head).
    pub fn end(&self) -> Position {
        self.anchor.max(self.head)
    }

    /// Check if this is a cursor (no selection).
    pub fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    /// Check if the selection is empty.
    pub fn is_empty(&self) -> bool {
        self.is_cursor()
    }
}
