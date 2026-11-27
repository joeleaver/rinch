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

use crate::model::Node;

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

/// Whether `from..to` of `doc` lies inside the content of one textblock: one walk
/// down from the top, a scan of each level's children (what a resolve scans too,
/// without building one).
pub(crate) fn inside_one_textblock(doc: &Node, from: usize, to: usize) -> bool {
    let (mut node, mut from, mut to) = (doc, from, to);
    loop {
        if node.is_textblock() {
            return true;
        }
        let content = node.content();
        if from > content.size() || to > content.size() {
            return false;
        }
        let (i, start) = content.find_index(from);
        let Some(child) = content.maybe_child(i) else {
            return false;
        };
        let end = start + child.node_size();
        if !(from > start && to < end) || child.is_leaf() {
            return false;
        }
        node = child;
        from -= start + 1;
        to -= start + 1;
    }
}
