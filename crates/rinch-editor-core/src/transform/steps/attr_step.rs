//! [`SetNodeAttrStep`] / [`SetDocAttrStep`] — change a single attribute.
//!
//! Setting `heading.level`, `image.alt`, `ordered_list.start`, or a doc-level
//! attribute. These induce no position change (their step map is empty). Unlike
//! ProseMirror's slice-based `AttrStep`, the node-attr step rebuilds the target
//! node in place and splices it back up the ancestor path — the result is
//! identical (an equal-sized node with new attrs) but avoids the empty-content
//! slice dance.
//!
//! `value` is `Option<AttrValue>`: `Some` sets the attribute, `None` removes it.
//! This lets `invert` capture "the attribute was absent" precisely, so
//! `apply∘invert` is an exact identity even for attrs that had no prior value.

use crate::model::{AttrValue, Node};
use crate::pos::{Pos, ResolvedPos};
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::{Mapping, StepMap};
use std::any::Any;

/// Rebuild the document with the node at `r.node(depth)`'s child index
/// `r.index(depth)` replaced by `new_child`, splicing every ancestor up to the
/// root (sharing all untouched subtrees).
fn rebuild_up(r: &ResolvedPos, depth: usize, new_child: Node) -> Node {
    let parent = r.node(depth);
    let idx = r.index(depth);
    let rebuilt = parent.copy_with_content(parent.content().replace_child(idx, new_child));
    if depth == 0 {
        rebuilt
    } else {
        rebuild_up(r, depth - 1, rebuilt)
    }
}

/// Set (or remove) one attribute on the node that starts at `pos`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetNodeAttrStep {
    /// Position before the target node (its open token).
    pub pos: usize,
    /// The attribute name.
    pub attr: String,
    /// The new value, or `None` to remove the attribute.
    pub value: Option<AttrValue>,
}

impl SetNodeAttrStep {
    /// Set the attribute `attr` of the node at `pos` to `value`.
    pub fn new(pos: usize, attr: impl Into<String>, value: AttrValue) -> SetNodeAttrStep {
        SetNodeAttrStep {
            pos,
            attr: attr.into(),
            value: Some(value),
        }
    }
}

impl Step for SetNodeAttrStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let r = doc
            .resolve(Pos(self.pos))
            .map_err(|e| StepError::new(e.to_string()))?;
        let depth = r.depth();
        let index = r.index(depth);
        let target = r
            .parent()
            .content()
            .maybe_child(index)
            .ok_or_else(|| StepError::new("no node at attr step position"))?;
        if target.is_text() {
            return Err(StepError::new("cannot set attributes on a text node"));
        }
        let new_attrs = match &self.value {
            Some(v) => target.attrs().with(self.attr.clone(), v.clone()),
            None => target.attrs().without(&self.attr),
        };
        let updated = target.with_attrs(new_attrs);
        Ok(rebuild_up(&r, depth, updated))
    }

    fn get_map(&self) -> StepMap {
        StepMap::empty()
    }

    fn invert(&self, doc: &Node) -> Box<dyn Step> {
        let prior = doc
            .node_at(self.pos)
            .and_then(|n| n.attrs().get(&self.attr).cloned());
        Box::new(SetNodeAttrStep {
            pos: self.pos,
            attr: self.attr.clone(),
            value: prior,
        })
    }

    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>> {
        let pos = mapping.map_result(self.pos, 1);
        if pos.deleted_after() {
            return None;
        }
        Some(Box::new(SetNodeAttrStep {
            pos: pos.pos,
            attr: self.attr.clone(),
            value: self.value.clone(),
        }))
    }

    fn clone_box(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Set (or remove) one document-level attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetDocAttrStep {
    /// The attribute name.
    pub attr: String,
    /// The new value, or `None` to remove the attribute.
    pub value: Option<AttrValue>,
}

impl SetDocAttrStep {
    /// Set the document attribute `attr` to `value`.
    pub fn new(attr: impl Into<String>, value: AttrValue) -> SetDocAttrStep {
        SetDocAttrStep {
            attr: attr.into(),
            value: Some(value),
        }
    }
}

impl Step for SetDocAttrStep {
    fn apply(&self, doc: &Node) -> Result<Node, StepError> {
        let new_attrs = match &self.value {
            Some(v) => doc.attrs().with(self.attr.clone(), v.clone()),
            None => doc.attrs().without(&self.attr),
        };
        Ok(doc.with_attrs(new_attrs))
    }

    fn get_map(&self) -> StepMap {
        StepMap::empty()
    }

    fn invert(&self, doc: &Node) -> Box<dyn Step> {
        Box::new(SetDocAttrStep {
            attr: self.attr.clone(),
            value: doc.attrs().get(&self.attr).cloned(),
        })
    }

    fn map(&self, _mapping: &Mapping) -> Option<Box<dyn Step>> {
        Some(self.clone_box())
    }

    fn clone_box(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
