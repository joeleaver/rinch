//! Document position and range types.

/// A position in the document (absolute character offset from start).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(pub usize);

impl Position {
    /// Create a new position.
    pub fn new(offset: usize) -> Self {
        Self(offset)
    }

    /// Get the offset value.
    pub fn offset(&self) -> usize {
        self.0
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

impl From<usize> for Position {
    fn from(offset: usize) -> Self {
        Self(offset)
    }
}

/// A range in the document (start inclusive, end exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Range {
    /// Start position (inclusive)
    pub start: Position,
    /// End position (exclusive)
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

    /// Create a collapsed (zero-width) range at the given position.
    pub fn collapsed(pos: impl Into<Position>) -> Self {
        let pos = pos.into();
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Check if the range is collapsed (cursor position).
    pub fn is_collapsed(&self) -> bool {
        self.start == self.end
    }

    /// Check if the range is empty (alias for is_collapsed).
    pub fn is_empty(&self) -> bool {
        self.is_collapsed()
    }

    /// Get the length of the range in characters.
    pub fn len(&self) -> usize {
        self.end.0 - self.start.0
    }

    /// Check if a position is contained in this range.
    pub fn contains(&self, pos: Position) -> bool {
        pos >= self.start && pos < self.end
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

/// A resolved position with block and inline coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPosition {
    /// Index of the block in the document
    pub block_index: usize,
    /// Index of the inline content within the block
    pub inline_index: usize,
    /// Character offset within the text content
    pub text_offset: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Position tests ----

    #[test]
    fn position_new_and_offset() {
        let p = Position::new(42);
        assert_eq!(p.offset(), 42);
        assert_eq!(p.0, 42);
    }

    #[test]
    fn position_from_usize() {
        let p: Position = 10.into();
        assert_eq!(p, Position(10));
    }

    #[test]
    fn position_ordering() {
        assert!(Position(0) < Position(1));
        assert!(Position(5) == Position(5));
        assert!(Position(10) > Position(3));
    }

    #[test]
    fn position_add() {
        assert_eq!(Position(5) + 3, Position(8));
        assert_eq!(Position(0) + 0, Position(0));
    }

    #[test]
    fn position_sub_usize() {
        assert_eq!(Position(5) - 3, Position(2));
        // Saturating: no underflow
        assert_eq!(Position(2) - 5, Position(0));
        assert_eq!(Position(0) - 1, Position(0));
    }

    #[test]
    fn position_sub_position() {
        let diff: usize = Position(10) - Position(3);
        assert_eq!(diff, 7);
        // Saturating
        let diff: usize = Position(3) - Position(10);
        assert_eq!(diff, 0);
    }

    // ---- Range tests ----

    #[test]
    fn range_new_normalizes() {
        let r = Range::new(Position(10), Position(5));
        assert_eq!(r.start, Position(5));
        assert_eq!(r.end, Position(10));
    }

    #[test]
    fn range_new_already_ordered() {
        let r = Range::new(2usize, 8usize);
        assert_eq!(r.start, Position(2));
        assert_eq!(r.end, Position(8));
    }

    #[test]
    fn range_collapsed() {
        let r = Range::collapsed(5usize);
        assert!(r.is_collapsed());
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn range_len() {
        assert_eq!(Range::new(3usize, 7usize).len(), 4);
        assert_eq!(Range::collapsed(0usize).len(), 0);
    }

    #[test]
    fn range_contains() {
        let r = Range::new(5usize, 10usize);
        assert!(r.contains(Position(5)));
        assert!(r.contains(Position(9)));
        assert!(!r.contains(Position(10))); // end is exclusive
        assert!(!r.contains(Position(4)));
    }

    #[test]
    fn range_contains_range() {
        let outer = Range::new(2usize, 10usize);
        let inner = Range::new(3usize, 8usize);
        assert!(outer.contains_range(&inner));
        assert!(!inner.contains_range(&outer));
        // Range contains itself
        assert!(outer.contains_range(&outer));
    }

    #[test]
    fn range_overlaps() {
        let a = Range::new(0usize, 5usize);
        let b = Range::new(3usize, 8usize);
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));

        // Adjacent ranges do not overlap
        let c = Range::new(5usize, 10usize);
        assert!(!a.overlaps(&c));

        // Collapsed ranges don't overlap
        let d = Range::collapsed(5usize);
        assert!(!a.overlaps(&d));
    }

    #[test]
    fn range_intersection() {
        let a = Range::new(0usize, 10usize);
        let b = Range::new(5usize, 15usize);
        let int = a.intersection(&b).unwrap();
        assert_eq!(int, Range::new(5usize, 10usize));

        // No intersection
        let c = Range::new(10usize, 20usize);
        assert!(a.intersection(&c).is_none());
    }

    #[test]
    fn range_union() {
        let a = Range::new(2usize, 5usize);
        let b = Range::new(8usize, 12usize);
        let u = a.union(&b);
        assert_eq!(u, Range::new(2usize, 12usize));
    }

    // ---- ResolvedPosition tests ----

    #[test]
    fn resolved_position_fields() {
        let rp = ResolvedPosition {
            block_index: 2,
            inline_index: 1,
            text_offset: 5,
        };
        assert_eq!(rp.block_index, 2);
        assert_eq!(rp.inline_index, 1);
        assert_eq!(rp.text_offset, 5);
    }
}
