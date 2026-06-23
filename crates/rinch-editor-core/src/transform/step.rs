//! The [`Step`] trait — the unit of document change.
//!
//! Every edit in the system is a sequence of `Step`s. A step is **invertible**
//! (so undo is "apply the inverse"), **mappable** (so a step can be rebased over
//! other steps for collaboration and redo), and carries a [`StepMap`] describing
//! how it shifts positions. This is the structural property that lets one engine
//! serve typing, undo/redo, and collaborative editing without special cases.
//!
//! Faithful port of ProseMirror's `transform/src/step.ts`. The one Rust-specific
//! addition is [`Step::clone_box`] (amendment A12): trait objects cannot derive
//! `Clone`, yet `Transaction`'s step list, history's inverted steps, and collab
//! rebase all need to clone boxed steps.

use crate::model::Node;
use crate::transform::step_map::{Mapping, StepMap};
use std::any::Any;
use thiserror::Error;

/// Why a [`Step::apply`] failed: the position was invalid, or the result would
/// violate the schema. A failed step rejects its entire transaction — invalid
/// content is never written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct StepError(pub String);

impl StepError {
    /// Construct a step error with a diagnostic message.
    pub fn new(msg: impl Into<String>) -> StepError {
        StepError(msg.into())
    }
}

/// A single, invertible, mappable change to a document.
pub trait Step: std::fmt::Debug {
    /// Apply this step to `doc`, returning the new document or a failure. **Schema
    /// enforcement lives here**: a step whose result would violate a parent's
    /// content expression returns `Err`, leaving the original document untouched.
    fn apply(&self, doc: &Node) -> Result<Node, StepError>;

    /// The position map this step induces (for mapping later steps and selections).
    fn get_map(&self) -> StepMap;

    /// The inverse of this step against the document it was applied to (for undo).
    /// `doc` must be the document this step was applied to.
    fn invert(&self, doc: &Node) -> Box<dyn Step>;

    /// Rebase this step over a mapping (collab/redo). Returns `None` if the step no
    /// longer has anywhere to apply (its range was entirely deleted).
    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>>;

    /// Try to fold a following step into this one (typing coalescing). Returns the
    /// merged step, or `None` if they cannot be combined.
    fn merge(&self, _other: &dyn Step) -> Option<Box<dyn Step>> {
        None
    }

    /// Clone into a fresh box (A12 — trait objects can't derive `Clone`).
    fn clone_box(&self) -> Box<dyn Step>;

    /// Downcast support, so `merge` can recognise a same-typed neighbour.
    fn as_any(&self) -> &dyn Any;
}

impl Clone for Box<dyn Step> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
