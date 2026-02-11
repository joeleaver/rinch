//! Direct blitz document adapter for the DomDocument trait.
//!
//! This module provides an implementation of [`DomDocument`] that operates
//! directly on blitz's [`HtmlDocument`] via the [`DocumentMutator`] API.
//!
//! Unlike the virtual DOM approach, this adapter:
//! - Creates nodes directly in blitz
//! - Mutations go straight to the underlying document
//! - No HTML serialization needed
//! - All operations (text, attrs, structural) are treated uniformly

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use blitz_dom::{BaseDocument, Document, DocumentMutator, QualName, LocalName, ns};
use rinch_core::dom::{DomDocument, NodeId};

// Stylo types for initializing element styles
use style::data::{ElementData, ElementStyles};
use style::properties::ComputedValues;
use style::properties::style_structs::Font;

/// A document adapter that operates directly on blitz's Document.
///
/// This wraps a blitz HtmlDocument and implements the DomDocument trait
/// by delegating to the DocumentMutator API. All DOM operations go
/// directly to blitz - no virtual DOM layer, no HTML serialization.
pub struct BlitzDocumentAdapter {
    /// The underlying blitz document (borrowed from ManagedWindow).
    doc: Weak<RefCell<Box<dyn Document>>>,
    /// The body element ID (where content is appended).
    body_id: NodeId,
    /// Whether the document needs re-layout.
    dirty: Cell<bool>,
    /// Dirty node IDs for incremental style traversal.
    dirty_nodes: RefCell<Vec<NodeId>>,
    /// Nodes detached from tree but not yet dropped from the slab.
    /// Deferred removal prevents blitz from panicking on stale slab keys
    /// during `resolve()`, which runs after all reactive effects complete.
    tombstone_nodes: RefCell<Vec<usize>>,
    /// Cache of parsed style properties per node.
    /// Avoids re-parsing the style attribute string on every `set_style()` call.
    style_cache: RefCell<HashMap<usize, HashMap<String, String>>>,
}

impl BlitzDocumentAdapter {
    /// Create a new adapter wrapping a blitz document.
    ///
    /// The document should have at least `<html><body></body></html>` structure.
    pub fn new(doc: Weak<RefCell<Box<dyn Document>>>) -> Self {
        // Find the body element
        let body_id = {
            let doc_rc = doc.upgrade().expect("Document dropped");
            let doc_ref = doc_rc.borrow();
            let inner = doc_ref.inner();
            find_body_id(&*inner)
        };

        Self {
            doc,
            body_id,
            dirty: Cell::new(false),
            dirty_nodes: RefCell::new(Vec::new()),
            tombstone_nodes: RefCell::new(Vec::new()),
            style_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Get the body element ID.
    pub fn body_id(&self) -> NodeId {
        self.body_id
    }

    /// Check if the document is dirty (needs re-layout).
    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Clear the dirty flag.
    pub fn clear_dirty(&self) {
        self.dirty.set(false);
    }

    /// Get the underlying document reference.
    pub fn doc(&self) -> Option<Rc<RefCell<Box<dyn Document>>>> {
        self.doc.upgrade()
    }

    fn with_mutator<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut DocumentMutator<'_>) -> R,
    {
        let doc_rc = self.doc.upgrade()?;
        let mut doc_ref = doc_rc.borrow_mut();
        let mut inner = doc_ref.inner_mut();
        let mut mutator = inner.mutate();
        Some(f(&mut mutator))
    }

    fn with_inner<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&BaseDocument) -> R,
    {
        let doc_rc = self.doc.upgrade()?;
        let doc_ref = doc_rc.borrow();
        let inner = doc_ref.inner();
        Some(f(&*inner))
    }

    /// Drop all tombstoned nodes from the slab.
    ///
    /// Call this AFTER `resolve()` completes, so that blitz never encounters
    /// stale slab keys during style resolution or paint traversal.
    pub fn drop_tombstoned_nodes(&self) {
        let nodes: Vec<usize> = self.tombstone_nodes.borrow_mut().drain(..).collect();
        if nodes.is_empty() {
            return;
        }
        tracing::debug!("Dropping {} tombstoned nodes", nodes.len());
        if let Some(doc_rc) = self.doc.upgrade() {
            let mut doc_ref = doc_rc.borrow_mut();
            let mut inner = doc_ref.inner_mut();
            let mut mutator = inner.mutate();
            for node_id in nodes {
                mutator.remove_and_drop_node(node_id);
            }
        }
    }

    fn mark_node_dirty(&self, node_id: NodeId) {
        self.dirty.set(true);
        self.dirty_nodes.borrow_mut().push(node_id);
    }

    /// Check if a node exists in the document.
    fn node_exists(&self, node_id: NodeId) -> bool {
        self.with_inner(|inner| inner.get_node(node_id.0).is_some())
            .unwrap_or(false)
    }

    /// Find an element by ID and return its NodeId.
    pub fn find_by_id(&self, id: &str) -> Option<NodeId> {
        self.with_inner(|inner| find_element_by_id(inner, id)).flatten()
    }

    /// Update the text content of an element (for updating style elements).
    pub fn update_element_text(&self, node_id: NodeId, text: &str) {
        // Guard: check parent node exists
        if !self.node_exists(node_id) {
            tracing::warn!("update_element_text: node doesn't exist ({})", node_id.0);
            return;
        }
        // For style elements, we need to update the first text child
        if let Some(children) = self.with_inner(|inner| {
            inner.get_node(node_id.0).map(|n| n.children.clone())
        }).flatten() {
            if let Some(&first_child) = children.first() {
                // Guard: check child node still exists
                if !self.node_exists(NodeId(first_child)) {
                    tracing::warn!("update_element_text: child node doesn't exist ({})", first_child);
                    return;
                }
                self.with_mutator(|mutator| {
                    mutator.set_node_text(first_child, text);
                });
                self.mark_node_dirty(NodeId(first_child));
            }
        }
    }

    /// Update the theme CSS by finding the theme style element and updating it.
    pub fn update_theme_css(&self, css: &str) {
        if let Some(style_id) = self.find_by_id("rinch-theme") {
            self.update_element_text(style_id, css);
        }
    }
}

impl DomDocument for BlitzDocumentAdapter {
    fn create_element(&mut self, tag: &str) -> NodeId {
        NodeId(
            self.with_mutator(|mutator| {
                let name = QualName::new(None, ns!(html), LocalName::from(tag));
                let id = mutator.create_element(name, vec![]);

                // Initialize element styles with initial computed values.
                // This is necessary for stylo's incremental style traversal which
                // assumes all elements have styles when checking should_process_descendants.
                // Without this, resolve() panics on is_display_none() for unstyled elements.
                if let Some(node) = mutator.doc.get_node(id) {
                    *node.stylo_element_data.borrow_mut() = Some(ElementData {
                        styles: ElementStyles {
                            primary: Some(
                                ComputedValues::initial_values_with_font_override(
                                    Font::initial_values(),
                                )
                                .to_arc(),
                            ),
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                }

                id
            })
            .unwrap_or(0),
        )
    }

    fn create_text(&mut self, text: &str) -> NodeId {
        NodeId(
            self.with_mutator(|mutator| mutator.create_text_node(text))
                .unwrap_or(0),
        )
    }

    fn create_comment(&mut self, text: &str) -> NodeId {
        // Blitz's create_comment_node doesn't take text, so we create it empty
        // and the text is just for our internal tracking
        let _ = text;
        NodeId(
            self.with_mutator(|mutator| mutator.create_comment_node())
                .unwrap_or(0),
        )
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // Guard: check both nodes exist
        if !self.node_exists(parent) || !self.node_exists(child) {
            tracing::warn!("append_child: node doesn't exist (parent={}, child={})", parent.0, child.0);
            return;
        }
        self.with_mutator(|mutator| {
            mutator.append_children(parent.0, &[child.0]);
        });
        self.mark_node_dirty(parent);
    }

    fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        // Guard: check child exists (parent may already be gone)
        if !self.node_exists(child) {
            tracing::warn!("remove_child: child node doesn't exist ({})", child.0);
            return;
        }
        // In blitz, we just remove the child - it tracks parent internally
        self.with_mutator(|mutator| {
            mutator.remove_node(child.0);
        });
        if self.node_exists(parent) {
            self.mark_node_dirty(parent);
        }
    }

    fn insert_before(&mut self, _parent: NodeId, child: NodeId, reference: NodeId) {
        // Guard: check nodes exist
        if !self.node_exists(child) || !self.node_exists(reference) {
            tracing::warn!("insert_before: node doesn't exist (child={}, ref={})", child.0, reference.0);
            return;
        }
        self.with_mutator(|mutator| {
            mutator.insert_nodes_before(reference.0, &[child.0]);
        });
        self.mark_node_dirty(child);
    }

    fn replace_node(&mut self, old: NodeId, new: NodeId) {
        // Guard: check old node exists (new should be freshly created)
        if !self.node_exists(old) {
            tracing::warn!("replace_node: old node doesn't exist ({})", old.0);
            return;
        }
        self.with_mutator(|mutator| {
            mutator.replace_node_with(old.0, &[new.0]);
        });
        self.mark_node_dirty(new);
    }

    fn remove_node(&mut self, node: NodeId) {
        // Guard: check node exists
        if !self.node_exists(node) {
            tracing::warn!("remove_node: node doesn't exist ({})", node.0);
            return;
        }
        let parent_id = self.with_inner(|inner| {
            inner.get_node(node.0).and_then(|n| n.parent)
        });
        // Clean up style cache for this node
        self.style_cache.borrow_mut().remove(&node.0);
        // Detach node from tree but don't drop from slab yet.
        // The slab slot is reclaimed later in drop_tombstoned_nodes(),
        // which runs after resolve() so blitz never hits stale keys.
        self.with_mutator(|mutator| {
            mutator.remove_node(node.0);
        });
        self.tombstone_nodes.borrow_mut().push(node.0);
        if let Some(Some(parent)) = parent_id {
            self.mark_node_dirty(NodeId(parent));
        } else {
            self.dirty.set(true);
        }
    }

    fn set_text_content(&mut self, node: NodeId, text: &str) {
        // Guard: check node exists
        if !self.node_exists(node) {
            tracing::warn!("set_text_content: node doesn't exist ({})", node.0);
            return;
        }

        // Check if this is a text node or an element node
        let is_text_node = self.with_inner(|inner| {
            inner.get_node(node.0)
                .map(|n| n.is_text_node())
                .unwrap_or(false)
        }).unwrap_or(false);

        if is_text_node {
            // For text nodes, update directly
            self.with_mutator(|mutator| {
                mutator.set_node_text(node.0, text);
            });
            self.mark_node_dirty(node);
        } else {
            // For element nodes, find or create a text child.
            let text_child_id = self.with_inner(|inner| {
                let element = inner.get_node(node.0)?;
                for &child_id in element.children.iter() {
                    if let Some(child) = inner.get_node(child_id) {
                        if child.is_text_node() {
                            return Some(child_id);
                        }
                    }
                }
                None
            }).flatten();

            if let Some(text_node_id) = text_child_id {
                // Update existing text child
                self.with_mutator(|mutator| {
                    mutator.set_node_text(text_node_id, text);
                });
                self.mark_node_dirty(node);
            } else {
                // No text child exists — create one
                let text_node_id = self.with_mutator(|mutator| {
                    let id = mutator.create_text_node(text);
                    mutator.append_children(node.0, &[id]);
                    id
                });
                if let Some(id) = text_node_id {
                    self.mark_node_dirty(NodeId(id));
                }
                self.mark_node_dirty(node);
            }
        }
    }

    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str) {
        // Guard: check node exists
        if !self.node_exists(node) {
            tracing::warn!("set_attribute: node doesn't exist ({})", node.0);
            return;
        }
        // Invalidate style cache when style attribute is set directly
        if name == "style" {
            self.style_cache.borrow_mut().remove(&node.0);
        }
        self.with_mutator(|mutator| {
            let qname = QualName::new(None, ns!(), LocalName::from(name));
            mutator.set_attribute(node.0, qname, value);
        });
        self.mark_node_dirty(node);
    }

    fn remove_attribute(&mut self, node: NodeId, name: &str) {
        // Guard: check node exists
        if !self.node_exists(node) {
            tracing::warn!("remove_attribute: node doesn't exist ({})", node.0);
            return;
        }
        self.with_mutator(|mutator| {
            let qname = QualName::new(None, ns!(), LocalName::from(name));
            mutator.clear_attribute(node.0, qname);
        });
        self.mark_node_dirty(node);
    }

    fn get_attribute(&self, node: NodeId, name: &str) -> Option<String> {
        self.with_inner(|inner| {
            inner.get_node(node.0).and_then(|n| {
                n.element_data().and_then(|el| {
                    el.attrs
                        .iter()
                        .find(|a| a.name.local.as_ref() == name)
                        .map(|a| a.value.to_string())
                })
            })
        })
        .flatten()
    }

    fn set_style(&mut self, node: NodeId, property: &str, value: &str) {
        // Guard: check node exists
        if !self.node_exists(node) {
            tracing::warn!("set_style: node doesn't exist ({})", node.0);
            return;
        }
        // We set the style attribute directly instead of using set_style_property
        // because blitz's set_style_property doesn't properly trigger style
        // recalculation during incremental updates. Setting the attribute does.

        let mut cache = self.style_cache.borrow_mut();
        let styles = cache.entry(node.0).or_insert_with(|| {
            // Parse existing style attribute into the cache on first access
            let current_style = self.get_attribute(node, "style").unwrap_or_default();
            current_style
                .split(';')
                .filter_map(|s| {
                    let s = s.trim();
                    if s.is_empty() { return None; }
                    let mut parts = s.splitn(2, ':');
                    let prop = parts.next()?.trim().to_string();
                    let val = parts.next()?.trim().to_string();
                    Some((prop, val))
                })
                .collect()
        });

        // Update or insert the property
        styles.insert(property.to_string(), value.to_string());

        // Build the new style string from cache
        let new_style: String = styles
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("; ");

        // Set the style attribute
        self.with_mutator(|mutator| {
            let qname = QualName::new(None, ns!(), LocalName::from("style"));
            mutator.set_attribute(node.0, qname, &new_style);
        });
        self.mark_node_dirty(node);
    }

    fn mark_dirty(&mut self, node: NodeId) {
        self.mark_node_dirty(node);
    }

    fn take_dirty_nodes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut *self.dirty_nodes.borrow_mut())
    }

    fn root(&self) -> NodeId {
        // Root is typically node 0 in blitz
        NodeId(0)
    }

    fn body(&self) -> NodeId {
        self.body_id
    }

    fn query_selector(&self, _selector: &str) -> Option<NodeId> {
        // TODO: Implement query selector if needed
        None
    }

    fn get_children(&self, node: NodeId) -> Vec<NodeId> {
        self.with_inner(|inner| {
            inner
                .get_node(node.0)
                .map(|n| n.children.iter().map(|&c| NodeId(c)).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    fn parent_node(&self, node: NodeId) -> Option<NodeId> {
        self.with_inner(|inner| {
            inner.get_node(node.0).and_then(|n| n.parent).map(NodeId)
        }).flatten()
    }

    fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        self.with_inner(|inner| {
            let parent_id = inner.get_node(node.0)?.parent?;
            let parent = inner.get_node(parent_id)?;
            let pos = parent.children.iter().position(|&c| c == node.0)?;
            parent.children.get(pos + 1).map(|&c| NodeId(c))
        }).flatten()
    }

    fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
        // Get children, find the reference node at index
        let children = self.get_children(parent);
        if index >= children.len() {
            // Append at end
            self.append_child(parent, child);
        } else {
            // Insert before the node at index
            let reference = children[index];
            self.insert_before(parent, child, reference);
        }
    }

    fn parse_html(&mut self, html: &str) -> Option<NodeId> {
        // Use blitz's set_inner_html to parse HTML into a temporary container
        // For now, we don't support inline HTML parsing - build DOM directly
        let _ = html;
        None
    }

    fn set_inner_html(&mut self, node: NodeId, html: &str) {
        if !self.node_exists(node) {
            tracing::warn!("set_inner_html: node doesn't exist ({})", node.0);
            return;
        }
        self.with_mutator(|mutator| {
            mutator.set_inner_html(node.0, html);

            // Nodes created by blitz's HTML parser lack initialized stylo_element_data.
            // Without it, stylo's incremental traversal skips them (should_process_descendants
            // returns false for unstyled elements). Walk the subtree and initialize styles
            // using the same pattern as create_element.
            fn init_styles_recursive(doc: &blitz_dom::BaseDocument, node_id: usize, depth: usize) -> usize {
                let Some(node) = doc.get_node(node_id) else { return 0 };
                let mut count = 0;
                if node.element_data().is_some() {
                    let needs_init = node.stylo_element_data.borrow().is_none();
                    if needs_init {
                        *node.stylo_element_data.borrow_mut() = Some(ElementData {
                            styles: ElementStyles {
                                primary: Some(
                                    ComputedValues::initial_values_with_font_override(
                                        Font::initial_values(),
                                    )
                                    .to_arc(),
                                ),
                                ..Default::default()
                            },
                            ..Default::default()
                        });
                        count += 1;
                    }
                }
                let children: Vec<usize> = node.children.clone();
                for child_id in children {
                    count += init_styles_recursive(doc, child_id, depth + 1);
                }
                count
            }
            let initialized = init_styles_recursive(&mutator.doc, node.0, 0);
            tracing::info!("set_inner_html: initialized styles on {} elements under node {}", initialized, node.0);
        });
        self.mark_node_dirty(node);
    }

    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        // Set the scroll offset on the node directly
        let node_id = node.0;
        if let Some(doc_rc) = self.doc.upgrade() {
            let mut doc_ref = doc_rc.borrow_mut();
            let mut inner = doc_ref.inner_mut();
            if let Some(blitz_node) = inner.get_node_mut(node_id) {
                let old_scroll = blitz_node.scroll_offset.y;
                blitz_node.scroll_offset.y = scroll_top;
                tracing::info!(
                    "set_scroll_top: node={}, old={}, new={}",
                    node_id, old_scroll, scroll_top
                );
            } else {
                tracing::warn!("set_scroll_top: node {} not found", node_id);
            }
        }
        self.mark_node_dirty(node);
    }
}

/// Find the body element ID in a blitz document.
fn find_body_id(inner: &BaseDocument) -> NodeId {
    find_element_by_tag(inner, "body").unwrap_or(NodeId(0))
}

/// Find an element by tag name.
fn find_element_by_tag(inner: &BaseDocument, tag: &str) -> Option<NodeId> {
    fn find_recursive(inner: &BaseDocument, node_id: usize, tag: &str) -> Option<NodeId> {
        let node = inner.get_node(node_id)?;
        if let Some(el) = node.element_data() {
            if el.name.local.as_ref() == tag {
                return Some(NodeId(node_id));
            }
        }
        for &child_id in &node.children {
            if let Some(found) = find_recursive(inner, child_id, tag) {
                return Some(found);
            }
        }
        None
    }
    find_recursive(inner, 0, tag)
}

/// Find an element by ID attribute.
fn find_element_by_id(inner: &BaseDocument, id: &str) -> Option<NodeId> {
    fn find_recursive(inner: &BaseDocument, node_id: usize, id: &str) -> Option<NodeId> {
        let node = inner.get_node(node_id)?;
        if let Some(el) = node.element_data() {
            if el.attrs.iter().any(|a| a.name.local.as_ref() == "id" && a.value.to_string() == id) {
                return Some(NodeId(node_id));
            }
        }
        for &child_id in &node.children {
            if let Some(found) = find_recursive(inner, child_id, id) {
                return Some(found);
            }
        }
        None
    }
    find_recursive(inner, 0, id)
}

/// Shared reference to a BlitzDocumentAdapter.
pub type SharedBlitzDocument = Rc<RefCell<BlitzDocumentAdapter>>;
