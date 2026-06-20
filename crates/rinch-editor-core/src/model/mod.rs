//! The document model: the single authoritative representation of editor content.
//!
//! A document is a persistent (structurally-shared, immutable) tree of [`Node`]s.
//! Inline content carries [`Mark`]s; nodes and marks carry typed [`Attrs`]. The
//! model is a faithful Rust port of ProseMirror's value model — cloning is cheap
//! (`Rc` bumps), every edit produces a new tree sharing unchanged subtrees, and
//! there is exactly one integer position space over the tree (see [`crate::pos`],
//! M1).
//!
//! Built across the editor-rearchitecture milestones:
//! - **M1 (here):** `attrs` (typed attributes) — done; `types`/`node`/`mark`/
//!   `fragment`/`slice` follow in this milestone.

pub mod attrs;
pub mod fragment;
pub mod mark;
pub mod node;
pub mod slice;
pub mod types;

pub use attrs::{AttrValue, Attrs};
pub use fragment::Fragment;
pub use mark::Mark;
pub use node::Node;
pub use slice::Slice;
pub use types::{MarkType, NodeType};
