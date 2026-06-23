//! The transform engine: invertible, mappable [`Step`](step::Step)s.
//!
//! Every change to a document is expressed as one or more `Step`s. A small,
//! deliberately minimal step set — `ReplaceStep`, `ReplaceAroundStep`,
//! `AddMarkStep`/`RemoveMarkStep`, `SetNodeAttrStep`/`SetDocAttrStep` — expresses
//! every editing gesture; nothing is added per gesture. Steps are **invertible**
//! (undo), **mappable** (redo, collab rebase), and **schema-enforcing** (an
//! invalid step is rejected by [`apply`](step::Step::apply), never silently
//! written). This is a faithful port of ProseMirror's `prosemirror-transform`.

pub mod build;
pub mod node_range;
pub mod replace;
pub mod step;
pub mod step_map;
pub mod steps;

pub use build::Transform;
pub use node_range::{NodeRange, block_range, block_range_simple};
pub use step::{Step, StepError};
pub use step_map::{MapResult, Mapping, StepMap};
pub use steps::{
    AddMarkStep, RemoveMarkStep, ReplaceAroundStep, ReplaceStep, SetDocAttrStep, SetNodeAttrStep,
};
