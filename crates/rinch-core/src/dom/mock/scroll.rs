//! [`DomScroll`](crate::dom::DomScroll) for [`MockDomDocument`].

use super::MockDomDocument;
use crate::dom::{DomScroll, NodeId};

impl DomScroll for MockDomDocument {
    fn set_scroll_top(&mut self, _node: NodeId, _scroll_top: f64) {
        // Mock implementation - does nothing
    }

    fn request_scroll_into_view(&mut self, node: NodeId) {
        self.scroll_into_view_requests.push(node);
    }

    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.scroll_into_view_requests)
    }

    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        self.scroll_to_fraction_requests
            .push((node, fraction, margin));
    }

    fn drain_scroll_to_fraction_requests(&mut self) -> Vec<(NodeId, f32, f32)> {
        std::mem::take(&mut self.scroll_to_fraction_requests)
    }
}
