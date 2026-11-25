//! [`DomLayout`](crate::dom::DomLayout) for [`MockDomDocument`].

use super::MockDomDocument;
use crate::dom::{DomLayout, GlyphBounds, NodeId};

impl DomLayout for MockDomDocument {
    fn watch_bounds(&mut self, node: NodeId) {
        self.watched_bounds.push(node);
    }

    fn query_caret_position(&self, _node_id: u64, _byte_offset: usize) -> Option<(f32, f32)> {
        None // Mock returns None
    }

    fn query_glyph_bounds(&self, _node_id: u64, _byte_offset: usize) -> Option<GlyphBounds> {
        None // Mock returns None
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // Mock does nothing
    }

    fn query_node_layout(&self, node_id: u64) -> Option<(f32, f32, f32, f32)> {
        // `None` unless a test injected a box with `__set_node_layout` — the mock
        // does not lay anything out.
        self.layout.get(&NodeId(node_id)).copied()
    }
}
