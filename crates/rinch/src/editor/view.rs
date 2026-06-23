//! [`RinchDomEditorView`] and its [`ViewDesc`] tree — the desktop projection of
//! `EditorState` onto rinch-dom (design §6).

use std::cell::RefCell;
use std::rc::Weak;

use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_editor_core::decoration::DecorationSet;
use rinch_editor_core::serialize::{mark_dom_tag, node_dom_tag};
use rinch_editor_core::{EditorState, EditorView, Mark, Node, Pos, ViewRequest};

/// A weak handle to the host document. The view upgrades + borrows it briefly to
/// create/patch nodes; holding only a `Weak` lets the document drop normally.
type DocRef = Weak<RefCell<dyn DomDocument>>;

/// Create a host element and wrap it in a [`NodeHandle`]. `None` if the document
/// has been dropped.
fn create_element(doc: &DocRef, tag: &str) -> Option<NodeHandle> {
    let d = doc.upgrade()?;
    let id = d.borrow_mut().create_element(tag);
    Some(NodeHandle::new(id, doc.clone()))
}

/// Create a host text node and wrap it in a [`NodeHandle`].
fn create_text(doc: &DocRef, text: &str) -> Option<NodeHandle> {
    let d = doc.upgrade()?;
    let id = d.borrow_mut().create_text(text);
    Some(NodeHandle::new(id, doc.clone()))
}

/// Set the host element's attributes from `node`: a `data-pm-type` marker (its
/// schema type, a debugging/MCP/a11y aid) plus the node-specific attributes the
/// tag itself does not encode. Idempotent — clears attributes that no longer
/// apply, so an in-place update can't leave a stale `start`/`src`.
fn apply_element_attrs(dom: &NodeHandle, node: &Node) {
    dom.set_attribute("data-pm-type", node.type_name());
    match node.type_name() {
        "image" => {
            dom.set_attribute("src", node.attrs().get_str("src").unwrap_or(""));
            match node.attrs().get_str("alt") {
                Some(alt) if !alt.is_empty() => dom.set_attribute("alt", alt),
                _ => dom.remove_attribute("alt"),
            }
        }
        "ordered_list" => match node.attrs().get_int("start") {
            Some(start) if start != 1 => dom.set_attribute("start", &start.to_string()),
            _ => dom.remove_attribute("start"),
        },
        _ => {}
    }
}

/// Set a mark wrapper element's attributes: a `data-pm-mark` marker plus the
/// attr-bearing specials (`link` → `href`/`title`/`target`+`rel`, `text_color` →
/// inline `color`, `highlight` → inline `background-color`). Mirrors the
/// schema-driven copy-out HTML in `serialize::html`.
fn apply_mark_attrs(dom: &NodeHandle, mark: &Mark) {
    dom.set_attribute("data-pm-mark", mark.type_name());
    match mark.type_name() {
        "link" => {
            dom.set_attribute("href", mark.attrs.get_str("href").unwrap_or(""));
            if let Some(title) = mark.attrs.get_str("title").filter(|t| !t.is_empty()) {
                dom.set_attribute("title", title);
            }
            if let Some(target) = mark.attrs.get_str("target").filter(|t| !t.is_empty()) {
                dom.set_attribute("target", target);
                if target != "_self" {
                    // Guard against reverse-tabnabbing, as copy-out HTML does.
                    dom.set_attribute("rel", "noopener noreferrer");
                }
            }
        }
        "text_color" => {
            if let Some(color) = mark.attrs.get_str("color").filter(|c| !c.is_empty()) {
                dom.set_style("color", color);
            }
        }
        "highlight" => {
            if let Some(color) = mark.attrs.get_str("color").filter(|c| !c.is_empty()) {
                dom.set_style("background-color", color);
            }
        }
        _ => {}
    }
}

/// Wrap `inner` in `marks`' host elements, innermost-first (`marks[0]` closest to
/// the text, matching copy-out HTML), and return the outermost node (or `inner`
/// itself when there are no marks / no mark renders to a tag).
fn wrap_marks(inner: &NodeHandle, marks: &[Mark], doc: &DocRef) -> NodeHandle {
    let mut current = inner.clone();
    for mark in marks {
        if let Some(tag) = mark_dom_tag(mark)
            && let Some(wrapper) = create_element(doc, &tag)
        {
            apply_mark_attrs(&wrapper, mark);
            wrapper.append_child(&current);
            current = wrapper;
        }
    }
    current
}

/// A descriptor mirroring one model [`Node`], owning the host node that projects
/// it. The conceptual successor to the old `BlockMap`, but a proper persistent
/// tree: because the model shares `Rc`s, [`Node::same_ref`] lets the diff skip an
/// unchanged subtree wholesale (design A12).
///
/// **Inline marks (M5.5a):** a marked inline run (`text("ab", [bold, italic])`)
/// projects to a *chain* of host nodes — `<strong><em>#text</em></strong>` — so
/// [`Self::dom`] (the inner text/leaf node, used for caret geometry and the A15
/// position map) is distinct from [`Self::outer`] (the outermost wrapper, the node
/// actually placed in / removed from the parent). For an unmarked node the two are
/// the same node. Wrappers are built **per run** (no cross-run sharing yet); a
/// shared-wrapper pass is a later refinement.
pub(crate) struct ViewDesc {
    /// The model node this descriptor currently projects.
    node: Node,
    /// The inner host node projecting `node` itself — a host text node for a text
    /// run, or the element for a block/leaf. Caret and the A15 map address this.
    dom: NodeHandle,
    /// The host node placed in the parent: the outermost inline mark wrapper, or
    /// (when `node` is unmarked) the same node as [`Self::dom`].
    outer: NodeHandle,
    /// Child descriptors, positionally 1:1 with `node`'s children.
    children: Vec<ViewDesc>,
    /// Whether [`Self::dom`] is a host text node (vs an element).
    is_text: bool,
}

impl ViewDesc {
    /// Build a descriptor (and its host subtree, including inline mark wrappers)
    /// for `node`.
    fn build(node: &Node, doc: &DocRef) -> Option<ViewDesc> {
        let (dom, children, is_text) = if let Some(text) = node.text() {
            (create_text(doc, text)?, Vec::new(), true)
        } else {
            let el = create_element(doc, &node_dom_tag(node))?;
            apply_element_attrs(&el, node);
            let mut children = Vec::with_capacity(node.child_count());
            for i in 0..node.child_count() {
                if let Some(child) = ViewDesc::build(node.child(i), doc) {
                    el.append_child(&child.outer);
                    children.push(child);
                }
            }
            (el, children, false)
        };
        // Inline marks (on a text run or inline leaf) wrap the node's own host
        // node; blocks are unmarked, so `outer == dom` there.
        let outer = wrap_marks(&dom, node.marks(), doc);
        Some(ViewDesc {
            node: node.clone(),
            dom,
            outer,
            children,
            is_text,
        })
    }

    /// Patch this descriptor in place to project `new`. Returns `false` if the
    /// kinds are incompatible (text vs element, a different element tag, or a
    /// changed mark set whose wrapper chain differs) and the caller must replace
    /// the whole subtree instead.
    fn update(&mut self, new: &Node, doc: &DocRef) -> bool {
        // Fast path: same persistent subtree — nothing changed (design A12).
        if self.node.same_ref(new) {
            return true;
        }
        // A changed mark set alters the wrapper chain (`outer`), which can't be
        // patched in place — force a rebuild. (Blocks are always unmarked.)
        if self.node.marks() != new.marks() {
            return false;
        }
        if new.is_text() {
            if !self.is_text {
                return false;
            }
            if self.node.text() != new.text() {
                self.dom.set_text(new.text().unwrap_or(""));
            }
            self.node = new.clone();
            return true;
        }
        // Element: the tag must match to patch in place.
        if self.is_text || node_dom_tag(&self.node) != node_dom_tag(new) {
            return false;
        }
        if self.node.attrs() != new.attrs() {
            apply_element_attrs(&self.dom, new);
        }
        self.diff_children(new, doc);
        self.node = new.clone();
        true
    }

    /// Reconcile this descriptor's children against `new`'s children: a positional
    /// diff that recurses (with the `same_ref` fast skip), replaces a child whose
    /// kind changed, appends new trailing children, and removes surplus ones.
    ///
    /// Positional (not keyed) is correct and minimal for text editing, where edits
    /// are local; a keyed/LIS pass can replace this if reorder churn ever matters.
    fn diff_children(&mut self, new: &Node, doc: &DocRef) {
        let new_count = new.child_count();
        for i in 0..new_count {
            let new_child = new.child(i);
            if i < self.children.len() {
                if self.children[i].update(new_child, doc) {
                    continue;
                }
                // Kind changed — build a replacement and swap it into the host
                // (by the placed `outer` node, which differs from `dom` for a
                // mark-wrapped run).
                if let Some(replacement) = ViewDesc::build(new_child, doc) {
                    self.children[i].outer.replace_with(&replacement.outer);
                    self.children[i] = replacement;
                }
            } else if let Some(new_desc) = ViewDesc::build(new_child, doc) {
                self.dom.append_child(&new_desc.outer);
                self.children.push(new_desc);
            }
        }
        while self.children.len() > new_count {
            // `pop` keeps removal O(1) and order-independent (host removal is by id).
            if let Some(extra) = self.children.pop() {
                extra.outer.remove();
            }
        }
    }

    /// The placed host node of the first block child, if any (the placeholder is
    /// inserted before it so it overlays the empty editor).
    fn first_child_dom(&self) -> Option<&NodeHandle> {
        self.children.first().map(|c| &c.outer)
    }
}

/// The desktop editor view: projects [`EditorState`] onto rinch-dom and keeps the
/// host in sync as transactions are applied (design §6). Implements the
/// renderer-agnostic [`EditorView`] seam.
pub struct RinchDomEditorView {
    doc: DocRef,
    /// The descriptor for the document root. Its [`ViewDesc::dom`] is the host
    /// container (created by the caller, e.g. the `Editor {}` component) and is
    /// **never replaced** — only its children diff.
    root: ViewDesc,
    /// The placeholder overlay node while shown — decoration-diff state (A4/A8).
    placeholder: Option<NodeHandle>,
    /// The caret overlay node — an absolutely-positioned bar the view owns and
    /// repositions from `state.selection` (design §6: caret is rendered *from*
    /// the selection, never via a `data-ce-cursor` attribute). It lives as a child
    /// of the **container** (never a textblock — a child node would disrupt the
    /// block's inline-formatting context), positioned in container space.
    caret: Option<NodeHandle>,
    /// The last rendered caret `(x, y, height)` (pixels rounded), so a re-run that
    /// lands on the same spot writes nothing — otherwise the caret's own style
    /// writes would re-dirty the tree and spin the repaint loop.
    last_caret: Option<(i32, i32, i32)>,
    /// The blink phase last applied to the caret (`Some(true)` = shown,
    /// `Some(false)` = hidden, `None` = no caret present). Guards
    /// [`Self::set_caret_blink_visible`] against redundant writes so a blink tick
    /// that doesn't change the phase doesn't re-dirty the tree.
    blink_shown: Option<bool>,
    /// Highlight rectangles for the current text selection (container children,
    /// reused across updates).
    selection_rects: Vec<NodeHandle>,
    /// The last rendered selection `(from, to)`, for change-detection.
    last_selection: Option<(usize, usize)>,
    /// The outline overlay for a [`Selection::Node`] — a translucent box tracing the
    /// selected leaf node (image / horizontal rule), drawn from `state.selection`
    /// (design §6 node-views). `None` when the current selection is not a node
    /// selection.
    node_outline: Option<NodeHandle>,
    /// The last rendered node-selection box `(x, y, w, h)` (pixels rounded), so a
    /// re-run on the same geometry writes nothing (mirrors [`Self::last_caret`]).
    last_node_outline: Option<(i32, i32, i32, i32)>,
    /// The decoration set currently projected, for the next diff.
    decorations: DecorationSet,
    /// Set whenever the post-layout pass writes an overlay's geometry (caret,
    /// selection rect, or node outline). The runtime reads it via
    /// [`Self::take_overlay_dirty`] to force a full repaint: these overlays are
    /// absolutely positioned, and the software renderer's dirty-region cache can't
    /// clear an absolute element's *old* rect when it moves, so a moved overlay
    /// would otherwise ghost.
    overlay_dirty: bool,
}

impl RinchDomEditorView {
    /// Build a view over `state`, rendering its document into `container` (the host
    /// element standing in for the `doc` node). `container` is owned by the caller
    /// and never replaced.
    pub fn new(container: NodeHandle, doc: DocRef, state: &EditorState) -> RinchDomEditorView {
        container.set_attribute("data-pm-type", state.doc.type_name());
        // The caret and selection overlays are positioned absolutely relative to the
        // container. A non-`auto` `z-index` makes the container a CSS stacking
        // context, so the selection rects (`z-index: -1`) paint *behind* the in-flow
        // text and the caret (`z-index: 1`) paints in front of it.
        container.set_styles(&[("position", "relative"), ("z-index", "0")]);
        let mut children = Vec::with_capacity(state.doc.child_count());
        for i in 0..state.doc.child_count() {
            if let Some(child) = ViewDesc::build(state.doc.child(i), &doc) {
                container.append_child(&child.outer);
                children.push(child);
            }
        }
        let root = ViewDesc {
            node: state.doc.clone(),
            outer: container.clone(),
            dom: container,
            children,
            is_text: false,
        };
        let mut view = RinchDomEditorView {
            doc,
            root,
            placeholder: None,
            caret: None,
            last_caret: None,
            blink_shown: None,
            selection_rects: Vec::new(),
            last_selection: None,
            node_outline: None,
            last_node_outline: None,
            decorations: DecorationSet::empty(),
            overlay_dirty: false,
        };
        view.sync_decorations(state);
        view
    }

    /// Diff `state`'s decorations against the projected set and patch the overlay
    /// nodes — independent of the document diff (design A4). M5.4 handles the
    /// placeholder widget only; M6 adds the IME preedit and richer kinds.
    fn sync_decorations(&mut self, state: &EditorState) {
        let next = state.decorations();
        if next == self.decorations {
            return;
        }
        let placeholder_text = next.iter().find_map(|d| d.as_placeholder());
        match (placeholder_text, self.placeholder.is_some()) {
            (Some(text), false) => {
                if let Some(node) = create_element(&self.doc, "div") {
                    node.set_attribute("data-pm-placeholder", "true");
                    node.set_attribute("class", "rinch-editor-placeholder");
                    if let Some(t) = create_text(&self.doc, text) {
                        node.append_child(&t);
                    }
                    // Overlay the (empty) first block; if none, append to the root.
                    match self.root.first_child_dom() {
                        Some(first) => self.root.dom.insert_before(&node, first),
                        None => self.root.dom.append_child(&node),
                    }
                    self.placeholder = Some(node);
                }
            }
            (None, true) => {
                if let Some(node) = self.placeholder.take() {
                    node.remove();
                }
            }
            // (Some, true): placeholder stays (its text is fixed per editor).
            // (None, false): nothing to do.
            _ => {}
        }
        self.decorations = next;
    }
}

impl EditorView for RinchDomEditorView {
    fn update_dom(&mut self, _prev: &EditorState, next: &EditorState) -> Vec<ViewRequest> {
        // Phase 1 (before layout): diff the document and patch the host. The root's
        // host element is fixed, so only its children reconcile.
        self.root.diff_children(&next.doc, &self.doc);
        self.root.node = next.doc.clone();
        // Decoration diff, independent of the document diff (A4).
        self.sync_decorations(next);
        Vec::new()
    }

    fn update_caret(&mut self, next: &EditorState) -> Vec<ViewRequest> {
        // Phase 2 (after layout): render the selection from `next.selection`.
        // A node selection (a selected image / horizontal rule) outlines the node
        // and shows neither a text highlight nor a caret (design §6 node-views).
        if let rinch_editor_core::Selection::Node(_) = &next.selection {
            self.clear_selection_rects();
            self.hide_caret();
            self.render_node_selection(next);
            return vec![ViewRequest::ScrollSelectionIntoView];
        }
        self.clear_node_selection();
        // Otherwise: the text selection highlight (for a range) and the caret (at a
        // collapsed cursor).
        self.render_selection(next);
        // No caret while a range is selected — the highlight conveys the head, a
        // caret blinking over a highlight reads as noise, and with no caret the
        // blink loop idles (`set_caret_blink` reports "nothing to blink").
        if !next.selection.is_empty() {
            self.hide_caret();
            return Vec::new();
        }
        let Some((block, flat_byte)) = self.caret_target(&next.doc, next.selection.head()) else {
            self.hide_caret();
            return Vec::new();
        };
        let Some(doc) = self.doc.upgrade() else {
            return Vec::new();
        };
        let block_id = block.node_id().0;
        // Query Parley for the caret geometry (layout-local to the textblock), then
        // translate into the container's coordinate space. An *empty* textblock has
        // no Parley layout, so `query_caret_position` returns `None` there — fall
        // back to the block's content origin with a sensible line height, so the
        // caret is still visible on a blank line.
        let geometry = {
            let d = doc.borrow();
            let from_parley =
                d.query_caret_position(block_id as u64, flat_byte)
                    .map(|(local_x, local_y)| {
                        let height = d
                            .query_glyph_bounds(block_id as u64, flat_byte)
                            .map(|g| g.height)
                            .unwrap_or(18.0);
                        let (ox, oy) = self.block_offset_in_container(&*d, block_id);
                        (ox + local_x, oy + local_y, height)
                    });
            from_parley
                .or_else(|| self.empty_block_caret(&*d, &next.doc, next.selection.head(), block_id))
        };
        match geometry {
            Some((x, y, height)) => self.position_caret(x, y, height),
            None => self.hide_caret(),
        }
        vec![ViewRequest::ScrollSelectionIntoView]
    }
}

impl RinchDomEditorView {
    /// The host id of the editor container (the `doc` node's element).
    pub(crate) fn container_id(&self) -> usize {
        self.root.dom.node_id().0
    }

    /// Resolve a model [`Pos`] to its host caret address `(textblock element id,
    /// flat UTF-8 byte offset)` — the address app-side geometry (caret point,
    /// vertical movement) queries Parley with. `None` if `pos` isn't in a textblock.
    pub(crate) fn caret_address(&self, doc: &Node, pos: Pos) -> Option<(usize, usize)> {
        self.caret_target(doc, pos)
            .map(|(dom, byte)| (dom.node_id().0, byte))
    }

    /// Resolve a model [`Pos`] to the host caret address `(textblock element, flat
    /// UTF-8 byte offset)` for the Parley layout query (the A15 char→byte IFC map).
    /// `None` when the position is not inside a textblock.
    fn caret_target(&self, doc: &Node, pos: Pos) -> Option<(NodeHandle, usize)> {
        let r = doc.resolve(pos).ok()?;
        if !r.parent().is_textblock() {
            return None;
        }
        // Walk the descriptor tree to the textblock that owns `pos`, mirroring the
        // resolved path's child indices.
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        let flat_byte = textblock_flat_byte(&desc.node, r.parent_offset());
        Some((desc.dom.clone(), flat_byte))
    }

    /// Map a host caret address — `(textblock element id, flat UTF-8 byte offset)`,
    /// as produced by a pointer hit-test — back to a model [`Pos`] (the inverse of
    /// [`Self::caret_target`]). `None` if `textblock_dom_id` isn't a known
    /// textblock in this view.
    pub(crate) fn pos_at(&self, textblock_dom_id: usize, ifc_byte: usize) -> Option<Pos> {
        let (content_start, block) = find_block(&self.root, textblock_dom_id, 0)?;
        Some(Pos(content_start + ifc_byte_to_char(block, ifc_byte)))
    }

    /// The textblock's offset in container coordinates — the sum of parent-relative
    /// layouts from the block up to (excluding) the container.
    fn block_offset_in_container(&self, d: &dyn DomDocument, block_id: usize) -> (f32, f32) {
        let container_id = self.root.dom.node_id().0;
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut cur = Some(block_id);
        while let Some(id) = cur {
            if id == container_id {
                break;
            }
            if let Some((nx, ny, _, _)) = d.query_node_layout(id as u64) {
                x += nx;
                y += ny;
            }
            cur = d.parent_node(rinch_core::dom::NodeId(id)).map(|n| n.0);
        }
        (x, y)
    }

    /// Caret geometry for an *empty* textblock (which has no Parley layout, so
    /// [`DomDocument::query_caret_position`] returns `None`): the block's content
    /// origin in container space with the block's own box height as the line
    /// height. `None` if the block at `pos` is *not* actually empty (a non-empty
    /// block that merely failed to measure — leave the caret hidden rather than
    /// misplace it) or has no host box yet.
    fn empty_block_caret(
        &self,
        d: &dyn DomDocument,
        doc: &Node,
        pos: Pos,
        block_id: usize,
    ) -> Option<(f32, f32, f32)> {
        let r = doc.resolve(pos).ok()?;
        if r.parent().content().size() != 0 {
            return None;
        }
        let (_, _, _, bh) = d.query_node_layout(block_id as u64)?;
        let (ox, oy) = self.block_offset_in_container(d, block_id);
        // An empty block's box height is its single line box; clamp to a sane range
        // and fall back to a default when it has collapsed to zero.
        let h = if (4.0..=80.0).contains(&bh) { bh } else { 18.0 };
        Some((ox, oy, h))
    }

    /// Take and clear the "an overlay moved" flag. The runtime forces a full repaint
    /// when set, because the caret / selection / node-outline overlays are absolutely
    /// positioned and the software renderer's dirty-region cache can't clear their
    /// *old* rect when they move (otherwise they ghost — see [`Self::overlay_dirty`]).
    pub(crate) fn take_overlay_dirty(&mut self) -> bool {
        std::mem::take(&mut self.overlay_dirty)
    }

    /// Render (or clear) the text-selection highlight from `state.selection`.
    fn render_selection(&mut self, state: &EditorState) {
        let sel = &state.selection;
        if sel.is_empty() {
            self.clear_selection_rects();
            return;
        }
        let key = (sel.from().0, sel.to().0);
        if self.last_selection == Some(key) {
            return;
        }
        self.last_selection = Some(key);
        let Some(doc) = self.doc.upgrade() else {
            return;
        };
        let targets = self.selection_targets(&state.doc, sel.from(), sel.to());
        let rects: Vec<(f32, f32, f32, f32)> = {
            let d = doc.borrow();
            let mut out = Vec::new();
            for (block_id, byte_a, byte_b) in targets {
                let (ox, oy) = self.block_offset_in_container(&*d, block_id);
                for (rx, ry, rw, rh) in d.query_selection_rects(block_id as u64, byte_a, byte_b) {
                    out.push((ox + rx, oy + ry, rw, rh));
                }
            }
            out
        };
        self.set_selection_rects(&rects);
    }

    /// For each textblock overlapping `[from, to)`, the host element id and the
    /// flat UTF-8 byte range of the selection within that block.
    fn selection_targets(&self, doc: &Node, from: Pos, to: Pos) -> Vec<(usize, usize, usize)> {
        let mut targets = Vec::new();
        doc.nodes_between(from.0, to.0, &mut |node, pos, _parent| {
            if node.is_textblock() {
                let content_start = pos + 1;
                let content_end = content_start + node.content().size();
                let a = from.0.max(content_start);
                let b = to.0.min(content_end);
                if a < b
                    && let Some((block_id, _)) = self.caret_address(doc, Pos(content_start))
                {
                    targets.push((
                        block_id,
                        textblock_flat_byte(node, a - content_start),
                        textblock_flat_byte(node, b - content_start),
                    ));
                }
                false // don't descend into the textblock's inline content
            } else {
                true
            }
        });
        targets
    }

    /// Reconcile the selection-highlight divs (container children) to `rects`
    /// (container space).
    fn set_selection_rects(&mut self, rects: &[(f32, f32, f32, f32)]) {
        self.overlay_dirty = true;
        while self.selection_rects.len() < rects.len() {
            let Some(div) = create_element(&self.doc, "div") else {
                break;
            };
            div.set_attribute("data-pm-selection", "true");
            div.set_styles(&[
                ("position", "absolute"),
                ("background-color", "rgba(26,115,232,0.3)"),
                ("pointer-events", "none"),
                // Behind the in-flow text (the container is a stacking context).
                ("z-index", "-1"),
            ]);
            self.root.dom.append_child(&div);
            self.selection_rects.push(div);
        }
        while self.selection_rects.len() > rects.len() {
            if let Some(div) = self.selection_rects.pop() {
                div.remove();
            }
        }
        for (div, (x, y, w, h)) in self.selection_rects.iter().zip(rects) {
            let left = format!("{x}px");
            let top = format!("{y}px");
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("left", &left),
                ("top", &top),
                ("width", &width),
                ("height", &height),
                ("display", "block"),
            ]);
        }
    }

    /// Remove all selection-highlight divs.
    fn clear_selection_rects(&mut self) {
        if self.last_selection.is_none() && self.selection_rects.is_empty() {
            return;
        }
        self.overlay_dirty = true;
        self.last_selection = None;
        for div in self.selection_rects.drain(..) {
            div.remove();
        }
    }

    /// Outline the node a [`Selection::Node`] selects — resolve the node's host
    /// element, compute its box in container space, and position a translucent
    /// overlay over it. Clears the outline if the node can't be located or has no
    /// geometry yet (e.g. an off-screen / virtualized block).
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    fn render_node_selection(&mut self, state: &EditorState) {
        let Some(node_id) = self.node_host_at(&state.doc, state.selection.from()) else {
            self.clear_node_selection();
            return;
        };
        let Some(doc) = self.doc.upgrade() else {
            return;
        };
        let geometry = {
            let d = doc.borrow();
            d.query_node_layout(node_id as u64).map(|(_, _, w, h)| {
                let (ox, oy) = self.block_offset_in_container(&*d, node_id);
                (ox, oy, w, h)
            })
        };
        match geometry {
            Some((x, y, w, h)) => self.position_node_outline(x, y, w, h),
            None => self.clear_node_selection(),
        }
    }

    /// Position the node-selection outline at the container-space box `(x, y, w, h)`.
    /// Reuses one overlay div (created lazily) and skips the writes when the box is
    /// unchanged, so a re-run doesn't re-dirty the tree (mirrors
    /// [`Self::position_caret`]).
    fn position_node_outline(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let key = (
            x.round() as i32,
            y.round() as i32,
            w.round() as i32,
            h.round() as i32,
        );
        if self.last_node_outline == Some(key) {
            return;
        }
        self.overlay_dirty = true;
        self.last_node_outline = Some(key);
        if self.node_outline.is_none() {
            let Some(div) = create_element(&self.doc, "div") else {
                return;
            };
            div.set_attribute("data-pm-selected", "true");
            div.set_styles(&[
                ("position", "absolute"),
                // Trace the node's box exactly (border drawn inside the box).
                ("box-sizing", "border-box"),
                ("border", "2px solid #1a73e8"),
                ("background-color", "rgba(26,115,232,0.15)"),
                ("pointer-events", "none"),
                // In front of the node content, like the caret.
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&div);
            self.node_outline = Some(div);
        }
        if let Some(div) = &self.node_outline {
            let left = format!("{x}px");
            let top = format!("{y}px");
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("left", &left),
                ("top", &top),
                ("width", &width),
                ("height", &height),
                ("display", "block"),
            ]);
        }
    }

    /// Hide the node-selection outline (the selection is no longer a node selection,
    /// or its node went off-screen).
    fn clear_node_selection(&mut self) {
        if self.last_node_outline.is_none() {
            return;
        }
        self.overlay_dirty = true;
        self.last_node_outline = None;
        if let Some(div) = &self.node_outline {
            div.set_style("display", "none");
        }
    }

    /// The host element id of the node *after* `pos` — the node a [`Selection::Node`]
    /// anchored at `pos` selects — navigating the descriptor tree by the resolved
    /// path. `None` if `pos` doesn't sit immediately before a child node.
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    fn node_host_at(&self, doc: &Node, pos: Pos) -> Option<usize> {
        let r = doc.resolve(pos).ok()?;
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        let child = desc.children.get(r.index(r.depth()))?;
        Some(child.outer.node_id().0)
    }

    /// The model position immediately **before** the node whose placed host element
    /// is `target`, plus that node — the inverse of [`Self::node_host_at`], used to
    /// node-select a leaf (image / horizontal rule) the user clicks. Matches either
    /// the node's inner element (`dom`) or its outermost mark wrapper (`outer`), so a
    /// hit on a marked inline image still resolves. `None` if `target` isn't a placed
    /// node in this view.
    pub(crate) fn node_pos_for_host(&self, target: usize) -> Option<(usize, Node)> {
        find_node_by_host(&self.root, target, 0).map(|(pos, node)| (pos, node.clone()))
    }

    /// Position the caret overlay at `(x, y)` in **container space** with `height`.
    /// The caret is a child of the container (created lazily), so it never disrupts
    /// a textblock's inline layout.
    fn position_caret(&mut self, x: f32, y: f32, height: f32) {
        let key = (x.round() as i32, y.round() as i32, height.round() as i32);
        // Nothing changed since last frame — skip the writes so the caret doesn't
        // re-dirty itself and spin the repaint loop.
        if self.last_caret == Some(key) {
            return;
        }
        self.overlay_dirty = true;
        self.last_caret = Some(key);
        // The caret moved (edit/cursor move) — restart the blink phase so the
        // caret is solid immediately after the interaction. Scope the reset to
        // the *focused* (blink-target) editor: with several editors mounted,
        // `update_all_carets` sweeps every one, and a programmatic caret move in
        // an unfocused editor must not stomp the focused editor's global phase.
        // (A focus change resets the phase separately, in `caret_blink_tick`.)
        if super::blink::target() == Some(self.container_id()) {
            super::blink::reset();
        }
        // The `display: block` written below always puts this caret in the
        // "shown" phase, so a moved caret is never left mid-blink (hidden).
        self.blink_shown = Some(true);
        if self.caret.is_none() {
            let Some(caret) = create_element(&self.doc, "div") else {
                return;
            };
            caret.set_attribute("data-pm-caret", "true");
            caret.set_styles(&[
                ("position", "absolute"),
                ("width", "2px"),
                ("background-color", "#1a73e8"),
                ("pointer-events", "none"),
                // In front of the in-flow text and the selection highlight.
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&caret);
            self.caret = Some(caret);
        }
        if let Some(caret) = &self.caret {
            let left = format!("{x}px");
            let top = format!("{y}px");
            let h = format!("{height}px");
            caret.set_styles(&[
                ("left", &left),
                ("top", &top),
                ("height", &h),
                ("display", "block"),
            ]);
        }
    }

    /// Hide all overlays — the caret and the selection highlight. Used when the
    /// editor loses focus, so a blurred editor shows neither (the runtime's
    /// focus-aware caret pass calls this for every non-focused editor).
    pub(crate) fn hide_overlays(&mut self) {
        self.hide_caret();
        self.clear_selection_rects();
        self.clear_node_selection();
    }

    /// Hide the caret (no collapsed cursor in a textblock).
    fn hide_caret(&mut self) {
        if self.last_caret.is_none() {
            return;
        }
        self.overlay_dirty = true;
        self.last_caret = None;
        self.blink_shown = None;
        if let Some(caret) = &self.caret {
            caret.set_style("display", "none");
        }
    }

    /// Apply a blink phase to the caret: show it (`visible == true`) or hide it.
    /// Returns `None` when there is no active caret to blink (no collapsed cursor),
    /// `Some(true)` when the caret's `display` actually toggled (the caller should
    /// repaint), and `Some(false)` when the phase was already applied (no write —
    /// so a blink tick that doesn't change the phase never re-dirties the tree).
    ///
    /// Toggles only `display`; the caret's *position* is owned by
    /// [`Self::position_caret`], which always leaves it in the shown phase.
    pub(crate) fn set_caret_blink_visible(&mut self, visible: bool) -> Option<bool> {
        // No collapsed-cursor caret present → nothing to blink.
        self.last_caret?;
        if self.blink_shown == Some(visible) {
            return Some(false);
        }
        self.blink_shown = Some(visible);
        if let Some(caret) = &self.caret {
            caret.set_style("display", if visible { "block" } else { "none" });
        }
        Some(true)
    }
}

/// The flat UTF-8 byte offset, within a textblock's concatenated inline text,
/// corresponding to the model **char** offset `char_off` — the A15 char→byte half
/// of the caret map. Walks the inline runs accumulating char and byte counts.
///
/// Inline leaves (image / hard_break) are treated as one char of zero flat-byte
/// width for now; exact leaf byte widths in the IFC text stream are tuned against
/// the live renderer (caret-after-leaf is the open edge).
/// Walk the descriptor tree for the block whose host node is `target`, returning
/// its model **content-start** position and its model node. `content_start` is the
/// position passed in for `desc`'s own content (0 for the root/doc).
fn find_block(desc: &ViewDesc, target: usize, content_start: usize) -> Option<(usize, &Node)> {
    let mut pos = content_start; // the position just before the current child
    for child in &desc.children {
        if child.dom.node_id().0 == target {
            // The child's content begins just inside its opening boundary token.
            return Some((pos + 1, &child.node));
        }
        if let Some(found) = find_block(child, target, pos + 1) {
            return Some(found);
        }
        pos += child.node.node_size();
    }
    None
}

/// Walk the descriptor tree for the node whose placed host element is `target`
/// (matching either its inner `dom` or its outermost mark wrapper `outer`),
/// returning its model **position-before** and the node. `content_start` is the
/// position just inside `desc`'s content (0 for the root/doc). The successor lookup
/// to [`find_block`] for *node* (not text-cursor) addressing.
fn find_node_by_host(
    desc: &ViewDesc,
    target: usize,
    content_start: usize,
) -> Option<(usize, &Node)> {
    let mut pos = content_start; // the position just before the current child
    for child in &desc.children {
        if child.dom.node_id().0 == target || child.outer.node_id().0 == target {
            return Some((pos, &child.node));
        }
        if let Some(found) = find_node_by_host(child, target, pos + 1) {
            return Some(found);
        }
        pos += child.node.node_size();
    }
    None
}

/// The model **char** offset within a textblock for a flat UTF-8 `ifc_byte` offset
/// — the inverse of [`textblock_flat_byte`], used to turn a pointer hit into a
/// cursor position. Leaves count as one char (matching `textblock_flat_byte`).
fn ifc_byte_to_char(block: &Node, ifc_byte: usize) -> usize {
    let mut bytes = 0usize;
    let mut chars = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            if ifc_byte <= bytes + text.len() {
                let into_byte = ifc_byte - bytes;
                for (b, _) in text.char_indices() {
                    if b >= into_byte {
                        return chars;
                    }
                    chars += 1;
                }
                return chars;
            }
            bytes += text.len();
            chars += text.chars().count();
        } else {
            chars += 1;
        }
    }
    chars
}

fn textblock_flat_byte(block: &Node, char_off: usize) -> usize {
    let mut chars_seen = 0usize;
    let mut bytes = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            let run_chars = text.chars().count();
            if chars_seen + run_chars >= char_off {
                let into = char_off - chars_seen;
                let byte_in_run = text
                    .char_indices()
                    .nth(into)
                    .map(|(b, _)| b)
                    .unwrap_or(text.len());
                return bytes + byte_in_run;
            }
            chars_seen += run_chars;
            bytes += text.len();
        } else {
            if chars_seen >= char_off {
                return bytes;
            }
            chars_seen += 1; // leaf occupies one model char
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_editor_core::model::Fragment;
    use rinch_editor_core::plugins::PlaceholderPlugin;
    use rinch_editor_core::{EditorState, Schema, Selection, default_plugins};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A host document plus a container node to mount the editor into.
    struct Harness {
        doc: Rc<RefCell<dyn DomDocument>>,
        container: NodeHandle,
        container_id: NodeId,
    }

    fn harness() -> Harness {
        let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        Harness {
            doc,
            container,
            container_id,
        }
    }

    fn doc_ref(h: &Harness) -> DocRef {
        Rc::downgrade(&h.doc)
    }

    fn schema() -> Rc<Schema> {
        Rc::new(Schema::starter_kit())
    }

    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }

    fn mk(s: &Schema, name: &str, attrs: rinch_editor_core::Attrs) -> Mark {
        Mark::new(s.mark_type(name).unwrap().clone(), attrs)
    }

    fn marked_para(s: &Schema, t: &str, marks: Vec<Mark>) -> Node {
        s.branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks(t, marks).unwrap()),
        )
        .unwrap()
    }

    fn doc_node(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }

    fn state(s: Rc<Schema>, doc: Node) -> EditorState {
        EditorState::create(s, doc, default_plugins())
    }

    // Read helpers over the host document.
    fn children(h: &Harness, id: NodeId) -> Vec<NodeId> {
        h.doc.borrow().get_children(id)
    }
    fn tag(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().tag_name(id)
    }
    fn pm_type(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().get_attribute(id, "data-pm-type")
    }
    fn text(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().text_content(id)
    }

    #[test]
    fn builds_initial_dom_from_doc() {
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));
        assert_eq!(pm_type(&h, blocks[0]).as_deref(), Some("paragraph"));
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("hello"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("world"));
        assert_eq!(pm_type(&h, h.container_id).as_deref(), Some("doc"));
    }

    #[test]
    fn blink_toggles_caret_display_and_guards_redundant_writes() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "hi")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // No caret present yet → nothing to blink.
        assert_eq!(view.set_caret_blink_visible(false), None);
        assert_eq!(view.set_caret_blink_visible(true), None);

        // Placing the caret leaves it in the shown phase (and resets the clock).
        view.position_caret(1.0, 2.0, 18.0);
        assert_eq!(view.blink_shown, Some(true));

        // Hiding it for the off-phase writes once...
        assert_eq!(view.set_caret_blink_visible(false), Some(true));
        // ...and a repeated off-phase is a no-op (so a tick never re-dirties).
        assert_eq!(view.set_caret_blink_visible(false), Some(false));
        // Back to the on-phase writes again, then is idempotent.
        assert_eq!(view.set_caret_blink_visible(true), Some(true));
        assert_eq!(view.set_caret_blink_visible(true), Some(false));

        // A re-position always returns the caret to the shown phase, so a moved
        // caret is never left mid-blink (hidden).
        view.set_caret_blink_visible(false);
        view.position_caret(5.0, 2.0, 18.0);
        assert_eq!(view.blink_shown, Some(true));

        // Removing the caret (selection/blur) clears the blink state.
        view.hide_caret();
        assert_eq!(view.blink_shown, None);
        assert_eq!(view.set_caret_blink_visible(true), None);
    }

    #[test]
    fn blink_reset_is_scoped_to_the_focused_editor() {
        // Two editors share one host doc so they get distinct container ids.
        let h = harness();
        let s = schema();
        let container_b_id = h.doc.borrow_mut().create_element("div");
        let container_b = NodeHandle::new(container_b_id, Rc::downgrade(&h.doc));

        let st_a = state(s.clone(), doc_node(&s, vec![para(&s, "aaa")]));
        let st_b = state(s.clone(), doc_node(&s, vec![para(&s, "bbb")]));
        let mut view_a = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st_a);
        let mut view_b = RinchDomEditorView::new(container_b, doc_ref(&h), &st_b);

        // Editor A is the focused / blink-target editor.
        crate::editor::blink::set_target(Some(view_a.container_id()));
        crate::editor::blink::reset();
        let anchor0 = crate::editor::blink::anchor_for_test();

        // A caret move in the UNFOCUSED editor B must NOT re-anchor the clock.
        view_b.position_caret(1.0, 2.0, 18.0);
        assert_eq!(
            crate::editor::blink::anchor_for_test(),
            anchor0,
            "an unfocused editor's caret move stomped the focused editor's blink phase",
        );

        // A caret move in the FOCUSED editor A re-anchors it (caret back to solid).
        view_a.position_caret(1.0, 2.0, 18.0);
        assert_ne!(
            crate::editor::blink::anchor_for_test(),
            anchor0,
            "the focused editor's caret move failed to reset its own blink phase",
        );

        crate::editor::blink::set_target(None);
    }

    #[test]
    fn heading_uses_level_tag() {
        let h = harness();
        let s = schema();
        let heading = s
            .create_node(
                "heading",
                rinch_editor_core::Attrs::from_iter([(
                    "level",
                    rinch_editor_core::AttrValue::from(3_i64),
                )]),
                Fragment::from_node(s.text("Title").unwrap()),
            )
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![heading]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let blocks = children(&h, h.container_id);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("h3"));
    }

    #[test]
    fn insert_text_patches_text_node_in_place() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "ab")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p_id = children(&h, h.container_id)[0];
        let text_id_before = children(&h, p_id)[0];

        // Type "X" at the end of "ab".
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(3)));
        tr.insert_text("X").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        let p_id_after = children(&h, h.container_id)[0];
        let text_id_after = children(&h, p_id_after)[0];
        assert_eq!(p_id, p_id_after, "the paragraph element is reused");
        assert_eq!(
            text_id_before, text_id_after,
            "the text node is patched in place"
        );
        assert_eq!(text(&h, p_id_after).as_deref(), Some("abX"));
    }

    #[test]
    fn split_block_adds_a_paragraph_and_keeps_the_first() {
        let h = harness();
        let s = schema();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "ab")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let first_p_before = children(&h, h.container_id)[0];

        // Split between "a" and "b".
        st.selection = Selection::cursor(rinch_editor_core::Pos(2));
        let next = st.run("splitBlock").expect("split applies");
        view.update_dom(&st, &next);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0], first_p_before,
            "the first paragraph is reused, not rebuilt"
        );
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("a"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("b"));
    }

    #[test]
    fn wrap_in_blockquote_restructures_host() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "quote me")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let next = st.run("wrapInBlockquote").expect("wrap applies");
        view.update_dom(&st, &next);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 1);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("blockquote"));
        let inner = children(&h, blocks[0]);
        assert_eq!(tag(&h, inner[0]).as_deref(), Some("p"));
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("quote me"));
    }

    #[test]
    fn unchanged_sibling_is_not_rebuilt() {
        // Editing the 2nd paragraph must not touch the 1st paragraph's host node
        // (the `same_ref` fast skip, A12).
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(&s, vec![para(&s, "keep"), para(&s, "edit")]),
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let first_before = children(&h, h.container_id)[0];
        let first_text_before = children(&h, first_before)[0];

        // Type into the second paragraph (positions: 0[p 1 keep 5]6[p 7 edit 11]12).
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(11)));
        tr.insert_text("!").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        let first_after = children(&h, h.container_id)[0];
        let first_text_after = children(&h, first_after)[0];
        assert_eq!(first_before, first_after, "1st paragraph element untouched");
        assert_eq!(
            first_text_before, first_text_after,
            "1st paragraph text node untouched"
        );
        assert_eq!(
            text(&h, children(&h, h.container_id)[1]).as_deref(),
            Some("edit!")
        );
    }

    #[test]
    fn placeholder_shows_for_empty_doc_and_hides_after_typing() {
        let h = harness();
        let s = schema();
        let empty = doc_node(&s, vec![s.branch("paragraph", Fragment::empty()).unwrap()]);
        let st = EditorState::create(
            s.clone(),
            empty,
            vec![Rc::new(PlaceholderPlugin::new("Write something…"))],
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // The placeholder overlay exists.
        let has_placeholder = |h: &Harness| {
            children(h, h.container_id).iter().any(|&id| {
                h.doc
                    .borrow()
                    .get_attribute(id, "data-pm-placeholder")
                    .is_some()
            })
        };
        assert!(has_placeholder(&h), "placeholder shown while empty");

        // Type a character — the doc is no longer empty, placeholder disappears.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(1)));
        tr.insert_text("x").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert!(
            !has_placeholder(&h),
            "placeholder removed once content exists"
        );
    }

    // ── inline mark wrappers (M5.5a) ─────────────────────────────────────────

    fn attr_of(h: &Harness, id: NodeId, name: &str) -> Option<String> {
        h.doc.borrow().get_attribute(id, name)
    }

    #[test]
    fn bold_run_wraps_text_in_strong() {
        let h = harness();
        let s = schema();
        let doc = doc_node(
            &s,
            vec![marked_para(
                &s,
                "hi",
                vec![mk(&s, "bold", Default::default())],
            )],
        );
        let st = state(s.clone(), doc);
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
        assert_eq!(attr_of(&h, strong, "data-pm-mark").as_deref(), Some("bold"));
        assert_eq!(text(&h, strong).as_deref(), Some("hi"));
    }

    #[test]
    fn nested_marks_nest_wrappers_innermost_first() {
        // marks = [bold, italic] → bold innermost, italic outermost (matches
        // copy-out HTML `<em><strong>x</strong></em>`).
        let h = harness();
        let s = schema();
        let marks = vec![
            mk(&s, "bold", Default::default()),
            mk(&s, "italic", Default::default()),
        ];
        let st = state(s.clone(), doc_node(&s, vec![marked_para(&s, "x", marks)]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let outer = children(&h, p)[0];
        assert_eq!(tag(&h, outer).as_deref(), Some("em"), "italic is outermost");
        let inner = children(&h, outer)[0];
        assert_eq!(
            tag(&h, inner).as_deref(),
            Some("strong"),
            "bold is innermost"
        );
        assert_eq!(text(&h, inner).as_deref(), Some("x"));
    }

    #[test]
    fn list_items_with_combined_marks_project_wrappers() {
        // bullet_list(li(p[bold "a"]), li(p[italic "b"]), li(p[bold+italic "c"]))
        // → <ul><li><p><strong>a</strong></p>…<li><p><em><strong>c</strong></em>…
        let h = harness();
        let s = schema();
        let li = |t: &str, marks: Vec<Mark>| {
            s.branch("list_item", Fragment::from_node(marked_para(&s, t, marks)))
                .unwrap()
        };
        let list = s
            .branch(
                "bullet_list",
                Fragment::from_children(vec![
                    li("a", vec![mk(&s, "bold", Default::default())]),
                    li("b", vec![mk(&s, "italic", Default::default())]),
                    li(
                        "c",
                        vec![
                            mk(&s, "bold", Default::default()),
                            mk(&s, "italic", Default::default()),
                        ],
                    ),
                ]),
            )
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![list]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let ul = children(&h, h.container_id)[0];
        assert_eq!(tag(&h, ul).as_deref(), Some("ul"));
        let items = children(&h, ul);
        assert_eq!(items.len(), 3, "three list items");

        // The first inline child of an item's <p> (its outermost mark wrapper).
        let item_wrapper = |idx: usize| {
            let p = children(&h, items[idx])[0];
            assert_eq!(
                tag(&h, p).as_deref(),
                Some("p"),
                "list item holds a paragraph"
            );
            children(&h, p)[0]
        };

        let bold = item_wrapper(0);
        assert_eq!(tag(&h, bold).as_deref(), Some("strong"));
        assert_eq!(text(&h, bold).as_deref(), Some("a"));

        let ital = item_wrapper(1);
        assert_eq!(tag(&h, ital).as_deref(), Some("em"));
        assert_eq!(text(&h, ital).as_deref(), Some("b"));

        let outer = item_wrapper(2);
        assert_eq!(tag(&h, outer).as_deref(), Some("em"), "italic outermost");
        let inner = children(&h, outer)[0];
        assert_eq!(tag(&h, inner).as_deref(), Some("strong"), "bold innermost");
        assert_eq!(text(&h, inner).as_deref(), Some("c"));
    }

    #[test]
    fn link_mark_sets_href_on_anchor() {
        let h = harness();
        let s = schema();
        let href = rinch_editor_core::Attrs::from_iter([(
            "href",
            rinch_editor_core::AttrValue::from("https://example.test"),
        )]);
        let st = state(
            s.clone(),
            doc_node(
                &s,
                vec![marked_para(&s, "click", vec![mk(&s, "link", href)])],
            ),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let a = children(&h, p)[0];
        assert_eq!(tag(&h, a).as_deref(), Some("a"));
        assert_eq!(
            attr_of(&h, a, "href").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(text(&h, a).as_deref(), Some("click"));
    }

    #[test]
    fn toggling_bold_over_a_range_rebuilds_the_run_with_a_wrapper() {
        // A mark-set change can't be patched in place — the run rebuilds, wrapping
        // the text in <strong>.
        let h = harness();
        let mut st = state(schema(), doc_node(&schema(), vec![para(&schema(), "abcd")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        assert_eq!(
            tag(&h, children(&h, p)[0]),
            None,
            "plain text node, no wrapper"
        );

        // Select the whole word and bold it.
        st.selection = Selection::text(rinch_editor_core::Pos(1), rinch_editor_core::Pos(5));
        let next = st.run("toggleBold").expect("toggleBold applies");
        view.update_dom(&st, &next);

        let p_after = children(&h, h.container_id)[0];
        let child = children(&h, p_after)[0];
        assert_eq!(
            tag(&h, child).as_deref(),
            Some("strong"),
            "run now wrapped in <strong>"
        );
        assert_eq!(text(&h, child).as_deref(), Some("abcd"));
    }

    // ── A15: char→flat-byte caret map (the off-by-one-prone part) ─────────────

    // ── node-views: NodeSelection of a leaf (image / horizontal rule) ─────────

    #[test]
    fn node_selection_host_round_trips_for_block_atom() {
        let h = harness();
        let s = schema();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "ab"), hr]));
        let view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // The hr is the 2nd container child; it renders as <hr>.
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(tag(&h, blocks[1]).as_deref(), Some("hr"));
        let hr_id = blocks[1].0;

        // Forward: the position just before the hr (model pos 4) → the hr host id.
        assert_eq!(
            view.node_host_at(&st.doc, rinch_editor_core::Pos(4)),
            Some(hr_id)
        );
        // Inverse: the hr host id → (pos-before, node).
        let (pos, node) = view.node_pos_for_host(hr_id).expect("hr is a placed node");
        assert_eq!(pos, 4, "position immediately before the hr");
        assert_eq!(node.type_name(), "horizontal_rule");

        // A textblock host is not a node-selectable leaf.
        assert!(view.node_pos_for_host(blocks[0].0).is_some()); // the paragraph IS a node…
        // …but its position-before is 0 and it isn't a leaf — selectability is the
        // core's call (`Selection::node_at`), exercised in the handle tests.
    }

    #[test]
    fn node_selection_hides_caret_and_text_highlight() {
        let h = harness();
        let s = schema();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "ab"), hr]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // Seed a caret so there is something for the node-selection pass to clear.
        view.position_caret(1.0, 2.0, 18.0);
        assert!(view.last_caret.is_some());

        // Select the hr and run the post-layout pass.
        st.selection =
            rinch_editor_core::Selection::node_at(&st.doc, rinch_editor_core::Pos(4)).unwrap();
        view.update_caret(&st);

        // A node selection shows neither a caret nor a text highlight (the mock has
        // no geometry, so the outline itself can't be positioned here — but the
        // branch runs without panicking and leaves no stale overlays).
        assert!(
            view.last_caret.is_none(),
            "caret hidden for a node selection"
        );
        assert!(
            view.selection_rects.is_empty(),
            "no text highlight for a node selection"
        );
    }

    #[test]
    fn textblock_flat_byte_spans_runs_and_multibyte() {
        // paragraph(text("ab",[bold]), text("é"), text("cd")) — flat text "abécd",
        // where 'é' is one char but two UTF-8 bytes.
        let s = schema();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text_with_marks("ab", vec![mk(&s, "bold", Default::default())])
                        .unwrap(),
                    s.text("é").unwrap(),
                    s.text("cd").unwrap(),
                ]),
            )
            .unwrap();
        assert_eq!(textblock_flat_byte(&p, 0), 0); // start
        assert_eq!(textblock_flat_byte(&p, 2), 2); // after "ab"
        assert_eq!(textblock_flat_byte(&p, 3), 4); // after "é" — past its 2 bytes
        assert_eq!(textblock_flat_byte(&p, 4), 5); // after "c"
        assert_eq!(textblock_flat_byte(&p, 5), 6); // end
    }
}
