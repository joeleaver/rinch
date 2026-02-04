//! Document model backed by Automerge CRDT.

mod fragment;
mod model;
mod position;

pub use fragment::{Fragment, FragmentBlock, FragmentInline};
pub use model::{EditorDocument, InlineRun, MarkData};
pub use position::{Position, Range, ResolvedPosition};
