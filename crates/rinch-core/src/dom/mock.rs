//! Mock DOM document for testing.

use super::NodeId;
use super::traits::{DomDocument, GlyphBounds};

/// A mock DOM document for testing.
pub struct MockDomDocument {
    doc_key: u64,
    next_id: usize,
    nodes: std::collections::HashMap<NodeId, MockNode>,
    dirty: Vec<NodeId>,
    root_id: NodeId,
    body_id: NodeId,
}

struct MockNode {
    kind: MockNodeKind,
    text: String,
    attributes: std::collections::HashMap<String, String>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

enum MockNodeKind {
    Element(String),
    Text,
    Comment,
}

impl Default for MockDomDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl MockDomDocument {
    pub fn new() -> Self {
        let mut doc = Self {
            doc_key: crate::dom::next_doc_key(),
            next_id: 0,
            nodes: std::collections::HashMap::new(),
            dirty: Vec::new(),
            root_id: NodeId(0),
            body_id: NodeId(0),
        };

        // Create root and body
        doc.root_id = doc.create_element("html");
        doc.body_id = doc.create_element("body");
        doc.append_child(doc.root_id, doc.body_id);

        doc
    }

    fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Unlink `child` from whatever parent currently lists it.
    ///
    /// `append_child`/`insert_before` call this first so a *move* does not leave
    /// the node listed twice, matching `RinchDocument` and the web backend.
    fn detach(&mut self, child: NodeId) {
        let old_parent = self.nodes.get(&child).and_then(|n| n.parent);
        if let Some(old_parent) = old_parent {
            if let Some(node) = self.nodes.get_mut(&old_parent) {
                node.children.retain(|&c| c != child);
            }
            self.mark_dirty(old_parent);
        }
    }

    /// Drop `node` and its whole subtree from the node table — what
    /// [`DomDocument::remove_node`] means by *retiring* a node.
    ///
    /// Ids are never recycled here (`next_id` only counts up), so a retired id
    /// can never name a different node later; a stale handle just resolves to
    /// nothing, and every accessor on this mock is `get`-guarded.
    fn forget_subtree(&mut self, node: NodeId) {
        let children = self
            .nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for child in children {
            self.forget_subtree(child);
        }
        self.nodes.remove(&node);
    }
}

impl DomDocument for MockDomDocument {
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
        // `if let (Some(p), Some(c)) = …`. Appending a **retired** child (one a
        // previous `remove_node` dropped) is a silent no-op there, so it has to
        // be one here too — otherwise the parent lists an id that resolves to
        // nothing and the caller's bug hides behind a plausible child count
        // (issue #184).
        if !self.nodes.contains_key(&child) {
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
        }
    }

    fn remove_node(&mut self, node: NodeId) {
        let parent = self.nodes.get(&node).and_then(|n| n.parent);
        if let Some(parent_id) = parent {
            self.remove_child(parent_id, node);
        }
        // Retire the node and its subtree, per the trait contract: a removed
        // handle must not be re-attached. The browser backend has to enforce
        // this to release the DOM node it was pinning (issue #184), so the mock
        // enforces it too — otherwise a caller that re-appends a removed node
        // passes every test here and breaks only on the web.
        self.forget_subtree(node);
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.text = text.to_string();
        }
        self.mark_dirty(node);
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.attributes.insert(name.to_string(), value.to_string());
        }
        self.mark_dirty(node);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.attributes.remove(name);
        }
        self.mark_dirty(node);
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.nodes.get(&node)?.attributes.get(name).cloned()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            // Simple style handling - just append to style attribute
            let style = n.attributes.entry("style".to_string()).or_default();
            style.push_str(&format!("{}: {};", property, value));
        }
        self.mark_dirty(node);
    }

    fn mark_dirty(&mut self, node: NodeId) {
        if !self.dirty.contains(&node) {
            self.dirty.push(node);
        }
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.dirty)
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

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.nodes
            .get(&node)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
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
                children: Vec::new(),
                parent: None,
            },
        );
        Some(id)
    }

    fn set_scroll_top(&mut self, _node: NodeId, _scroll_top: f64) {
        // Mock implementation - does nothing
    }

    fn set_inner_html(&mut self, _node: NodeId, _html: &str) {
        // Mock implementation - no-op for tests
    }

    fn query_caret_position(&self, _node_id: u64, _byte_offset: usize) -> Option<(f32, f32)> {
        None // Mock returns None
    }

    fn query_glyph_bounds(&self, _node_id: u64, _byte_offset: usize) -> Option<GlyphBounds> {
        None // Mock returns None
    }

    fn focus_element(&mut self, _node_id: NodeId) {
        // Mock does nothing
    }

    fn resolve_layout(&mut self, _width: f32, _height: f32) {
        // Mock does nothing
    }

    fn query_node_layout(&self, _node_id: u64) -> Option<(f32, f32, f32, f32)> {
        None // Mock returns None
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

    fn text_content(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node)?;
        match n.kind {
            MockNodeKind::Text | MockNodeKind::Comment => Some(n.text.clone()),
            MockNodeKind::Element(_) => {
                // Concatenate descendant text (depth-first), matching the real DOM.
                let mut out = String::new();
                for &child in &n.children {
                    if let Some(t) = self.text_content(child) {
                        out.push_str(&t);
                    }
                }
                Some(out)
            }
        }
    }
}
