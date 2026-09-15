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
    /// [`DomDocument::discard_node`] means by *retiring* a node.
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
        // A retired id must not come back out of `take_dirty_nodes`: a consumer
        // that resolves the ids it is handed would find nothing there.
        self.dirty.retain(|&d| d != node);
    }

    /// Whether `child` still names a node — the guard every structural mutation
    /// applies before listing it under a parent.
    ///
    /// Re-attaching a **discarded** child (one a previous `discard_node`
    /// dropped) is a silent no-op on the web backend, whose
    /// `self.nodes.get(&child.0)` simply misses. It has to be one here too, or
    /// the parent lists an id that resolves to nothing and the caller's bug
    /// hides behind a plausible child count (issues #184, #719).
    ///
    /// A merely **removed** child is still live and still re-insertable — that
    /// is the other half of the same oracle (issue #719).
    fn is_live(&self, child: NodeId) -> bool {
        self.nodes.contains_key(&child)
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

    /// Merge one declaration into the node's inline `style`, the way both real
    /// backends do (#666).
    ///
    /// This used to append `"{property}: {value};"` to whatever string was
    /// there, with no separator and no replacement: a node whose `style` said
    /// `"color: red"` came out of `set_style("padding", "12px")` as
    /// `"color: redpadding: 12px;"` — one declaration whose value is
    /// `redpadding: 12px` — and setting the same property twice appended
    /// twice. That made the mock unusable as an oracle for anything about
    /// inline-style *composition*, which is the one thing a `style:` prop and
    /// a style shorthand on the same element are about.
    ///
    /// The parse and the join are
    /// [`split_declarations`](crate::dom::split_declarations)/[`serialize_declarations`](crate::dom::serialize_declarations),
    /// which is also what `RinchDocument::set_styles` uses (#670) — so the mock
    /// now agrees with desktop declaration for declaration, quoted and
    /// bracketed values included. A property already declared is replaced
    /// **where it stands**, matching CSSOM's `setProperty` and desktop's
    /// `dom_tests::set_style_replaces_a_declaration_in_place`, and `property`
    /// is matched ASCII case-insensitively unless it is a custom property
    /// ([`normalize_property_name`](crate::dom::normalize_property_name), #711).
    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        if let Some(n) = self.nodes.get_mut(&node) {
            let style = n.attributes.entry("style".to_string()).or_default();
            let mut decls = super::split_declarations(style);
            let property = super::normalize_property_name(property);
            match decls.iter_mut().find(|(k, _)| k.as_str() == &*property) {
                Some(slot) => slot.1 = value.to_string(),
                None => decls.push((property.into_owned(), value.to_string())),
            }
            *style = super::serialize_declarations(&decls);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every structural mutation refuses a **discarded** child, not just
    /// `append_child` (issue #184).
    ///
    /// `NodeHandle::insert_after` routes to `insert_before` when the anchor has a
    /// next sibling and to `append_child` when it does not, so a mock that guarded
    /// only one of them would answer the *same* operation differently depending on
    /// where in the list it landed — and would list an id that resolves to nothing,
    /// hiding the caller's bug behind a plausible child count.
    #[test]
    fn a_discarded_child_is_refused_by_every_insertion_path() {
        type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
        let paths: [Relist; 3] = [
            |doc, parent, child, _anchor| doc.append_child(parent, child),
            |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
            |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
        ];
        for relist in paths {
            let mut doc = MockDomDocument::new();
            let body = doc.body();
            let anchor = doc.create_element("div");
            doc.append_child(body, anchor);
            let child = doc.create_element("div");
            doc.append_child(body, child);

            doc.discard_node(child);
            relist(&mut doc, body, child, anchor);

            assert!(
                !doc.get_children(body).contains(&child),
                "#184: a discarded child must not be relisted under its parent"
            );
            for id in doc.get_children(body) {
                assert!(
                    doc.tag_name(id).is_some(),
                    "#184: every listed child must still resolve to a node"
                );
            }
        }
    }

    /// The other half: every one of those paths **accepts** a merely *removed*
    /// child, because `remove_node` is a detach and the subtree must come back
    /// (issue #719).
    ///
    /// The pair is what makes this mock an oracle for both mistakes rather than
    /// one. Sitting on `remove_node` alone — the shape before #719 — a mock that
    /// retires everything and a mock that retires nothing are both "consistent";
    /// only testing the two verbs against each other tells them apart.
    #[test]
    fn a_removed_child_is_accepted_by_every_insertion_path() {
        type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
        let paths: [Relist; 3] = [
            |doc, parent, child, _anchor| doc.append_child(parent, child),
            |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
            |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
        ];
        for relist in paths {
            let mut doc = MockDomDocument::new();
            let body = doc.body();
            let anchor = doc.create_element("div");
            doc.append_child(body, anchor);
            let child = doc.create_element("div");
            let grandchild = doc.create_element("span");
            doc.append_child(child, grandchild);
            doc.append_child(body, child);

            doc.remove_node(child);
            assert!(
                !doc.get_children(body).contains(&child),
                "precondition: remove_node unlinks the child"
            );

            relist(&mut doc, body, child, anchor);

            assert!(
                doc.get_children(body).contains(&child),
                "#719: a removed child must be re-insertable"
            );
            assert_eq!(
                doc.get_children(child),
                vec![grandchild],
                "#719: its subtree must come back with it"
            );
        }
    }

    /// `replace_node` **detaches** the node it displaced, like both real backends
    /// (issue #719): it keeps its identity and its subtree, and can be inserted
    /// again.
    #[test]
    fn replacing_a_node_detaches_it_and_keeps_its_subtree() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let old = doc.create_element("div");
        doc.append_child(body, old);
        let grandchild = doc.create_element("span");
        doc.append_child(old, grandchild);
        let new = doc.create_element("p");

        doc.replace_node(old, new);

        assert_eq!(doc.get_children(body), vec![new]);
        assert!(doc.tag_name(old).is_some(), "#719: `old` must stay live");
        assert_eq!(
            doc.parent_node(old),
            None,
            "#719: but it must be unlinked from its parent"
        );
        assert_eq!(
            doc.get_children(old),
            vec![grandchild],
            "#719: `old` keeps its own subtree"
        );

        doc.append_child(body, old);
        assert_eq!(
            doc.get_children(body),
            vec![new, old],
            "#719: a displaced node can be inserted again"
        );
    }

    /// `discard_node` is the route that retires: the subtree leaves the table
    /// (issue #184).
    #[test]
    fn discarding_a_node_retires_it_and_its_subtree() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let old = doc.create_element("div");
        doc.append_child(body, old);
        let grandchild = doc.create_element("span");
        doc.append_child(old, grandchild);

        doc.discard_node(old);

        assert!(doc.get_children(body).is_empty());
        assert!(doc.tag_name(old).is_none(), "#184: `old` must be retired");
        assert!(
            doc.tag_name(grandchild).is_none(),
            "#184: `old`'s subtree must be retired with it"
        );
    }

    /// Replacing a node with itself is a no-op the browser accepts, so it must not
    /// unlink a node that is still in the tree (issue #184).
    #[test]
    fn replacing_a_node_with_itself_does_not_unlink_it() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let node = doc.create_element("div");
        doc.append_child(body, node);

        doc.replace_node(node, node);

        assert_eq!(doc.get_children(body), vec![node]);
        assert_eq!(
            doc.parent_node(node),
            Some(body),
            "#184: a self-replace must leave the node in its parent"
        );
    }

    /// A retired id must not resurface from `take_dirty_nodes` — a consumer that
    /// resolves what it is handed would find nothing there (issue #184).
    #[test]
    fn a_discarded_node_leaves_the_dirty_list() {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let node = doc.create_element("div");
        doc.append_child(body, node);
        doc.set_attribute(node, "class", "x");

        doc.discard_node(node);

        assert!(
            !doc.take_dirty_nodes().contains(&node),
            "#184: a discarded node must not be reported dirty"
        );
    }

    // === set_style composes an inline style (#666) ===
    //
    // The mock is the oracle two other layers are tested against — `StyleProp`
    // in this crate, and every component test that renders onto a
    // `MockDomDocument` — so its `style` attribute has to be the string a real
    // backend would leave. It used to be `push_str("{prop}: {value};")` with no
    // separator and no replacement, which is neither parseable CSS nor
    // anything `RinchDocument` or a browser would produce.

    fn style_of(doc: &MockDomDocument, node: NodeId) -> String {
        doc.get_attribute(node, "style")
            .expect("a style was written")
    }

    /// A `set_style` onto an attribute somebody else wrote joins it with the
    /// separator that makes it a second declaration.
    ///
    /// Appending with none gave `"color: redpadding: 12px;"` — one declaration
    /// whose value is `redpadding: 12px`.
    #[test]
    fn set_style_separates_itself_from_an_existing_declaration() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "color: red");
        doc.set_style(div, "padding", "12px");
        assert_eq!(style_of(&doc, div), "color: red; padding: 12px");
    }

    /// Setting a property twice replaces it **where it stands**, which is what
    /// CSSOM's `setProperty` does and what
    /// `rinch-dom`'s `dom_tests::set_style_replaces_a_declaration_in_place`
    /// pins for desktop. Appending instead left both declarations in the block.
    ///
    /// The `gap` between them is load-bearing: with only the two `color`
    /// declarations, replace-in-place and remove-then-append give the same
    /// string, so the fixture would sit on a fixed point and pass either way.
    #[test]
    fn set_style_replaces_a_declaration_in_place() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_style(div, "color", "red");
        doc.set_style(div, "gap", "4px");
        doc.set_style(div, "color", "blue");
        assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
    }

    /// `property` is matched ASCII case-insensitively, so the mock composes an
    /// inline style the way desktop and a browser do (#711). The `gap` is here
    /// for the same reason as above — without it, folding and appending give
    /// the same string.
    ///
    /// Kills the mutant that drops `normalize_property_name` from
    /// `MockDomDocument::set_style`.
    #[test]
    fn set_style_matches_a_property_name_case_insensitively() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "color: red; gap: 4px");
        doc.set_style(div, "COLOR", "blue");
        assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
    }

    /// …but a **custom** property is compared exactly, so these are two.
    /// Chrome 150: `setProperty("--Foo", "3px")` on a block holding
    /// `--foo: 2px` gives `cssText === "--foo: 2px; --Foo: 3px;"`.
    ///
    /// Kills the mutant that lowercases unconditionally.
    #[test]
    fn set_style_keeps_two_custom_properties_that_differ_only_in_case() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", "--foo: 2px");
        doc.set_style(div, "--Foo", "3px");
        assert_eq!(style_of(&doc, div), "--foo: 2px; --Foo: 3px");
    }

    /// And it goes through the workspace's one inline-style parser (#670), so a
    /// `;` inside a `url()` already in the attribute is part of that value
    /// here too — the mock diverging from desktop on this would make it an
    /// oracle for the wrong thing.
    #[test]
    fn set_style_does_not_split_a_url_already_in_the_attribute() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "background-image: url(data:image/png;base64,AAA=)",
        );
        doc.set_style(div, "padding", "12px");
        assert_eq!(
            style_of(&doc, div),
            "background-image: url(data:image/png;base64,AAA=); padding: 12px"
        );
    }

    /// The first write onto a node with no style attribute is still just the
    /// one declaration — no leading separator, no trailing `;`.
    #[test]
    fn the_first_set_style_writes_one_bare_declaration() {
        let mut doc = MockDomDocument::new();
        let div = doc.create_element("div");
        doc.set_style(div, "color", "red");
        assert_eq!(style_of(&doc, div), "color: red");
    }
}
