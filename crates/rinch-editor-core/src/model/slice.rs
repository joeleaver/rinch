//! A [`Slice`]: a piece of document content with "open" depths at each end.
//!
//! When you cut a range out of the document (for copy, or as the replacement in a
//! `ReplaceStep`), the cut may pass through the middle of nodes. `open_start` /
//! `open_end` record how many node boundaries are left open at each side, so the
//! slice can be re-joined into a destination at the right depth. A fully "flat"
//! slice (e.g. a run of text or whole blocks) has open depths of 0.
//!
//! The transform engine (M2) uses `Slice` as the payload of `ReplaceStep`; copy
//! and paste (M6) serialize to/from it.

use crate::model::{Fragment, Node};

/// A slice of document content with open boundary depths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
    /// The content of the slice.
    pub content: Fragment,
    /// Number of open node boundaries at the start.
    pub open_start: usize,
    /// Number of open node boundaries at the end.
    pub open_end: usize,
}

impl Slice {
    /// The empty slice.
    pub fn empty() -> Slice {
        Slice {
            content: Fragment::empty(),
            open_start: 0,
            open_end: 0,
        }
    }

    /// A slice with explicit open depths.
    pub fn new(content: Fragment, open_start: usize, open_end: usize) -> Slice {
        Slice {
            content,
            open_start,
            open_end,
        }
    }

    /// A flat slice (open depths 0) from a fragment — e.g. a run of inline
    /// content or a sequence of whole blocks.
    pub fn from_fragment(content: Fragment) -> Slice {
        Slice {
            content,
            open_start: 0,
            open_end: 0,
        }
    }

    /// The size this slice will occupy when inserted (content size minus the open
    /// boundary tokens at each end).
    pub fn size(&self) -> usize {
        self.content
            .size()
            .saturating_sub(self.open_start)
            .saturating_sub(self.open_end)
    }

    /// True if the slice has no content.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Insert `fragment` into this slice's content at the (content) offset `pos`,
    /// descending into open nodes as needed. Returns `None` if it cannot fit.
    /// Used by `ReplaceAroundStep` to drop the preserved gap content into the
    /// wrapping slice. Port of `Slice.insertAt`.
    pub fn insert_at(&self, pos: usize, fragment: &Fragment) -> Option<Slice> {
        let content = insert_into(&self.content, pos + self.open_start, fragment)?;
        Some(Slice::new(content, self.open_start, self.open_end))
    }

    /// Remove the (content) range `from..to` from this slice's content, descending
    /// into open nodes. The range must be flat (not cross a node boundary at an
    /// unequal depth). Used by `ReplaceAroundStep::invert`. Port of
    /// `Slice.removeBetween`.
    pub fn remove_between(&self, from: usize, to: usize) -> Slice {
        Slice::new(
            remove_range(&self.content, from + self.open_start, to + self.open_start),
            self.open_start,
            self.open_end,
        )
    }
}

/// Recursive helper for [`Slice::insert_at`]. Port of `insertInto` (the `parent`
/// argument is always absent in this call chain, so the `canReplace` check PM
/// guards with it is a no-op and omitted; schema validity is enforced later by the
/// replace's `close`).
fn insert_into(content: &Fragment, dist: usize, insert: &Fragment) -> Option<Fragment> {
    let (index, offset) = content.find_index(dist);
    let child = content.maybe_child(index);
    if offset == dist || child.is_some_and(Node::is_text) {
        Some(
            content
                .cut(0, dist)
                .append(insert)
                .append(&content.cut(dist, content.size())),
        )
    } else {
        let child = child?;
        let inner = insert_into(child.content(), dist - offset - 1, insert)?;
        Some(content.replace_child(index, child.copy_with_content(inner)))
    }
}

/// Recursive helper for [`Slice::remove_between`]. Port of `removeRange`.
fn remove_range(content: &Fragment, from: usize, to: usize) -> Fragment {
    let (index, offset) = content.find_index(from);
    let child = content.maybe_child(index);
    let (index_to, offset_to) = content.find_index(to);
    if offset == from || child.is_some_and(Node::is_text) {
        debug_assert!(
            offset_to == to || content.child(index_to).is_text(),
            "remove_range: non-flat range"
        );
        return content
            .cut(0, from)
            .append(&content.cut(to, content.size()));
    }
    debug_assert!(index == index_to, "remove_range: non-flat range");
    let child = content.child(index);
    content.replace_child(
        index,
        child.copy_with_content(remove_range(
            child.content(),
            from - offset - 1,
            to - offset - 1,
        )),
    )
}

impl Default for Slice {
    fn default() -> Self {
        Slice::empty()
    }
}
