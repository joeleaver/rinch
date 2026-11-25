//! [`DomFocus`](crate::dom::DomFocus) for [`MockDomDocument`].

use super::{MockDomDocument, MockNodeKind};
use crate::dom::{DomContent, DomFocus, NodeId, SelectionDirection};

impl DomFocus for MockDomDocument {
    fn focus_element(&mut self, node_id: NodeId) {
        self.focused = Some(node_id);
    }

    fn active_element(&self) -> Option<NodeId> {
        self.focused
    }

    /// Records `(node_id, start, end, direction)` as-is — swapped so `start
    /// <= end`, the way a browser's own `setSelectionRange` would (issue
    /// #552). The mock does not model "stash until this node is focused"
    /// separately from "applied now": there is nothing else that could be
    /// holding a selection, so recording unconditionally gives both for free.
    fn set_selection_range(
        &mut self,
        node_id: NodeId,
        start: usize,
        end: usize,
        direction: SelectionDirection,
    ) {
        let (start, end) = (start.min(end), start.max(end));
        self.selection_range = Some((node_id, start, end, direction));
    }

    /// The last selection set on `node_id`, clamped to its live value; a
    /// caret at the start for any other `<input>`/`<textarea>` (issue #1678).
    fn text_selection_range(&self, node_id: NodeId) -> Option<(usize, usize, SelectionDirection)> {
        let n = self.nodes.get(&node_id)?;
        if !matches!(&n.kind, MockNodeKind::Element(tag) if tag == "input" || tag == "textarea") {
            return None;
        }
        let len = self
            .live_value(node_id)
            .map(|v| v.encode_utf16().count())
            .unwrap_or(0);
        match self.selection_range {
            Some((n, start, end, dir)) if n == node_id => Some((start.min(len), end.min(len), dir)),
            _ => Some((0, 0, SelectionDirection::Forward)),
        }
    }
}
