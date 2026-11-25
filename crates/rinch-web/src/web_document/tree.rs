//! [`DomTree`] for [`WebDocument`]: the node tree.

use super::*;

use rinch_core::dom::{DomTree, NodeId};
use wasm_bindgen::JsCast;

impl DomTree for WebDocument {
    fn doc_key(&self) -> u64 {
        self.doc_key
    }

    fn create_element(&mut self, tag: &str) -> NodeId {
        let el = if is_svg_tag(tag) {
            self.browser_doc
                .create_element_ns(Some("http://www.w3.org/2000/svg"), tag)
                .unwrap()
        } else {
            self.browser_doc.create_element(tag).unwrap()
        };
        let node: web_sys::Node = el.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        let text_node = self.browser_doc.create_text_node(text);
        let node: web_sys::Node = text_node.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        let comment = self.browser_doc.create_comment(text);
        let node: web_sys::Node = comment.into();
        let id = self.alloc_id();
        set_nid(&node, id);
        self.nodes.insert(id.0, node);
        id
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            insert_or_move(&children_of(p), c, None);
        }
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            if c.parent_node().is_some_and(|cp| !cp.is_same_node(Some(p))) {
                children_of(p).remove_child(c).ok();
            } else {
                p.remove_child(c).ok();
            }
        }
    }

    fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if let (Some(p), Some(c), Some(r)) = (
            self.nodes.get(&parent.0),
            self.nodes.get(&child.0),
            self.nodes.get(&reference.0),
        ) {
            // Into the shadow root when the reference is there (#2357).
            let container = r.parent_node().unwrap_or_else(|| p.clone());
            let container = if container.is_same_node(Some(p)) {
                p.clone()
            } else {
                children_of(p)
            };
            insert_or_move(&container, c, Some(r));
        }
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // `replaceChild(n, n)` succeeds — the DOM spec re-inserts `n` before its
        // own next sibling — so guard it rather than let the detach below
        // report a node as displaced when it is still exactly where it was.
        if old == new {
            return;
        }
        let (Some(old_node), Some(new_node)) =
            (self.nodes.get(&old.0).cloned(), self.nodes.get(&new.0))
        else {
            return;
        };
        let Some(parent) = old_node.parent_node() else {
            return;
        };
        // `old` is **detached**, not retired: it keeps both map entries and a
        // handle to it can be inserted again, exactly as on desktop (issue
        // #719). The callers that are finished with it — the editor's
        // `ViewDesc` diff, a per-keystroke path — say so with `discard_node`,
        // which is what releases it (issue #184).
        parent.replace_child(new_node, &old_node).ok();
    }

    fn renew_element(&mut self, node: NodeId) -> bool {
        // A shallow clone is the same tag with the same attributes and none of
        // the element's own state: a canvas clone has no context (#1768).
        let Some(old) = self.nodes.get(&node.0).cloned() else {
            return false;
        };
        let Some(fresh) = old
            .clone_node()
            .ok()
            .filter(|n| n.node_type() == web_sys::Node::ELEMENT_NODE)
        else {
            return false;
        };
        while let Some(child) = old.first_child() {
            if fresh.append_child(&child).is_err() {
                return false;
            }
        }
        if let Some(parent) = old.parent_node()
            && parent.replace_child(&fresh, &old).is_err()
        {
            return false;
        }
        // The id now names the new element in both maps, and the element
        // carries it for the reverse lookups.
        self.nodes.insert(node.0, fresh.clone());
        set_nid(&fresh, node);
        // A bounds signal on the node follows it to the new element (#2813).
        if let Some(el) = fresh.dyn_ref::<web_sys::Element>() {
            crate::bounds::renew(self.doc_key, node.0, el);
        }
        true
    }

    fn remove_node(&mut self, node: NodeId) {
        // No `cloned()`, unlike `discard_node` below: nothing here mutates
        // `self.nodes`, so the borrow can live across the detach.
        let Some(n) = self.nodes.get(&node.0) else {
            return;
        };
        if let Some(parent) = n.parent_node() {
            parent.remove_child(n).ok();
        }
        // Both map entries stay. A removed node must be re-insertable — append
        // its handle again and the whole subtree comes back, which is the
        // post-condition desktop has always had and the one a reactive branch
        // re-showing a *captured* handle rests on (issue #719). Pruning here is
        // what made that silently lose the subtree on this backend.
        //
        // The bookkeeping is released by `discard_node` instead, which every
        // caller that throws a subtree away calls (issue #184).
    }

    /// Detach `node` and drop it and every `__nid`-tagged descendant from both
    /// node maps (issues #184, #719).
    ///
    /// This is where the browser backend actually lets go: both maps hold a
    /// *strong* `web_sys::Node`, so an entry pins its browser node against GC
    /// for the life of the wasm module, and node creation is not structural —
    /// a keyed `for`, a `show`, or the editor's per-keystroke `ViewDesc` churn
    /// creates and destroys nodes forever within one mounted root.
    fn discard_node(&mut self, node: NodeId) {
        let Some(n) = self.nodes.get(&node.0).cloned() else {
            return;
        };
        if let Some(parent) = n.parent_node() {
            parent.remove_child(&n).ok();
        }
        // Prune *unconditionally*, outside the `parent_node()` guard: a node
        // that was built and never appended (or is already detached — every
        // `discard` after a `replace_node` is) is stranded just as hard as an
        // attached one. Descendants stay attached to `n` when `n` leaves its
        // parent, so the sweep works either side of the detach.
        forget_subtree(&mut self.nodes, &n);
        // A selection box among them stops painting its text or cell (#2667).
        self.highlights.forget_retired(&self.nodes);
    }

    fn is_retired(&self, node: NodeId) -> bool {
        !self.nodes.contains_key(&node.0)
    }

    /// The element's local name: lowercase for an HTML element, the author's
    /// spelling for an SVG one — what desktop stores (#1784, so a key listener
    /// can ask `active_element()?.tag_name()` on both backends).
    fn tag_name(&self, node: NodeId) -> Option<String> {
        let n = self.nodes.get(&node.0)?;
        let el: &web_sys::Element = n.dyn_ref()?;
        Some(el.local_name())
    }

    fn mark_dirty(&mut self, _node: NodeId) {
        // No-op: browser handles reflow automatically.
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        // No-op: browser handles reflow automatically.
        Vec::new()
    }

    fn root(&self) -> NodeId {
        self.root_id
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, selector: &str) -> Option<NodeId> {
        let el = warn_invalid_selector(selector, self.browser_doc.query_selector(selector))??;
        let node: web_sys::Node = el.into();
        get_nid(&node)
    }

    fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        node_list_ids(warn_invalid_selector(
            selector,
            self.browser_doc.query_selector_all(selector),
        ))
    }

    /// `Element.querySelector` on the scope (#1796); a document scope asks
    /// it, and any other node (a text node) has no descendants.
    fn query_selector_in(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        let n = self.nodes.get(&scope.0)?;
        let found = if let Some(el) = n.dyn_ref::<web_sys::Element>() {
            el.query_selector(selector)
        } else {
            n.dyn_ref::<web_sys::Document>()?.query_selector(selector)
        };
        let node: web_sys::Node = warn_invalid_selector(selector, found)??.into();
        get_nid(&node)
    }

    fn query_selector_all_in(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        let Some(n) = self.nodes.get(&scope.0) else {
            return Vec::new();
        };
        let found = if let Some(el) = n.dyn_ref::<web_sys::Element>() {
            el.query_selector_all(selector)
        } else if let Some(d) = n.dyn_ref::<web_sys::Document>() {
            d.query_selector_all(selector)
        } else {
            return Vec::new();
        };
        node_list_ids(warn_invalid_selector(selector, found))
    }

    /// The DOM `nodeType` (1 element, 3 text, 8 comment), as desktop's.
    fn node_type(&self, node: NodeId) -> Option<u16> {
        self.nodes.get(&node.0).map(|n| n.node_type())
    }

    fn is_text_node(&self, node: NodeId) -> bool {
        self.nodes
            .get(&node.0)
            .is_some_and(|n| n.node_type() == web_sys::Node::TEXT_NODE)
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        if let Some(n) = self.nodes.get(&node.0) {
            // An `isolate_styles` host's light children (those it had
            // before), then its shadow root's: desktop's order (#2357).
            let mut lists = vec![n.child_nodes()];
            if let Some(shadow) = shadow_of(n) {
                lists.push(shadow.child_nodes());
            }
            for children in lists {
                for i in 0..children.length() {
                    if let Some(child) = children.item(i)
                        && let Some(nid) = get_nid(&child)
                    {
                        result.push(nid);
                    }
                }
            }
        }
        result
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        if let (Some(p), Some(c)) = (self.nodes.get(&parent.0), self.nodes.get(&child.0)) {
            // The index counts what `get_children` lists: an `isolate_styles`
            // host's light children, then its shadow root's (#2357); one
            // past the light ones goes into the shadow root.
            let light = p.child_nodes().length() as usize;
            let (p, index) = match shadow_of(p) {
                Some(shadow) if index >= light => (shadow.into(), index - light),
                _ => (p.clone(), index),
            };
            // `item` past the end is `None`, which appends.
            let reference = p.child_nodes().item(index as u32);
            insert_or_move(&p, c, reference.as_ref());
        }
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let parent = parent_of(n)?;
        get_nid(&parent)
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        let n = self.nodes.get(&node.0)?;
        let sibling = n.next_sibling()?;
        get_nid(&sibling)
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        let temp = self.browser_doc.create_element("div").ok()?;
        temp.set_inner_html(html);
        let first_child = temp.first_child()?;
        // Register the subtree
        self.register_subtree(&first_child);
        get_nid(&first_child)
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            // `set_inner_html` discards every existing child, so forget them
            // first — the desktop backend already does this (issue #184). One
            // registry borrow for the whole sweep, not one per child.
            forget_children(&mut self.nodes, &el);
            // An `isolate_styles` element's children are its shadow root's
            // (#2357).
            let shadow = el.shadow_root();
            match &shadow {
                Some(shadow) => shadow.set_inner_html(html),
                None => el.set_inner_html(html),
            }
            // Walk all new child nodes and register them
            let children = children_of(&el).child_nodes();
            let mut parsed = Vec::new();
            for i in 0..children.length() {
                if let Some(child) = children.item(i) {
                    self.register_subtree(&child);
                    parsed.extend(child.dyn_into::<web_sys::Element>().ok());
                }
            }
            // Parsed markup skipped `set_attribute`: an `<img>` source the
            // app answers for goes through the same path as one set one.
            if shadow.is_none() {
                crate::images::adopt_parsed_images(&el);
            } else {
                for child in &parsed {
                    crate::images::adopt_parsed_images(child);
                }
            }
        }
    }
}
