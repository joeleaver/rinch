//! View layer for rendering the document.

mod render;
pub mod input_bridge;
pub mod visual_layer;

pub use render::{Renderer, BlockSignals, apply_changes, render_document_reactive};
pub use input_bridge::{create_input_bridge, destroy_input_bridge};
pub use visual_layer::{
    VisualLayerState, GlyphBounds, SelectionRect,
    create_visual_layer, update_cursor_position, update_selection_rects,
    register_block_nodes, cursor_blink_css,
};
