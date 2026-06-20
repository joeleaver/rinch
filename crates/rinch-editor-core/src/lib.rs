//! # rinch-editor-core
//!
//! The pure, renderer-agnostic core of the Rinch rich-text editor.
//!
//! This crate is the single authoritative home of the editor's *document model*,
//! *transform/step engine*, *schema*, *state*, *commands*, and *history*. It has
//! **zero** dependency on `rinch-dom`, `winit`, `web_sys`, `parley`, `taffy`,
//! `vello`, or `automerge`, and compiles cleanly to `wasm32`. Rendering and input
//! live behind a `View` seam implemented by platform crates (desktop: `rinch`;
//! web: `rinch-web`); collaboration is an optional, feature-gated adapter in
//! `rinch-editor-collab`.
//!
//! See `docs/design/editor-rearchitecture.md` for the full design and the
//! milestone plan. This is **M0**: the crate scaffold plus the schema, lifted
//! from the old `rinch-editor` crate (dyn `Node`/`Mark` traits and the
//! Automerge error variant dropped). The concrete `Node`/`Mark`/`Fragment`/
//! `Slice` value model, `Pos`, the `Step`/`Transaction` engine, and typed
//! attrs (`AttrValue`) arrive in M1+.

pub mod error;
pub mod model;
pub mod pos;
pub mod schema;

pub use error::EditorError;
pub use model::{AttrValue, Attrs, Fragment, Mark, MarkType, Node, NodeType, Slice};
pub use pos::{Pos, ResolvedPos};
pub use schema::{
    AttrSpec, MarkSet, MarkSpec, MarkSpecBuilder, NodeSpec, NodeSpecBuilder, Schema, SchemaBuilder,
};
