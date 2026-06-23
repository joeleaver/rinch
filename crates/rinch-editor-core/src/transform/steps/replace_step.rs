//! [`ReplaceStep`] — replace a range with a slice. The workhorse step.
//!
//! Text insertion, deletion, splitting, joining, and paste all reduce to a
//! `ReplaceStep`. It is its own inverse family (a replace inverts to another
//! replace) and merges with adjacent replaces so that typing groups into one
//! undo step.

use crate::Slice;
use crate::model::Node;
use crate::pos::Pos;
use crate::transform::replace::replace_doc;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};
use std::any::Any;

/// Replace the document range `from..to` with `slice`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceStep {
    /// Start of the replaced range.
    pub from: usize,
    /// End of the replaced range.
    pub to: usize,
    /// The content to insert in its place.
    pub slice: Slice,
    /// When true this is a *structural* replace (e.g. a split): it refuses to run
    /// if it would overwrite real content, and never merges with a neighbour.
    pub structure: bool,
}

impl ReplaceStep {
    /// A content replace (`structure = false`) — text insert/delete/paste.
    pub fn new(from: usize, to: usize, slice: Slice) -> ReplaceStep {
        ReplaceStep {
            from,
            to,
            slice,
            structure: false,
        }
    }

    /// A structural replace (`structure = true`) — split/join/boundary edits.
    pub fn structural(from: usize, to: usize, slice: Slice) -> ReplaceStep {
        ReplaceStep {
            from,
            to,
            slice,
            structure: true,
        }
    }
}

impl Step for ReplaceStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        if self.structure && content_between(doc, self.from, self.to) {
            return Err(StepError::new("structure replace would overwrite content"));
        }
        replace_doc(doc, self.from, self.to, &self.slice)
    }

    fn get_map(&self) -> StepMap {
        StepMap::new(vec![self.from, self.to - self.from, self.slice.size()])
    }

    fn invert(&self, doc: &Node) -> Box<dyn Step> {
        // The forward step's slice now occupies `from .. from + slice.size`; the
        // inverse puts back what was originally in `from .. to`. The positions are
        // valid in `doc` by contract (this is the doc the step was applied to).
        let recovered = doc
            .slice(self.from, self.to)
            .expect("invert: original range must be valid in the pre-step document");
        Box::new(ReplaceStep::new(
            self.from,
            self.from + self.slice.size(),
            recovered,
        ))
    }

    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let from = mapping.map_result(self.from, 1);
        let to = mapping.map_result(self.to, -1);
        if from.deleted_across() && to.deleted_across() {
            return None;
        }
        Some(Box::new(ReplaceStep {
            from: from.pos,
            to: from.pos.max(to.pos),
            slice: self.slice.clone(),
            structure: self.structure,
        }))
    }

    fn merge(&self, other: &dyn Step) -> Option<Box<dyn Step>> {
        let other = other.as_any().downcast_ref::<ReplaceStep>()?;
        if other.structure || self.structure {
            return None;
        }
        if self.from + self.slice.size() == other.from
            && self.slice.open_end == 0
            && other.slice.open_start == 0
        {
            // adjacent inserts (typing) — append the two slices
            let slice = if self.slice.size() + other.slice.size() == 0 {
                Slice::empty()
            } else {
                Slice::new(
                    self.slice.content.append(&other.slice.content),
                    self.slice.open_start,
                    other.slice.open_end,
                )
            };
            Some(Box::new(ReplaceStep {
                from: self.from,
                to: self.to + (other.to - other.from),
                slice,
                structure: self.structure,
            }))
        } else if other.to == self.from && self.slice.open_start == 0 && other.slice.open_end == 0 {
            // adjacent deletes
            let slice = if self.slice.size() + other.slice.size() == 0 {
                Slice::empty()
            } else {
                Slice::new(
                    other.slice.content.append(&self.slice.content),
                    other.slice.open_start,
                    self.slice.open_end,
                )
            };
            Some(Box::new(ReplaceStep {
                from: other.from,
                to: self.to,
                slice,
                structure: self.structure,
            }))
        } else {
            None
        }
    }

    fn clone_box(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// True if `from..to` spans any real (non-boundary) content — used by structural
/// replaces to refuse overwriting content. Port of `contentBetween`.
fn content_between(doc: &Node, from: usize, to: usize) -> bool {
    let r_from = match doc.resolve(Pos(from)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let mut dist = to as isize - from as isize;
    let mut depth = r_from.depth();
    while dist > 0 && depth > 0 && r_from.index_after(depth) == r_from.node(depth).child_count() {
        depth -= 1;
        dist -= 1;
    }
    if dist > 0 {
        let mut next = r_from
            .node(depth)
            .content()
            .maybe_child(r_from.index_after(depth))
            .cloned();
        while dist > 0 {
            let n = match &next {
                None => return true,
                Some(n) => n,
            };
            if n.is_leaf() {
                return true;
            }
            next = n.content().maybe_child(0).cloned();
            dist -= 1;
        }
    }
    false
}
