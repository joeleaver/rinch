//! The position space.
//!
//! Positions into the document are a single integer space (amendment A2 / §3),
//! measured in chars inside text. [`Pos`] is the newtype that names a position so
//! it cannot be confused with a byte offset (which exists only transiently at
//! platform seams in the view layer). [`ResolvedPos`] is the result of resolving a
//! `Pos` against a document: the path of ancestor nodes, the depth, and the offset
//! within the immediate parent.

pub mod resolved;

pub use resolved::ResolvedPos;

/// A position in the document's single integer position space (char-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pos(pub usize);

impl Pos {
    /// The document start.
    pub const ZERO: Pos = Pos(0);

    /// The underlying integer.
    pub fn get(self) -> usize {
        self.0
    }
}

impl From<usize> for Pos {
    fn from(v: usize) -> Self {
        Pos(v)
    }
}
impl From<Pos> for usize {
    fn from(p: Pos) -> Self {
        p.0
    }
}
