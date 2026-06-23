//! [`NodeRange`] — a range of sibling block nodes, the unit `wrap`/`lift` act on.
//!
//! A `NodeRange` is two resolved positions that share a common ancestor at a
//! given `depth`; the range covers the children of that ancestor between the two
//! positions. [`block_range`] finds the shallowest such range satisfying a
//! predicate. Port of ProseMirror's `NodeRange` and `ResolvedPos.blockRange`.

use crate::model::Node;
use crate::pos::ResolvedPos;

/// A contiguous range of sibling nodes within a common parent.
#[derive(Clone, Debug)]
pub struct NodeRange {
    /// The resolved start position.
    pub from: ResolvedPos,
    /// The resolved end position.
    pub to: ResolvedPos,
    /// The depth of the common parent.
    pub depth: usize,
}

impl NodeRange {
    /// The document position before the first node in the range.
    ///
    /// Total: if `from` resolves shallower than `depth + 1` (a degenerate range
    /// whose `from` lands on a boundary above the common depth), this falls back to
    /// `from.pos()` rather than indexing the resolved path out of bounds. A
    /// well-formed block range (both endpoints inside content) never hits the
    /// fallback.
    pub fn start(&self) -> usize {
        if self.depth + 1 > self.from.depth() {
            self.from.pos().0
        } else {
            self.from.before(self.depth + 1).expect("node range start")
        }
    }

    /// The document position after the last node in the range.
    ///
    /// Total: if `to` resolves shallower than `depth + 1` (e.g. the selection head
    /// lands on a gap above the common depth), this falls back to `to.pos()` rather
    /// than indexing the resolved path out of bounds.
    pub fn end(&self) -> usize {
        if self.depth + 1 > self.to.depth() {
            self.to.pos().0
        } else {
            self.to.after(self.depth + 1).expect("node range end")
        }
    }

    /// The common parent node.
    pub fn parent(&self) -> &Node {
        self.from.node(self.depth)
    }

    /// The index of the first node in the parent.
    pub fn start_index(&self) -> usize {
        self.from.index(self.depth)
    }

    /// The index just past the last node in the parent.
    pub fn end_index(&self) -> usize {
        self.to.index_after(self.depth)
    }
}

/// Find the shallowest [`NodeRange`] containing both positions whose parent
/// satisfies `pred`. `from`/`to` must already be ordered (`from.pos <= to.pos`).
/// Port of `ResolvedPos.blockRange`.
pub fn block_range(
    from: &ResolvedPos,
    to: &ResolvedPos,
    pred: impl Fn(&Node) -> bool,
) -> Option<NodeRange> {
    let adjust = usize::from(from.parent().is_textblock() || from.pos() == to.pos());
    let mut d = from.depth() as isize - adjust as isize;
    while d >= 0 {
        let du = d as usize;
        if to.pos().0 <= from.end(du) && pred(from.node(du)) {
            return Some(NodeRange {
                from: from.clone(),
                to: to.clone(),
                depth: du,
            });
        }
        d -= 1;
    }
    None
}

/// Convenience: the shallowest block range covering both positions (no predicate).
pub fn block_range_simple(from: &ResolvedPos, to: &ResolvedPos) -> Option<NodeRange> {
    block_range(from, to, |_| true)
}
