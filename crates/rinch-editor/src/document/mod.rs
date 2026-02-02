//! Document model backed by Automerge CRDT.

mod model;
mod position;
mod fragment;

pub use model::{EditorDocument, InlineRun, MarkData};
pub use position::{Position, Range, ResolvedPosition};
pub use fragment::{Fragment, FragmentBlock, FragmentInline};
