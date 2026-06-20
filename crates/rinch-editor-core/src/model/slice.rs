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

use crate::model::Fragment;

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
}

impl Default for Slice {
    fn default() -> Self {
        Slice::empty()
    }
}
