use crate::{Position, Range};

/// Text selection with anchor and head positions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Where selection started (fixed point)
    pub anchor: Position,
    /// Current cursor position (moves with shift+arrow)
    pub head: Position,
}

impl Selection {
    pub fn cursor(pos: impl Into<Position>) -> Self {
        let pos = pos.into();
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn new(anchor: impl Into<Position>, head: impl Into<Position>) -> Self {
        Self {
            anchor: anchor.into(),
            head: head.into(),
        }
    }

    pub fn range(&self) -> Range {
        Range::new(self.start(), self.end())
    }

    pub fn start(&self) -> Position {
        if self.anchor <= self.head {
            self.anchor
        } else {
            self.head
        }
    }

    pub fn end(&self) -> Position {
        if self.anchor <= self.head {
            self.head
        } else {
            self.anchor
        }
    }

    pub fn is_cursor(&self) -> bool {
        self.anchor == self.head
    }

    /// Check if the selection is empty (alias for is_cursor).
    pub fn is_empty(&self) -> bool {
        self.is_cursor()
    }

    pub fn is_forward(&self) -> bool {
        self.anchor <= self.head
    }
}
