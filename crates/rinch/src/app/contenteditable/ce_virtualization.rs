//! Block-level virtualization for contenteditable elements.
//!
//! Keeps all DOM nodes in the tree but prevents off-screen blocks from doing
//! expensive Parley text measurement by giving them a fixed estimated height
//! in Taffy. Only blocks within the visible viewport (plus overscan) are
//! "materialized" — meaning they get full Parley layout and painting.
//!
//! Uses Taffy's actual layout positions (from the previous frame) to determine
//! which blocks to materialize before layout runs. No parallel height model.

use rinch_dom::RinchDocument;

/// Default estimated height for never-measured blocks (pixels).
const DEFAULT_ESTIMATED_HEIGHT: f32 = 24.0;

/// Number of extra blocks above/below viewport to keep materialized.
const OVERSCAN: usize = 10;

/// Minimum number of blocks to require before enabling virtualization.
const MIN_BLOCKS_FOR_VIRTUALIZATION: usize = 60;

/// Number of initial blocks to materialize on load.
const INITIAL_MATERIALIZED: usize = 40;

/// Tracks which blocks of a contenteditable are materialized (full layout)
/// vs collapsed (estimated height, no Parley work).
pub(crate) struct CeVirtualWindow {
    /// CE root node ID.
    ce_node_id: usize,

    /// Cached measured heights for blocks, indexed by child position.
    /// Used when collapsing a block so it keeps its real height.
    measured_heights: Vec<Option<f32>>,

    /// Start index of the materialized range (inclusive).
    mat_start: usize,

    /// End index of the materialized range (exclusive).
    mat_end: usize,

    /// Whether virtualization is active (enough blocks to justify it).
    active: bool,

    /// True after the first layout pass has cached heights.
    initialized: bool,

    /// Block IDs that were just materialized by `materialize_for_navigation`
    /// and must not be collapsed by the next `pre_layout_update`.
    /// Cleared after each `pre_layout_update`.
    pub pending_nav_blocks: Vec<usize>,
}

impl CeVirtualWindow {
    /// Create a new VirtualWindow and collapse blocks outside the initial range.
    pub fn new(ce_node_id: usize, doc: &mut RinchDocument) -> Self {
        let children: Vec<usize> = doc.tree.nodes[ce_node_id].children.clone();
        let block_count = children.len();

        let active = block_count >= MIN_BLOCKS_FOR_VIRTUALIZATION;

        let mat_end = if active {
            INITIAL_MATERIALIZED.min(block_count)
        } else {
            block_count
        };

        let vw = CeVirtualWindow {
            ce_node_id,
            measured_heights: vec![None; block_count],
            mat_start: 0,
            mat_end,
            active,
            initialized: false,
            pending_nav_blocks: Vec::new(),
        };

        if active {
            for &child_id in &children[mat_end..] {
                doc.tree.nodes[child_id].estimated_height = Some(DEFAULT_ESTIMATED_HEIGHT);
                doc.tree.style_dirty_nodes.push(child_id);
            }
            doc.tree.layout_dirty = true;
            doc.tree.ifc_dirty = true;
            doc.tree.styles_dirty = true;
        }

        vw
    }

    /// Pre-layout phase: use previous frame's layout positions to determine
    /// which blocks should be materialized, then materialize/collapse them
    /// so the upcoming layout pass includes the correct set.
    ///
    /// `protected_blocks` are blocks that must not be collapsed (cursor block
    /// + any blocks just materialized by materialize_for_navigation).
    pub fn pre_layout_update(
        &mut self,
        doc: &mut RinchDocument,
        protected_blocks: &[usize],
    ) -> bool {
        if !self.active || !self.initialized {
            return false;
        }

        let children: Vec<usize> = doc.tree.nodes[self.ce_node_id].children.clone();
        let block_count = children.len();
        if block_count == 0 {
            return false;
        }

        if self.measured_heights.len() != block_count {
            self.measured_heights.resize(block_count, None);
        }

        // Use previous frame's layout positions (still valid) to find visible range
        let scroll_y = doc.tree.nodes[self.ce_node_id].scroll_offset.1 as f32;
        let viewport_h = doc.tree.nodes[self.ce_node_id].layout.height;
        if viewport_h <= 0.0 {
            return false;
        }

        let (visible_start, visible_end) =
            Self::find_visible_range(doc, &children, scroll_y, viewport_h);

        let new_start = visible_start.saturating_sub(OVERSCAN);
        let new_end = (visible_end + OVERSCAN).min(block_count);

        if new_start == self.mat_start && new_end == self.mat_end {
            return false;
        }

        let old_start = self.mat_start;
        let old_end = self.mat_end;

        // Collapse blocks leaving the range (but never the cursor's block)
        for (i, &child_id) in children[old_start..old_end].iter().enumerate() {
            let abs_i = old_start + i;
            if abs_i < new_start || abs_i >= new_end {
                // Never collapse protected blocks (cursor block + pending materializations)
                if protected_blocks.contains(&child_id) {
                    continue;
                }
                let h = doc.tree.nodes[child_id].layout.height;
                if h > 0.0 {
                    self.measured_heights[abs_i] = Some(h);
                }
                let est_h = self.measured_heights[abs_i].unwrap_or(DEFAULT_ESTIMATED_HEIGHT);
                doc.tree.nodes[child_id].estimated_height = Some(est_h);
                doc.tree.style_dirty_nodes.push(child_id);
            }
        }

        // Materialize blocks entering the range
        for (i, &child_id) in children[new_start..new_end].iter().enumerate() {
            let abs_i = new_start + i;
            if abs_i < old_start || abs_i >= old_end {
                if doc.tree.nodes[child_id].estimated_height.is_some() {
                    doc.tree.nodes[child_id].estimated_height = None;
                    doc.tree.style_dirty_nodes.push(child_id);
                    doc.tree.dirty_ifc_text_roots.insert(child_id);
                }
            }
        }

        self.mat_start = new_start;
        self.mat_end = new_end;

        doc.tree.layout_dirty = true;
        doc.tree.ifc_dirty = true;
        doc.tree.styles_dirty = true;

        true
    }

    /// Post-layout phase: cache measured heights from Taffy.
    pub fn post_layout_cache(&mut self, doc: &mut RinchDocument) {
        if !self.active {
            return;
        }

        let children: Vec<usize> = doc.tree.nodes[self.ce_node_id].children.clone();
        if self.measured_heights.len() != children.len() {
            self.measured_heights.resize(children.len(), None);
        }

        let end = self.mat_end.min(children.len());
        for (i, &child_id) in children[self.mat_start..end].iter().enumerate() {
            let h = doc.tree.nodes[child_id].layout.height;
            if h > 0.0 {
                self.measured_heights[self.mat_start + i] = Some(h);
            }
        }

        if !self.initialized {
            self.initialized = true;
            // Update unmeasured collapsed blocks with average height
            self.update_unmeasured_estimates(doc, &children);
        }
    }

    /// Find visible block range using Taffy layout.y positions.
    fn find_visible_range(
        doc: &RinchDocument,
        children: &[usize],
        scroll_y: f32,
        viewport_h: f32,
    ) -> (usize, usize) {
        let scroll_end = scroll_y + viewport_h;

        let visible_start = children
            .partition_point(|&id| {
                let n = &doc.tree.nodes[id];
                n.layout.y + n.layout.height <= scroll_y
            })
            .min(children.len().saturating_sub(1));

        let visible_end = children
            .partition_point(|&id| doc.tree.nodes[id].layout.y < scroll_end)
            .min(children.len());

        (visible_start, visible_end)
    }

    /// Update unmeasured collapsed blocks with average measured height.
    fn update_unmeasured_estimates(&mut self, doc: &mut RinchDocument, children: &[usize]) {
        let mut sum = 0.0_f32;
        let mut count = 0u32;
        for h in self.measured_heights.iter().flatten() {
            sum += h;
            count += 1;
        }
        if count == 0 {
            return;
        }
        let avg = sum / count as f32;
        if (avg - DEFAULT_ESTIMATED_HEIGHT).abs() < 2.0 {
            return;
        }

        for (i, &child_id) in children.iter().enumerate() {
            if let Some(est) = doc.tree.nodes[child_id].estimated_height {
                if self.measured_heights[i].is_none()
                    && (est - DEFAULT_ESTIMATED_HEIGHT).abs() < 1.0
                {
                    doc.tree.nodes[child_id].estimated_height = Some(avg);
                    doc.tree.style_dirty_nodes.push(child_id);
                }
            }
        }
        doc.tree.layout_dirty = true;
        doc.tree.styles_dirty = true;
    }

    /// Ensure a specific block is materialized.
    #[allow(dead_code)]
    pub fn ensure_materialized(&mut self, doc: &mut RinchDocument, block_node_id: usize) -> bool {
        if !self.active {
            return false;
        }
        let children = &doc.tree.nodes[self.ce_node_id].children;
        let Some(idx) = children.iter().position(|&id| id == block_node_id) else {
            return false;
        };
        if doc.tree.nodes[block_node_id].estimated_height.is_none() {
            return false;
        }

        doc.tree.nodes[block_node_id].estimated_height = None;
        doc.tree.style_dirty_nodes.push(block_node_id);
        doc.tree.dirty_ifc_text_roots.insert(block_node_id);
        doc.tree.layout_dirty = true;
        doc.tree.ifc_dirty = true;
        doc.tree.styles_dirty = true;

        if idx < self.mat_start {
            self.mat_start = idx;
        }
        if idx >= self.mat_end {
            self.mat_end = idx + 1;
        }
        true
    }

    /// Called when blocks are inserted/removed.
    pub fn on_blocks_changed(&mut self, doc: &mut RinchDocument) {
        let children: Vec<usize> = doc.tree.nodes[self.ce_node_id].children.clone();
        let block_count = children.len();

        if block_count < MIN_BLOCKS_FOR_VIRTUALIZATION {
            if self.active {
                for &child_id in &children {
                    if doc.tree.nodes[child_id].estimated_height.is_some() {
                        doc.tree.nodes[child_id].estimated_height = None;
                        doc.tree.style_dirty_nodes.push(child_id);
                    }
                }
                doc.tree.layout_dirty = true;
                doc.tree.ifc_dirty = true;
                doc.tree.styles_dirty = true;
            }
            self.active = false;
            self.measured_heights.resize(block_count, None);
            self.mat_start = 0;
            self.mat_end = block_count;
            return;
        }

        self.active = true;
        self.measured_heights = vec![None; block_count];
        self.mat_end = self.mat_end.min(block_count);

        for (i, &child_id) in children.iter().enumerate() {
            if i >= self.mat_start && i < self.mat_end {
                if doc.tree.nodes[child_id].estimated_height.is_some() {
                    doc.tree.nodes[child_id].estimated_height = None;
                    doc.tree.style_dirty_nodes.push(child_id);
                }
            } else if doc.tree.nodes[child_id].estimated_height.is_none() {
                doc.tree.nodes[child_id].estimated_height = Some(DEFAULT_ESTIMATED_HEIGHT);
                doc.tree.style_dirty_nodes.push(child_id);
            }
        }

        doc.tree.layout_dirty = true;
        doc.tree.ifc_dirty = true;
        doc.tree.styles_dirty = true;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}
