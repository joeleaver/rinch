//! [`AddMarkStep`] / [`RemoveMarkStep`] — add or remove a mark across an inline
//! range.
//!
//! Both rebuild the inline content of the range with the mark added to / removed
//! from each text or inline-leaf node's mark set, then replace the range with the
//! re-marked copy. They induce no position change (their step map is empty) and
//! invert into each other. Port of ProseMirror's mark steps.

use crate::Slice;
use crate::model::{Fragment, Mark, Node};
use crate::pos::Pos;
use crate::transform::replace::replace_doc;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};
use std::any::Any;

/// Map every inline node in `fragment` through `f`, recursing into block content.
/// Port of `mapFragment` (the `i` index argument is unused here and dropped).
fn map_inline<F: Fn(&Node, &Node) -> Node>(fragment: &Fragment, parent: &Node, f: &F) -> Fragment {
    let mut mapped = Vec::with_capacity(fragment.child_count());
    for child in fragment.children() {
        let mut child = child.clone();
        if child.content_size() > 0 {
            let new_content = map_inline(child.content(), &child, f);
            child = child.copy_with_content(new_content);
        }
        if child.is_inline() {
            child = f(&child, parent);
        }
        mapped.push(child);
    }
    Fragment::from_children(mapped)
}

/// True for inline nodes that can carry marks (text and inline leaves/atoms) —
/// ProseMirror's `Node.isAtom` (`isLeaf || atom`).
fn markable(node: &Node) -> bool {
    node.is_leaf() || node.is_atom()
}

/// Add `mark` to every markable inline node in `from..to` (where the parent
/// permits it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddMarkStep {
    /// Start of the inline range.
    pub from: usize,
    /// End of the inline range.
    pub to: usize,
    /// The mark to add.
    pub mark: Mark,
}

impl AddMarkStep {
    /// Construct an add-mark step.
    pub fn new(from: usize, to: usize, mark: Mark) -> AddMarkStep {
        AddMarkStep { from, to, mark }
    }
}

impl Step for AddMarkStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let old_slice = doc
            .slice(self.from, self.to)
            .map_err(|e| StepError::new(e.to_string()))?;
        let r_from = doc
            .resolve(Pos(self.from))
            .map_err(|e| StepError::new(e.to_string()))?;
        let parent = r_from.node(r_from.shared_depth(Pos(self.to))).clone();
        let mark = self.mark.clone();
        let f = move |node: &Node, parent: &Node| -> Node {
            if !markable(node) || !parent.node_type().spec().marks.allows(mark.type_name()) {
                node.clone()
            } else {
                node.mark(mark.add_to_set(node.marks()))
            }
        };
        let content = map_inline(&old_slice.content, &parent, &f);
        let slice = Slice::new(content, old_slice.open_start, old_slice.open_end);
        replace_doc(doc, self.from, self.to, &slice)
    }

    fn get_map(&self) -> StepMap {
        StepMap::empty()
    }

    fn invert(&self, _doc: &Node) -> Box<dyn Step> {
        Box::new(RemoveMarkStep::new(self.from, self.to, self.mark.clone()))
    }

    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let from = mapping.map_result(self.from, 1);
        let to = mapping.map_result(self.to, -1);
        if (from.deleted() && to.deleted()) || from.pos >= to.pos {
            return None;
        }
        Some(Box::new(AddMarkStep::new(
            from.pos,
            to.pos,
            self.mark.clone(),
        )))
    }

    fn merge(&self, other: &dyn Step) -> Option<Box<dyn Step>> {
        let other = other.as_any().downcast_ref::<AddMarkStep>()?;
        if other.mark == self.mark && self.from <= other.to && self.to >= other.from {
            Some(Box::new(AddMarkStep::new(
                self.from.min(other.from),
                self.to.max(other.to),
                self.mark.clone(),
            )))
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

/// Remove `mark` from every inline node in `from..to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveMarkStep {
    /// Start of the inline range.
    pub from: usize,
    /// End of the inline range.
    pub to: usize,
    /// The mark to remove.
    pub mark: Mark,
}

impl RemoveMarkStep {
    /// Construct a remove-mark step.
    pub fn new(from: usize, to: usize, mark: Mark) -> RemoveMarkStep {
        RemoveMarkStep { from, to, mark }
    }
}

impl Step for RemoveMarkStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let old_slice = doc
            .slice(self.from, self.to)
            .map_err(|e| StepError::new(e.to_string()))?;
        let r_from = doc
            .resolve(Pos(self.from))
            .map_err(|e| StepError::new(e.to_string()))?;
        let parent = r_from.node(r_from.shared_depth(Pos(self.to))).clone();
        let mark = self.mark.clone();
        let f = move |node: &Node, _parent: &Node| -> Node {
            node.mark(mark.remove_from_set(node.marks()))
        };
        let content = map_inline(&old_slice.content, &parent, &f);
        let slice = Slice::new(content, old_slice.open_start, old_slice.open_end);
        replace_doc(doc, self.from, self.to, &slice)
    }

    fn get_map(&self) -> StepMap {
        StepMap::empty()
    }

    fn invert(&self, _doc: &Node) -> Box<dyn Step> {
        Box::new(AddMarkStep::new(self.from, self.to, self.mark.clone()))
    }

    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let from = mapping.map_result(self.from, 1);
        let to = mapping.map_result(self.to, -1);
        if (from.deleted() && to.deleted()) || from.pos >= to.pos {
            return None;
        }
        Some(Box::new(RemoveMarkStep::new(
            from.pos,
            to.pos,
            self.mark.clone(),
        )))
    }

    fn merge(&self, other: &dyn Step) -> Option<Box<dyn Step>> {
        let other = other.as_any().downcast_ref::<RemoveMarkStep>()?;
        if other.mark == self.mark && self.from <= other.to && self.to >= other.from {
            Some(Box::new(RemoveMarkStep::new(
                self.from.min(other.from),
                self.to.max(other.to),
                self.mark.clone(),
            )))
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
