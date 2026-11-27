//! [`Transform`] — the builder that turns editing gestures into [`Step`]s.
//!
//! A `Transform` starts from a document and accumulates steps; each gesture
//! helper (`insert_text`, `delete`, `split`, `set_block_type`, `add_mark`, …)
//! appends one or more steps, keeping a [`Mapping`] of every position shift and a
//! per-step snapshot of the document for inversion. This is ProseMirror's
//! `Transform`; the state-layer `Transaction` (selection, stored marks, plugin
//! meta) extends it in M4.
//!
//! Helpers return `Result<&mut Self, StepError>` so failures (schema-invalid
//! steps) surface and chain with `?` — `tr.insert_text(..)?.delete(..)?`.

use std::collections::HashMap;
use std::rc::Rc;

use crate::Slice;
use crate::model::{AttrValue, Attrs, Fragment, Mark, Node, NodeType};
use crate::pos::Pos;
use crate::schema::data_attrs::carry_app_data;
use crate::schema::{DataAttrSplit, Schema};
use crate::transform::data_attrs::{Edit, Spot};
use crate::transform::fit;
use crate::transform::node_range::NodeRange;
use crate::transform::step::{Step, StepError};
use crate::transform::step_map::Mapping;
use crate::transform::steps::{
    AddMarkStep, BatchEdit, BatchStep, RemoveMarkStep, ReplaceAroundStep, ReplaceStep,
    SetDocAttrStep, SetNodeAttrStep,
};

/// An abstraction over a document that accumulates [`Step`]s.
pub struct Transform<'a> {
    schema: &'a Schema,
    /// The current document (after all steps applied so far).
    pub doc: Node,
    steps: Vec<Box<dyn Step>>,
    docs: Vec<Node>,
    mapping: Mapping,
    /// What a split does with the app data attributes named here; any other
    /// is copied. A state's transaction brings its plugins' choices.
    pub(crate) data_attr_splits: Option<Rc<HashMap<String, DataAttrSplit>>>,
}

impl<'a> Transform<'a> {
    /// Start a transform from `doc` (which must belong to `schema`).
    pub fn new(schema: &'a Schema, doc: Node) -> Transform<'a> {
        Transform {
            schema,
            doc,
            steps: Vec::new(),
            docs: Vec::new(),
            mapping: Mapping::new(),
            data_attr_splits: None,
        }
    }

    /// Reconstitute a transform from previously-extracted parts. Paired with
    /// [`Transform::into_parts`] so the state-layer
    /// [`Transaction`](crate::state::Transaction) — which owns an `Rc<Schema>`
    /// rather than a borrow — can borrow a local `&Schema` for the duration of one
    /// gesture, run it through the shared gesture logic here, then take the
    /// accumulation back. This avoids duplicating every gesture on `Transaction`.
    pub(crate) fn from_parts(
        schema: &'a Schema,
        doc: Node,
        steps: Vec<Box<dyn Step>>,
        docs: Vec<Node>,
        mapping: Mapping,
    ) -> Transform<'a> {
        Transform {
            schema,
            doc,
            steps,
            docs,
            mapping,
            data_attr_splits: None,
        }
    }

    /// What a split does with the app data attribute `name`
    /// ([`Plugin::data_attr_splits`](crate::plugin::Plugin::data_attr_splits)).
    pub(crate) fn data_attr_split(&self, name: &str) -> DataAttrSplit {
        self.data_attr_splits
            .as_ref()
            .and_then(|splits| splits.get(name).copied())
            .unwrap_or_default()
    }

    /// Extract the accumulated state (the inverse of [`Transform::from_parts`]).
    pub(crate) fn into_parts(self) -> (Node, Vec<Box<dyn Step>>, Vec<Node>, Mapping) {
        (self.doc, self.steps, self.docs, self.mapping)
    }

    /// The document before any step was applied.
    pub fn before(&self) -> &Node {
        self.docs.first().unwrap_or(&self.doc)
    }

    /// The accumulated steps.
    pub fn steps(&self) -> &[Box<dyn Step>] {
        &self.steps
    }

    /// The per-step document snapshots: `docs()[i]` is the document *before* step
    /// `i` was applied. Used to invert a whole transform (undo) and by collab.
    pub fn docs(&self) -> &[Node] {
        &self.docs
    }

    /// The accumulated position mapping.
    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    /// True if any step changed the document.
    pub fn doc_changed(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Apply `step`, recording it (and its map and the pre-step doc). Returns the
    /// step error if it fails to apply (e.g. the result would be schema-invalid),
    /// leaving the transform unchanged.
    pub fn step(&mut self, step: Box<dyn Step>) -> Result<&mut Self, StepError> {
        let new_doc = step.apply(&self.doc)?;
        self.docs.push(self.doc.clone());
        self.mapping.append_map(step.get_map());
        self.steps.push(step);
        self.doc = new_doc;
        Ok(self)
    }
}

mod marks;
mod replace;
mod structure;

pub use structure::can_split;
