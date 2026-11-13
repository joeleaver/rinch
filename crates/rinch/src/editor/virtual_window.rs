//! Block-level virtualization for mounted editors.
//!
//! Keeps all DOM nodes in the tree but prevents off-screen blocks from doing
//! expensive Parley text measurement by giving them a fixed estimated height
//! in Taffy. Only blocks within the visible part of the editor (plus
//! overscan) are "materialized" — meaning they get full Parley layout and
//! painting.
//!
//! The visible part is read off the editor's **scrollport**: the editor itself
//! when it is a scroll container, otherwise its nearest scrolling ancestor (an
//! editor that grows with its content inside a scrolling pane, #2624). Until
//! then only an editor that scrolled itself was virtualized, and an editor in
//! a scrolling pane laid out, and re-laid out, every block of a long
//! document.
//!
//! Uses Taffy's actual layout positions (from the previous frame) to determine
//! which blocks to materialize before layout runs. No parallel height model.
//!
//! Blocks are tracked by node id, not by index: a document loaded over
//! another, a block the view built again, or a remote edit far from the view
//! hands the window new nodes, which [`CeVirtualWindow::sync`] collapses unless
//! they are in the materialized range — whether or not the number of blocks
//! changed.
//!
//! Driven by [`super::virtualization`] (the per-editor two-phase hooks). The
//! window models only the container's `data-pm-type` block children, excluding
//! caret / selection / outline / placeholder overlay siblings.

use std::collections::{HashMap, HashSet};

use rinch_dom::RinchDocument;

/// Default estimated height for never-measured blocks (pixels).
const DEFAULT_ESTIMATED_HEIGHT: f32 = 24.0;

/// Number of extra blocks above/below viewport to keep materialized.
const OVERSCAN: usize = 10;

/// The window stays where it is while it still has at least this many laid
/// out blocks beyond each visible edge (or reaches the document's end), and
/// is re-centred, with [`OVERSCAN`] each side, only once it has not. Moving
/// it is a layout of the whole editor; following every block scrolled past
/// made nearly every frame of a scroll one.
const KEEP: usize = 3;

/// Minimum number of blocks to require before enabling virtualization.
const MIN_BLOCKS_FOR_VIRTUALIZATION: usize = 60;

/// Number of initial blocks to materialize on load.
const INITIAL_MATERIALIZED: usize = 40;

/// Tracks which blocks of a contenteditable are materialized (full layout)
/// vs collapsed (estimated height, no Parley work).
pub(crate) struct CeVirtualWindow {
    /// CE root node ID.
    ce_node_id: u64,

    /// The scrollport whose visible rect decides what is materialized: the
    /// editor itself, or its nearest scrolling ancestor.
    scroller: u64,

    /// The block children as last seen, in order.
    blocks: Vec<u64>,

    /// Measured heights by block node id, so a collapsed block keeps its real
    /// height wherever it moves.
    heights: HashMap<u64, f32>,

    /// The average of `heights`, the estimate for a block never measured.
    average: Option<f32>,

    /// Blocks laid out outside `mat_start..mat_end`: protected when they left
    /// the range, or laid out by [`Self::materialize`]. Collapsed again once
    /// they are neither protected nor in the range.
    extras: Vec<u64>,

    /// Start index of the materialized range (inclusive).
    mat_start: usize,

    /// End index of the materialized range (exclusive).
    mat_end: usize,

    /// Whether virtualization is active (enough blocks to justify it).
    active: bool,

    /// True after the first layout pass has cached heights.
    initialized: bool,

    /// When true, the window models only the container's `data-pm-type` children
    /// (the editor's block elements), excluding overlay siblings (caret /
    /// selection / node-outline / placeholder). Editors always pass `true`; the
    /// `false` path (every direct child is a block) is retained for tests.
    filter_blocks: bool,
}

impl CeVirtualWindow {
    /// Create a window over `ce_node_id`'s blocks, following `scroller`'s
    /// visible rect, and collapse the blocks outside the initial range.
    ///
    /// When `filter_blocks` is true the window models only the container's
    /// `data-pm-type` children (editor blocks), so overlay siblings are never
    /// collapsed or counted.
    pub fn new_filtered(
        ce_node_id: u64,
        scroller: u64,
        doc: &mut RinchDocument,
        filter_blocks: bool,
    ) -> Self {
        let mut vw = CeVirtualWindow {
            ce_node_id,
            scroller,
            blocks: Vec::new(),
            heights: HashMap::new(),
            average: None,
            extras: Vec::new(),
            mat_start: 0,
            mat_end: 0,
            active: false,
            initialized: false,
            filter_blocks,
        };
        vw.sync(doc, &[]);
        vw
    }

    /// Follow another scrollport (the editor moved, or a style change made a
    /// different box the one that scrolls).
    pub fn set_scroller(&mut self, scroller: u64) {
        self.scroller = scroller;
    }

    /// Bring the window up to date with the container's block children, every
    /// frame: every block outside the materialized range that is laid out is
    /// collapsed, and every one inside it laid out, so a new element — a
    /// document loaded over another, whatever its length, or a block the view
    /// built again — is never left laid out far from the view. This is a sweep
    /// of one field per block, not a comparison of ids, so an id a freed node
    /// slot gave back cannot slip past it (#2624's review). A block in
    /// `protected_blocks` outside the range is left laid out and listed in
    /// `extras`, to be collapsed once it is not protected.
    ///
    /// Activates the window at [`MIN_BLOCKS_FOR_VIRTUALIZATION`] blocks (with
    /// the first [`INITIAL_MATERIALIZED`] laid out) and gives every block its
    /// layout back below it. Returns whether the list of blocks changed.
    pub fn sync(&mut self, doc: &mut RinchDocument, protected_blocks: &[u64]) -> bool {
        let children = block_children_of(doc, self.ce_node_id, self.filter_blocks);
        let changed = children != self.blocks;
        if changed {
            self.blocks = children;
            let present: HashSet<u64> = self.blocks.iter().copied().collect();
            self.heights.retain(|id, _| present.contains(id));
            self.extras.retain(|id| present.contains(id));
        }
        let n = self.blocks.len();

        if n < MIN_BLOCKS_FOR_VIRTUALIZATION {
            for &id in &self.blocks {
                set_collapsed(doc, id, None);
            }
            self.active = false;
            self.extras.clear();
            self.mat_start = 0;
            self.mat_end = n;
            return changed;
        }

        if !self.active {
            self.active = true;
            self.mat_start = 0;
            self.mat_end = INITIAL_MATERIALIZED.min(n);
        } else {
            self.mat_end = self.mat_end.min(n);
            self.mat_start = self.mat_start.min(self.mat_end);
        }

        for i in 0..n {
            let id = self.blocks[i];
            if i >= self.mat_start && i < self.mat_end {
                set_collapsed(doc, id, None);
                continue;
            }
            if doc.tree.nodes[id].estimated_height.is_some() {
                continue;
            }
            if protected_blocks.contains(&id) {
                if !self.extras.contains(&id) {
                    self.extras.push(id);
                }
                continue;
            }
            if self.extras.contains(&id) {
                continue;
            }
            self.remember_height(doc, id);
            let est = self.estimate(id);
            set_collapsed(doc, id, Some(est));
        }
        changed
    }

    /// Give every block its layout back: the window is going (the editor has
    /// no scrollport any more).
    pub fn release(self, doc: &mut RinchDocument) {
        for id in block_children_of(doc, self.ce_node_id, self.filter_blocks) {
            set_collapsed(doc, id, None);
        }
    }

    /// Pre-layout phase: use previous frame's layout positions to determine
    /// which blocks should be materialized, then materialize/collapse them
    /// so the upcoming layout pass includes the correct set.
    ///
    /// `protected_blocks` are blocks that must not be collapsed (e.g. the block
    /// holding the caret).
    pub fn pre_layout_update(&mut self, doc: &mut RinchDocument, protected_blocks: &[u64]) -> bool {
        if !self.active || !self.initialized {
            return false;
        }

        let block_count = self.blocks.len();
        if block_count == 0 {
            return false;
        }

        // Use previous frame's layout positions (still valid) to find visible
        // range. The ids are held across frames, so they are looked up, not
        // indexed (#723): a container that is gone reports nothing to do.
        let Some((view_top, view_h)) = self.view_in_editor(doc) else {
            return false;
        };
        if self
            .blocks
            .iter()
            .any(|&id| doc.tree.nodes.get(id).is_none())
        {
            return false;
        }

        let (visible_start, visible_end) =
            Self::find_visible_range(doc, &self.blocks, view_top, view_h);

        let new_start = visible_start.saturating_sub(OVERSCAN);
        let new_end = (visible_end + OVERSCAN).min(block_count);

        // Blocks laid out outside the range that no longer need to be.
        let mut changed = false;
        let extras = std::mem::take(&mut self.extras);
        for id in extras {
            let Some(i) = self.blocks.iter().position(|&b| b == id) else {
                continue;
            };
            if i >= new_start && i < new_end {
                continue; // in the range now: laid out like any other
            }
            if protected_blocks.contains(&id) {
                self.extras.push(id);
                continue;
            }
            if doc.tree.nodes[id].estimated_height.is_none() {
                self.remember_height(doc, id);
                let est = self.estimate(id);
                set_collapsed(doc, id, Some(est));
                changed = true;
            }
        }

        let covered = visible_start >= self.mat_start
            && visible_end <= self.mat_end
            && (self.mat_start == 0 || visible_start >= self.mat_start + KEEP)
            && (self.mat_end >= block_count || visible_end + KEEP <= self.mat_end);
        if covered || (new_start == self.mat_start && new_end == self.mat_end) {
            return changed;
        }

        let old_start = self.mat_start;
        let old_end = self.mat_end.min(block_count);

        // Collapse blocks leaving the range (but never the cursor's block)
        for abs_i in old_start..old_end {
            if abs_i >= new_start && abs_i < new_end {
                continue;
            }
            let id = self.blocks[abs_i];
            // Never collapse protected blocks (cursor block + pending
            // materializations); they stay laid out until they are not.
            if protected_blocks.contains(&id) {
                if !self.extras.contains(&id) {
                    self.extras.push(id);
                }
                continue;
            }
            if doc.tree.nodes[id].estimated_height.is_none() {
                self.remember_height(doc, id);
                let est = self.estimate(id);
                set_collapsed(doc, id, Some(est));
            }
        }

        // Materialize blocks entering the range
        for abs_i in new_start..new_end {
            if abs_i >= old_start && abs_i < old_end {
                continue;
            }
            set_collapsed(doc, self.blocks[abs_i], None);
        }

        self.mat_start = new_start;
        self.mat_end = new_end;
        true
    }

    /// Lay out `block` (a direct child of the editor) now, wherever it is: if it
    /// is collapsed to its estimated height, give it back its real layout.
    /// Returns whether it was collapsed. The window's range is not moved, so
    /// the block stays laid out while the caller keeps it among
    /// `protected_blocks`, and until the range reaches it otherwise.
    ///
    /// For `EditorHandle::scroll_into_view`, which needs the geometry of a
    /// block the user has not scrolled to — the one thing a window driven by
    /// the scroll position cannot supply by itself (#845's shape).
    pub fn materialize(&mut self, doc: &mut RinchDocument, block: u64) -> bool {
        if !self.active || !self.initialized {
            return false;
        }
        if doc
            .tree
            .nodes
            .get(block)
            .is_none_or(|n| n.estimated_height.is_none())
        {
            return false;
        }
        set_collapsed(doc, block, None);
        if let Some(i) = self.blocks.iter().position(|&b| b == block)
            && (i < self.mat_start || i >= self.mat_end)
            && !self.extras.contains(&block)
        {
            self.extras.push(block);
        }
        true
    }

    /// Post-layout phase: cache measured heights from Taffy.
    pub fn post_layout_cache(&mut self, doc: &mut RinchDocument) {
        if !self.active {
            return;
        }
        let end = self.mat_end.min(self.blocks.len());
        let start = self.mat_start.min(end);
        for i in start..end {
            let id = self.blocks[i];
            self.remember_height(doc, id);
        }
        for i in 0..self.extras.len() {
            let id = self.extras[i];
            self.remember_height(doc, id);
        }
        if !self.heights.is_empty() {
            self.average = Some(self.heights.values().sum::<f32>() / self.heights.len() as f32);
        }

        if !self.initialized {
            self.initialized = true;
            // Update unmeasured collapsed blocks with average height
            self.update_unmeasured_estimates(doc);
        }
    }

    /// Record `id`'s laid-out height, if it is laid out (not collapsed) and
    /// has one.
    fn remember_height(&mut self, doc: &RinchDocument, id: u64) {
        let Some(node) = doc.tree.nodes.get(id) else {
            return;
        };
        if node.estimated_height.is_none() && node.layout.height > 0.0 {
            self.heights.insert(id, node.layout.height);
        }
    }

    /// The height a collapsed `id` stands at: its own when it was measured,
    /// else the average, else [`DEFAULT_ESTIMATED_HEIGHT`].
    fn estimate(&self, id: u64) -> f32 {
        self.heights
            .get(&id)
            .copied()
            .or(self.average)
            .unwrap_or(DEFAULT_ESTIMATED_HEIGHT)
    }

    /// The scrollport's visible rect in the editor's coordinates (the
    /// coordinates its blocks' `layout.y` are in): `(top, height)`. `None`
    /// when the scrollport or the editor is gone, the scrollport has no
    /// height yet, or it is not an ancestor of the editor.
    fn view_in_editor(&self, doc: &RinchDocument) -> Option<(f32, f32)> {
        let sc = doc.tree.nodes.get(self.scroller)?;
        let viewport_h = sc.layout.height;
        if viewport_h <= 0.0 {
            return None;
        }
        let mut top = sc.scroll_offset.1 as f32;
        // The editor's offset in the scrollport's content: every box between
        // them is positioned relative to its parent, and none of them scrolls
        // (the scrollport is the nearest one that does).
        let mut id = self.ce_node_id;
        while id != self.scroller {
            let node = doc.tree.nodes.get(id)?;
            top -= node.layout.y;
            id = node.parent?;
        }
        Some((top, viewport_h))
    }

    /// Find visible block range using Taffy layout.y positions.
    fn find_visible_range(
        doc: &RinchDocument,
        children: &[u64],
        view_top: f32,
        view_h: f32,
    ) -> (usize, usize) {
        let view_end = view_top + view_h;

        let visible_start = children
            .partition_point(|&id| {
                let n = &doc.tree.nodes[id];
                n.layout.y + n.layout.height <= view_top
            })
            .min(children.len().saturating_sub(1));

        let visible_end = children
            .partition_point(|&id| doc.tree.nodes[id].layout.y < view_end)
            .min(children.len());

        (visible_start, visible_end.max(visible_start))
    }

    /// Update unmeasured collapsed blocks with average measured height.
    fn update_unmeasured_estimates(&mut self, doc: &mut RinchDocument) {
        let Some(avg) = self.average else {
            return;
        };
        if (avg - DEFAULT_ESTIMATED_HEIGHT).abs() < 2.0 {
            return;
        }
        for i in 0..self.blocks.len() {
            let id = self.blocks[i];
            if let Some(est) = doc.tree.nodes[id].estimated_height
                && !self.heights.contains_key(&id)
                && (est - DEFAULT_ESTIMATED_HEIGHT).abs() < 1.0
            {
                set_collapsed(doc, id, Some(avg));
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// Collapse `id` to `height`, or (`None`) give it back its layout
/// (`RinchDocument::set_estimated_height`); nothing when it is already so.
fn set_collapsed(doc: &mut RinchDocument, id: u64, height: Option<f32>) {
    doc.set_estimated_height(id, height);
}

/// The block children of `ce_node_id`: every direct child when `filter_blocks` is
/// false, or only the `data-pm-type` children when true (the editor, where caret /
/// selection / outline / placeholder overlays are container siblings with no
/// `data-pm-type`).
fn block_children_of(doc: &RinchDocument, ce_node_id: u64, filter_blocks: bool) -> Vec<u64> {
    let Some(ce) = doc.tree.nodes.get(ce_node_id) else {
        return Vec::new();
    };
    let children = &ce.children;
    if !filter_blocks {
        return children.clone();
    }
    children
        .iter()
        .copied()
        .filter(|&id| {
            doc.tree
                .nodes
                .get(id)
                .is_some_and(|n| n.attributes.contains_key("data-pm-type"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::DomDocument;

    /// Build a new-editor-like host: a `data-pm-editor` scroll container with `n`
    /// `<p data-pm-type="paragraph">` blocks (each wrapping to ~2 lines in a narrow
    /// column) plus a `data-pm-caret` overlay sibling that has **no** `data-pm-type`
    /// (so the block filter must exclude it). Returns `(container, block_ids, caret)`.
    fn build_editor_host(doc: &mut RinchDocument, n: usize) -> (u64, Vec<u64>, u64) {
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(container, "data-pm-editor", "true");
        doc.set_attribute(container, "data-pm-type", "doc");
        doc.set_attribute(
            container,
            "style",
            "width: 150px; height: 300px; overflow-y: auto; position: relative",
        );
        doc.append_child(body, container);

        let mut blocks = Vec::new();
        for i in 0..n {
            let p = doc.create_element("p");
            doc.set_attribute(p, "data-pm-type", "paragraph");
            doc.append_child(container, p);
            let text = doc.create_text(&format!(
                "Block {i} with plenty of words here so it wraps across at least two lines in a narrow column"
            ));
            doc.append_child(p, text);
            blocks.push(p.0);
        }

        // A caret overlay (container child, no data-pm-type) — like the live tree
        // after focus. The block filter must never collapse it.
        let caret = doc.create_element("div");
        doc.set_attribute(caret, "data-pm-caret", "true");
        doc.set_attribute(
            caret,
            "style",
            "position: absolute; left: 0; top: 0; width: 2px; height: 18px",
        );
        doc.append_child(container, caret);

        (container.0, blocks, caret.0)
    }

    #[test]
    fn block_filter_excludes_overlay_siblings() {
        let mut doc = RinchDocument::new();
        let (container, blocks, caret) = build_editor_host(&mut doc, 5);
        let filtered = block_children_of(&doc, container, true);
        assert_eq!(filtered, blocks, "only data-pm-type blocks are modeled");
        assert!(!filtered.contains(&caret), "the caret overlay is excluded");
        // Without filtering (contenteditable), every child is a block.
        assert_eq!(
            block_children_of(&doc, container, false).len(),
            blocks.len() + 1
        );
    }

    #[test]
    fn new_filtered_collapses_far_offscreen_block_and_spares_overlay() {
        let mut doc = RinchDocument::new();
        let (container, blocks, caret) = build_editor_host(&mut doc, 80);
        doc.resolve_layout(400.0, 600.0);

        let probe = blocks[50];
        let full_h = doc.tree.nodes[probe].layout.height;
        assert!(
            full_h > 30.0,
            "expected a multi-line block (>30px), got {full_h}"
        );
        let caret_h = doc.tree.nodes[caret].layout.height;

        // This is exactly what the driver does on first run for a scroll editor.
        let vw = CeVirtualWindow::new_filtered(container, container, &mut doc, true);
        assert!(vw.is_active(), "80 blocks must activate virtualization");
        doc.resolve_layout(400.0, 600.0);

        // Collapsed at the height it was measured at, so nothing below it moves.
        assert_eq!(
            doc.tree.nodes[probe].estimated_height,
            Some(full_h),
            "off-screen block 50 should collapse to its measured height"
        );
        assert!((doc.tree.nodes[probe].layout.height - full_h).abs() < 0.5);
        assert!(
            doc.tree.nodes[blocks[79]].estimated_height.is_some(),
            "the last block is collapsed"
        );
        assert_eq!(
            doc.tree.nodes[blocks[0]].estimated_height, None,
            "the first is laid out"
        );
        // The overlay must be untouched by virtualization.
        assert_eq!(
            doc.tree.nodes[caret].estimated_height, None,
            "the caret overlay must not get an estimated height"
        );
        assert!((doc.tree.nodes[caret].layout.height - caret_h).abs() < 0.5);
    }

    /// An editor that does not scroll, inside a scroller that does: the
    /// window reads the scroller's visible rect in the editor's coordinates,
    /// less the editor's offset in the scroller's content (#2624).
    #[test]
    fn view_in_editor_follows_an_ancestor_scroller() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "width: 150px; height: 300px; overflow-y: auto",
        );
        doc.append_child(body, scroller);
        let header = doc.create_element("div");
        doc.set_attribute(header, "style", "height: 100px");
        doc.append_child(scroller, header);
        let editor = doc.create_element("div");
        doc.set_attribute(editor, "data-pm-editor", "true");
        doc.set_attribute(editor, "style", "width: 150px; position: relative");
        doc.append_child(scroller, editor);
        let mut blocks = Vec::new();
        for i in 0..80 {
            let p = doc.create_element("p");
            doc.set_attribute(p, "data-pm-type", "paragraph");
            doc.append_child(editor, p);
            let text = doc.create_text(&format!(
                "Block {i} with plenty of words here so it wraps across at least two lines"
            ));
            doc.append_child(p, text);
            blocks.push(p.0);
        }
        let container = editor.0;
        doc.resolve_layout(400.0, 600.0);
        let mut vw = CeVirtualWindow::new_filtered(container, scroller.0, &mut doc, true);
        doc.resolve_layout(400.0, 600.0);
        vw.post_layout_cache(&mut doc);

        // Below the header, and the first paragraph's top margin, which
        // collapses through the editor's top.
        let off = doc.tree.nodes[container].layout.y;
        assert!(
            off >= 100.0,
            "control: the editor is below the header ({off})"
        );
        assert_eq!(vw.view_in_editor(&doc), Some((-off, 300.0)), "at the top");
        doc.tree.nodes[scroller.0].scroll_offset.1 = 4500.0;
        assert_eq!(
            vw.view_in_editor(&doc),
            Some((4500.0 - off, 300.0)),
            "scrolled"
        );

        // The window moves to the blocks the scroller shows (past the first
        // 40, which it lays out to begin with).
        assert!(vw.pre_layout_update(&mut doc, &[]));
        doc.resolve_layout(400.0, 600.0);
        let shown = blocks
            .iter()
            .position(|&b| {
                let n = &doc.tree.nodes[b];
                n.layout.y + n.layout.height > 4500.0 - off
            })
            .expect("a block reaches the scroller's view");
        assert_eq!(
            doc.tree.nodes[blocks[shown]].estimated_height, None,
            "laid out"
        );
        assert!(
            doc.tree.nodes[blocks[0]].estimated_height.is_some(),
            "the top collapsed"
        );
    }
}
