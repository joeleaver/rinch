//! View layer for rendering the document.

mod render;
pub mod input_bridge;

pub use render::{Renderer, BlockSignals, apply_changes, render_document_reactive};
pub use input_bridge::{create_input_bridge, destroy_input_bridge};
