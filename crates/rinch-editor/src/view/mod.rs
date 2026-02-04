//! View layer for rendering the document.

pub mod input_bridge;
mod render;
pub mod visual_layer;

pub use input_bridge::{create_input_bridge, destroy_input_bridge};
pub use render::{BlockSignals, Renderer, apply_changes, render_document_reactive};
pub use visual_layer::{
    GlyphBounds, SelectionRect, VisualLayerState, create_visual_layer, cursor_blink_css,
    register_block_nodes, update_cursor_position, update_selection_rects,
};
