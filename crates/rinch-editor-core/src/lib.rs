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
//! milestone plan. Implemented through **M4**: the schema (M0), the value model +
//! positions + `ContentMatch` (M1), the transform/`Step` engine (M2), total
//! schema-driven serialization — the durable `DocNode` JSON wire shape, HTML, and
//! markdown (M3) — and the state layer: [`EditorState`]/[`Transaction`],
//! [`Selection`], [`Plugin`]s, the [`Command`] catalogue, the [`keymap`], input
//! rules, and the Step-based undo/redo [`plugins::HistoryPlugin`] (M4). The
//! platform view seams (M5+) follow.

use std::rc::Rc;

#[cfg(feature = "a11y")]
pub mod a11y;
pub mod command;
pub mod commands;
pub mod decoration;
pub mod error;
pub mod input_rules;
pub mod keymap;
pub mod model;
pub mod motion;
pub mod plugin;
pub mod plugins;
pub mod pos;
pub mod schema;
pub mod selection;
pub mod serialize;
pub mod state;
pub mod tables;
pub mod transform;
pub mod view;

pub use command::{Command, Dispatch, chain};
pub use commands::BaseCommandsPlugin;
pub use decoration::{Decoration, DecorationSet, Widget};
pub use error::EditorError;
pub use input_rules::{InputRule, apply_input_rules};
pub use keymap::{Key, KeyBinding, Keymap, Modifiers};
pub use model::{AttrValue, Attrs, Fragment, Mark, MarkType, Node, NodeType, Slice};
pub use motion::{CursorMotion, block_range_at, resolve_cursor_motion, word_range_at};
pub use plugin::{Plugin, PluginKey};
pub use plugins::{HistoryPlugin, MarkdownInputRulesPlugin, PlaceholderPlugin};
pub use pos::{Pos, ResolvedPos};
pub use schema::{
    AttrSpec, MarkSet, MarkSpec, MarkSpecBuilder, NodeSpec, NodeSpecBuilder, Schema, SchemaBuilder,
};
pub use selection::{CellSelection, NodeSelection, Selection, TextSelection};
pub use state::{EditorState, Transaction};
pub use tables::{Axis, Rect, TableMap};
pub use transform::{
    AddMarkStep, MapResult, Mapping, NodeRange, RemoveMarkStep, ReplaceAroundStep, ReplaceStep,
    SetDocAttrStep, SetNodeAttrStep, Step, StepError, StepMap, Transform, block_range,
    block_range_simple,
};
pub use view::{EditorView, ViewRequest};

/// The default plugin set for a standard rich-text editor: the built-in command
/// catalogue + keymap ([`BaseCommandsPlugin`]), markdown input rules
/// ([`MarkdownInputRulesPlugin`]), and Step-based undo/redo ([`HistoryPlugin`]).
/// Pass to [`EditorState::create`].
pub fn default_plugins() -> Vec<Rc<dyn Plugin>> {
    vec![
        Rc::new(BaseCommandsPlugin),
        Rc::new(MarkdownInputRulesPlugin),
        Rc::new(HistoryPlugin::new()),
    ]
}
