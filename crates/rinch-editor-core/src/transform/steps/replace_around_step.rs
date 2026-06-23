//! [`ReplaceAroundStep`] — replace around a preserved gap.
//!
//! Wrapping (blockquote, list), lifting, and node-type changes that re-parent
//! content all keep an inner range of content while changing the structure around
//! it. `ReplaceAroundStep` replaces `from..to` with `slice`, but first drops the
//! content currently in the *gap* `gap_from..gap_to` into the slice at offset
//! `insert`. Port of ProseMirror's `ReplaceAroundStep`.

use crate::Slice;
use crate::model::Node;
use crate::transform::replace::replace_doc;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};
use std::any::Any;

/// Replace `from..to` with `slice`, preserving the gap `gap_from..gap_to` by
/// inserting it into `slice` at content offset `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceAroundStep {
    /// Start of the outer replaced range.
    pub from: usize,
    /// End of the outer replaced range.
    pub to: usize,
    /// Start of the preserved gap.
    pub gap_from: usize,
    /// End of the preserved gap.
    pub gap_to: usize,
    /// The wrapping content.
    pub slice: Slice,
    /// Content offset inside `slice` at which the gap content is inserted.
    pub insert: usize,
    /// Structural marker (these steps are always structural).
    pub structure: bool,
}

impl ReplaceAroundStep {
    /// Construct a replace-around step.
    pub fn new(
        from: usize,
        to: usize,
        gap_from: usize,
        gap_to: usize,
        slice: Slice,
        insert: usize,
        structure: bool,
    ) -> ReplaceAroundStep {
        ReplaceAroundStep {
            from,
            to,
            gap_from,
            gap_to,
            slice,
            insert,
            structure,
        }
    }
}

impl Step for ReplaceAroundStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let gap = doc
            .slice(self.gap_from, self.gap_to)
            .map_err(|e| StepError::new(e.to_string()))?;
        if gap.open_start != 0 || gap.open_end != 0 {
            return Err(StepError::new(
                "structure gap-replace would overwrite content",
            ));
        }
        let inserted = self
            .slice
            .insert_at(self.insert, &gap.content)
            .ok_or_else(|| StepError::new("content does not fit in gap"))?;
        replace_doc(doc, self.from, self.to, &inserted)
    }

    fn get_map(&self) -> StepMap {
        StepMap::new(vec![
            self.from,
            self.gap_from - self.from,
            self.insert,
            self.gap_to,
            self.to - self.gap_to,
            self.slice.size() - self.insert,
        ])
    }

    fn invert(&self, doc: &Node) -> Box<dyn Step> {
        let gap = self.gap_to - self.gap_from;
        let recovered = doc
            .slice(self.from, self.to)
            .expect("invert: outer range must be valid in the pre-step document")
            .remove_between(self.gap_from - self.from, self.gap_to - self.from);
        Box::new(ReplaceAroundStep {
            from: self.from,
            to: self.from + self.slice.size() + gap,
            gap_from: self.from + self.insert,
            gap_to: self.from + self.insert + gap,
            slice: recovered,
            insert: self.gap_from - self.from,
            structure: self.structure,
        })
    }

    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let from = mapping.map_result(self.from, 1);
        let to = mapping.map_result(self.to, -1);
        let gap_from = if self.from == self.gap_from {
            from.pos
        } else {
            mapping.map(self.gap_from, -1)
        };
        let gap_to = if self.to == self.gap_to {
            to.pos
        } else {
            mapping.map(self.gap_to, 1)
        };
        if (from.deleted_across() && to.deleted_across()) || gap_from < from.pos || gap_to > to.pos
        {
            return None;
        }
        Some(Box::new(ReplaceAroundStep {
            from: from.pos,
            to: to.pos,
            gap_from,
            gap_to,
            slice: self.slice.clone(),
            insert: self.insert,
            structure: self.structure,
        }))
    }

    fn clone_box(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
