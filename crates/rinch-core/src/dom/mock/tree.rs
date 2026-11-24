//! [`DomTree`](crate::dom::DomTree) for [`MockDomDocument`].

use super::{MockDomDocument, MockNode, MockNodeKind};
use crate::dom::{DomTree, NodeId};

impl DomTree for MockDomDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Element(tag.to_string()),
                text: String::new(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Comment,
                text: text.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // Re-parenting detaches first, exactly as `RinchDocument` and the web
        // backend do. Without this a *move* leaves the node listed twice in its
        // old parent's `children`, so sibling-order assertions in tests read a
        // reordered node as a second mount.
        self.detach(child);
        // Both ends must exist, like the web backend's
        // `if let (Some(p), Some(c)) = …` (see [`MockDomDocument::is_live`]).
        if !self.is_live(child) {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|&c| c != child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = None;
        }
        self.mark_dirty(parent);
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        self.detach(child);
        // Same discarded-child guard as `append_child` — the web backend's
        // `if let (Some(p), Some(c), Some(r)) = …` misses on a discarded `child`
        // too. `NodeHandle::insert_after` routes here when the anchor has a next
        // sibling and to `append_child` when it does not, so without this the
        // mock would answer the *same* operation differently depending on where
        // in the list it lands (issue #184).
        if !self.is_live(child) {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&parent)
            && let Some(pos) = node.children.iter().position(|&c| c == reference)
        {
            node.children.insert(pos, child);
        }
        if let Some(node) = self.nodes.get_mut(&child) {
            node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // Replacing a node with itself is a no-op the browser accepts (the DOM
        // spec re-inserts `node` before its own next sibling), so it must not
        // report the node that is still in the tree as displaced.
        if old == new {
            return;
        }
        let parent = self.nodes.get(&old).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.nodes.get_mut(&parent_id)
                && let Some(pos) = parent_node.children.iter().position(|&c| c == old)
            {
                parent_node.children[pos] = new;
            }
            if let Some(node) = self.nodes.get_mut(&new) {
                node.parent = Some(parent_id);
            }
            self.mark_dirty(parent_id);
            // `old` is **detached**, not retired — it stays in the table and a
            // caller may insert it again, which is what both real backends do
            // (issue #719). A caller that is finished with it says so with
            // `discard_node`.
            if let Some(node) = self.nodes.get_mut(&old) {
                node.parent = None;
            }
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
        // Detach only. The node and its subtree stay in the table and stay
        // re-insertable, per the trait contract (issue #719) — that is the
        // post-condition both real backends owe, so a caller that toggles a
        // captured handle must pass here too.
    }

    /// Detach and **retire**: the node and its subtree leave the table, so every
    /// later operation on those ids is a silent no-op.
    ///
    /// This is the half of the oracle that catches the *other* mistake (issues
    /// #184, #719): the web backend releases a discarded subtree from its maps,
    /// so a caller that discards a node and then re-attaches it would pass every
    /// test in this workspace and break only on the web. Retiring here makes
    /// that a failure on the host.
    fn discard_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
        self.forget_subtree(node);
    }

    fn is_retired(&self, node: NodeId) -> bool {
        !self.nodes.contains_key(&node)
    }

    fn mark_dirty(&mut self, node: NodeId) {
        if self.dirty_set.insert(node) {
            self.dirty.push(node);
        }
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        let marked = std::mem::take(&mut self.dirty);
        // `remove` answers true once per node still dirty: a discarded node
        // is dropped, and one marked again after a discard is reported once.
        marked
            .into_iter()
            .filter(|n| self.dirty_set.remove(n))
            .collect()
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, _selector: &str) -> Option<NodeId> {
        // Mock implementation - just return body
        Some(self.body_id)
    }

    fn query_selector_all(&self, _selector: &str) -> Vec<NodeId> {
        // Mock implementation - return empty vec
        Vec::new()
    }

    fn is_text_node(&self, node: NodeId) -> bool {
        self.is_text_node_calls
            .set(self.is_text_node_calls.get() + 1);
        matches!(
            self.nodes.get(&node).map(|n| &n.kind),
            Some(MockNodeKind::Text)
        )
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.get_children_calls
            .set(self.get_children_calls.get() + 1);
        self.count(|ops| ops.children_reads += 1);
        self.nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        // Discarded children are not listed — see [`MockDomDocument::is_live`].
        if !self.is_live(child) {
            return;
        }
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            let len = parent_node.children.len();
            let idx = index.min(len);
            parent_node.children.insert(idx, child);
        }
        if let Some(child_node) = self.nodes.get_mut(&child) {
            child_node.parent = Some(parent);
        }
        self.mark_dirty(parent);
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node)?.parent
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let parent_id = self.nodes.get(&node)?.parent?;
        let parent = self.nodes.get(&parent_id)?;
        let pos = parent.children.iter().position(|&c| c == node)?;
        parent.children.get(pos + 1).copied()
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        // Simple mock implementation - just create a text node with the content
        // Real implementations would parse the HTML properly
        let id = self.next_id();
        self.nodes.insert(
            id,
            MockNode {
                kind: MockNodeKind::Text,
                text: html.to_string(),
                attributes: std::collections::HashMap::new(),
                live_value: None,
                children: Vec::new(),
                parent: None,
            },
        );
        Some(id)
    }

    /// Retires every existing child and parses **nothing**: the mock has no
    /// HTML parser, so the element is left with no children at all. The first
    /// half is what both real backends do to the children there were (issue
    /// #184: `rinch-dom` frees them, `rinch-web` forgets them), and it is the
    /// half a test of what the replaced children leave behind needs (#1440).
    fn set_inner_html(&mut self, node: NodeId, _html: &str) {
        let children = match self.nodes.get_mut(&node) {
            Some(n) => {
                n.text.clear();
                std::mem::take(&mut n.children)
            }
            None => return,
        };
        for child in children {
            self.forget_subtree(child);
        }
        self.mark_dirty(node);
    }

    fn tag_name(&self, node: NodeId) -> Option<String> {
        match &self.nodes.get(&node)?.kind {
            MockNodeKind::Element(tag) => Some(tag.clone()),
            _ => None,
        }
    }

    fn node_type(&self, node: NodeId) -> Option<u16> {
        match &self.nodes.get(&node)?.kind {
            MockNodeKind::Element(_) => Some(1),
            MockNodeKind::Text => Some(3),
            MockNodeKind::Comment => Some(8),
        }
    }
}
