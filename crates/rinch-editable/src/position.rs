/// A position in text (UTF-8 byte offset from start).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(pub usize);

impl Position {
    pub fn new(offset: usize) -> Self {
        Self(offset)
    }

    pub fn offset(&self) -> usize {
        self.0
    }
}

impl From<usize> for Position {
    fn from(offset: usize) -> Self {
        Self(offset)
    }
}

impl std::ops::Add<usize> for Position {
    type Output = Position;
    fn add(self, rhs: usize) -> Self::Output {
        Position(self.0 + rhs)
    }
}

impl std::ops::Sub<usize> for Position {
    type Output = Position;
    fn sub(self, rhs: usize) -> Self::Output {
        Position(self.0.saturating_sub(rhs))
    }
}

impl std::ops::Sub<Position> for Position {
    type Output = usize;
    fn sub(self, rhs: Position) -> Self::Output {
        self.0.saturating_sub(rhs.0)
    }
}

/// A range in text [start, end).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    /// Create a new range. Automatically normalizes so start <= end.
    pub fn new(start: impl Into<Position>, end: impl Into<Position>) -> Self {
        let start = start.into();
        let end = end.into();
        Self {
            start: std::cmp::min(start, end),
            end: std::cmp::max(start, end),
        }
    }

    pub fn collapsed(pos: impl Into<Position>) -> Self {
        let pos = pos.into();
        Self {
            start: pos,
            end: pos,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.start == self.end
    }

    pub fn len(&self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, pos: Position) -> bool {
        pos >= self.start && pos < self.end
    }

    /// Normalize so start <= end
    pub fn normalized(&self) -> Self {
        if self.start <= self.end {
            *self
        } else {
            Self {
                start: self.end,
                end: self.start,
            }
        }
    }

    /// Check if this range fully contains another range.
    pub fn contains_range(&self, other: &Range) -> bool {
        self.start <= other.start && self.end >= other.end
    }

    /// Check if this range overlaps with another range.
    pub fn overlaps(&self, other: &Range) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Get the intersection of two ranges, if any.
    pub fn intersection(&self, other: &Range) -> Option<Range> {
        let start = std::cmp::max(self.start, other.start);
        let end = std::cmp::min(self.end, other.end);
        if start < end {
            Some(Range { start, end })
        } else {
            None
        }
    }

    /// Get the union (bounding range) of two ranges.
    pub fn union(&self, other: &Range) -> Range {
        Range {
            start: std::cmp::min(self.start, other.start),
            end: std::cmp::max(self.end, other.end),
        }
    }
}
