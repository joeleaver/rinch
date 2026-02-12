//! Inline Formatting Context (IFC): Parley-based inline layout for text and inline elements.

use std::collections::HashMap;

use peniko::Brush;

use crate::RinchDocument;
use crate::layout;
use crate::node::{DisplayMode, InlineLayout, LayoutResult, Node, NodeContext, NodeKind};

impl RinchDocument {
    /// Build inline layouts for all IFC roots after Taffy layout.
    ///
    /// Uses the computed width from Taffy as the available width for Parley line breaking.
    pub(crate) fn build_ifc_layouts(&mut self, paint_layout_cx: &mut parley::LayoutContext<Brush>) {
        // Collect IFC roots (elements that have inline children with ifc_root set)
        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() {
                continue;
            }
            if matches!(
                node.display_mode,
                DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex
            ) {
                continue;
            }
            // Check if any child has ifc_root pointing to this node
            let is_ifc = node.children.iter().any(|&child_id| {
                self.tree
                    .nodes
                    .get(child_id)
                    .map(|c| c.ifc_root == Some(id))
                    .unwrap_or(false)
            });
            if is_ifc {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let node = &self.tree.nodes[root_id];
            // Use content-box width (subtract padding+border) for line breaking.
            // For auto-width elements, don't re-constrain text to measured width
            // as floating-point precision can cause unwanted line breaks.
            let max_width = {
                let cs = &node.computed_style;
                let padding_h = cs.padding_left.to_px() + cs.padding_right.to_px();
                let border_h = cs.border_left_width.to_px() + cs.border_right_width.to_px();
                let content_width = node.layout.width - padding_h - border_h;
                if content_width > 0.0 {
                    // For auto-width elements, add a small tolerance to avoid
                    // floating-point precision causing unwanted line breaks.
                    let tolerance = if matches!(
                        cs.width,
                        crate::computed_style::DimensionValue::Auto
                    ) {
                        0.5
                    } else {
                        0.0
                    };
                    Some(content_width + tolerance)
                } else {
                    None
                }
            };

            let inline_layout = Self::build_inline_layout(
                &self.tree.nodes,
                root_id,
                max_width,
                1.0, // scale=1.0 for layout; paint scales later
                &mut self.font_cx,
                paint_layout_cx,
            );

            // Write positions from Parley layout back to child nodes
            // Walk the layout lines to find positioned inline boxes and text runs
            self.write_inline_positions(root_id, &inline_layout);

            self.tree.nodes[root_id].text_layout = Some(Box::new(inline_layout));
        }
    }

    /// Copy cached text layouts from measurement to nodes.
    ///
    /// Uses the exact layouts built during Taffy measurement to ensure
    /// paint uses identical text shaping results.
    pub(crate) fn copy_cached_text_layouts(
        &mut self,
        cache: HashMap<(usize, u32), parley::layout::Layout<Brush>>,
    ) {
        // First collect node IDs and their layouts to apply
        let updates: Vec<(usize, parley::layout::Layout<Brush>)> = self
            .tree
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                if node.ifc_root.is_some() {
                    return None;
                } // Skip IFC-managed nodes
                if !matches!(&node.kind, NodeKind::Text(_)) {
                    return None;
                }

                let width = node.layout.width;
                let wrap_bits = if width > 0.0 {
                    width.to_bits()
                } else {
                    u32::MAX
                };

                // Try to find a cached layout for this node with the final width
                if let Some(layout) = cache.get(&(id, wrap_bits)) {
                    return Some((id, layout.clone()));
                }

                // Fallback: prefer the MaxContent layout (wrap_bits = u32::MAX) which has natural width.
                // This avoids picking a MinContent layout that was wrapped to 0 width.
                if let Some(layout) = cache.get(&(id, u32::MAX)) {
                    return Some((id, layout.clone()));
                }
                // Last resort: find any cached layout for this node (prefer widest/tallest ratio)
                let mut best: Option<&parley::layout::Layout<Brush>> = None;
                for ((nid, _), layout) in &cache {
                    if *nid == id {
                        // Prefer single-line layouts (lower height) over wrapped ones
                        if best.is_none() || layout.height() < best.unwrap().height() {
                            best = Some(layout);
                        }
                    }
                }
                best.map(|layout| (id, layout.clone()))
            })
            .collect();

        // Apply the updates with alignment
        for (id, mut layout) in updates {
            // Get text-align from parent's computed style
            let parent_id = self.tree.nodes[id].parent;
            let alignment = parent_id
                .and_then(|p| self.tree.nodes.get(p))
                .map(|p| p.computed_style.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);

            // Apply alignment before caching
            layout.align(alignment, parley::layout::AlignmentOptions::default());

            self.tree.nodes[id].cached_text_parley = Some(Box::new(layout));
        }
    }

    /// Write computed positions from an InlineLayout back into child node layout fields.
    pub(crate) fn write_inline_positions(&mut self, root_id: usize, inline_layout: &InlineLayout) {
        let root_layout = self.tree.nodes[root_id].layout;

        // Walk Parley layout lines to find positioned items
        for line in inline_layout.layout.lines() {
            for item in line.items() {
                match item {
                    parley::layout::PositionedLayoutItem::GlyphRun(_) => {
                        // Text runs don't map to individual child nodes
                    }
                    parley::layout::PositionedLayoutItem::InlineBox(positioned_box) => {
                        let child_id = positioned_box.id as usize;
                        if let Some(child) = self.tree.nodes.get_mut(child_id) {
                            child.layout.x = positioned_box.x;
                            child.layout.y = positioned_box.y;
                        }
                    }
                }
            }
        }

        // For text nodes that are direct children, set their layout to reflect
        // the actual IFC content extent (not the constrained container height).
        // This is critical for scroll containers: compute_content_height uses
        // children's layout bounds to determine if content overflows.
        let ifc_content_height = inline_layout.layout.height();
        let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
        for child_id in children {
            if let Some(child) = self.tree.nodes.get(child_id)
                && child.is_text()
                && child.ifc_root == Some(root_id)
            {
                if let Some(child) = self.tree.nodes.get_mut(child_id) {
                    child.layout.x = 0.0;
                    child.layout.y = 0.0;
                    child.layout.width = root_layout.width;
                    child.layout.height = ifc_content_height;
                }
            }
        }
    }

    /// Clean up anonymous block boxes from the previous layout pass.
    ///
    /// Anonymous block boxes wrap runs of inline children in mixed-content
    /// block containers (CSS spec: "anonymous block boxes"). They are
    /// recreated each layout pass to ensure correctness after DOM mutations.
    fn cleanup_anonymous_block_boxes(&mut self) {
        let anon_ids = std::mem::take(&mut self.tree.anonymous_block_boxes);
        if anon_ids.is_empty() {
            return;
        }

        // Track which parents need Taffy child rebuild
        let mut parents_affected: Vec<usize> = Vec::new();

        for &anon_id in &anon_ids {
            let (parent_id, children) = {
                let node = match self.tree.nodes.get(anon_id) {
                    Some(n) => n,
                    None => continue,
                };
                let parent_id = match node.parent {
                    Some(p) => p,
                    None => continue,
                };
                (parent_id, node.children.clone())
            };

            if !parents_affected.contains(&parent_id) {
                parents_affected.push(parent_id);
            }

            // Find position of anonymous box in parent's DOM children
            let pos = self.tree.nodes[parent_id]
                .children
                .iter()
                .position(|&c| c == anon_id)
                .unwrap_or(0);

            // Remove anonymous box from parent's DOM children
            self.tree.nodes[parent_id].children.remove(pos);

            // Insert anonymous box's children back into parent at the same position
            for (i, &child_id) in children.iter().enumerate() {
                self.tree.nodes[parent_id]
                    .children
                    .insert(pos + i, child_id);
                if let Some(child) = self.tree.nodes.get_mut(child_id) {
                    child.parent = Some(parent_id);
                    child.ifc_root = None;
                }
            }

            // Remove anonymous Taffy node (children are detached, not deleted)
            if let Some(anon_taffy) = self.tree.nodes[anon_id].taffy_id {
                self.tree.taffy_map.remove(&anon_taffy);
                let _ = self.tree.taffy.remove(anon_taffy);
            }

            // Remove anonymous DOM node from slab
            self.tree.nodes.remove(anon_id);
        }

        // Rebuild Taffy children for all affected parents from DOM order
        for parent_id in parents_affected {
            if let Some(parent_taffy) = self.tree.nodes.get(parent_id).and_then(|n| n.taffy_id) {
                let dom_children: Vec<usize> = self.tree.nodes[parent_id].children.clone();
                let _ = self.tree.taffy.set_children(parent_taffy, &[]);
                for &child_id in &dom_children {
                    if let Some(child_taffy) =
                        self.tree.nodes.get(child_id).and_then(|n| n.taffy_id)
                    {
                        let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
                    }
                }
            }
        }
    }

    /// Create anonymous block boxes for block containers with mixed content.
    ///
    /// Per CSS spec, when a block container has both inline-level and block-level
    /// children, consecutive runs of inline children are wrapped in anonymous
    /// block boxes. These boxes become IFC roots for text layout.
    fn create_anonymous_block_boxes(&mut self) {
        // Phase 1: Detect mixed-content block containers
        let mut containers: Vec<(usize, Vec<Vec<usize>>)> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if !node.is_element() || node.is_anonymous_block_box {
                continue;
            }
            // Only block containers can have anonymous boxes
            if matches!(
                node.display_mode,
                DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex
            ) {
                continue;
            }

            let has_inline = node.children.iter().any(|&c| {
                self.tree
                    .nodes
                    .get(c)
                    .map(|n| n.is_inline())
                    .unwrap_or(false)
            });
            let has_block = node.children.iter().any(|&c| {
                self.tree
                    .nodes
                    .get(c)
                    .map(|n| n.is_element() && !n.is_inline())
                    .unwrap_or(false)
            });

            if !(has_inline && has_block) {
                continue;
            }

            // Group consecutive inline children into runs
            let mut runs: Vec<Vec<usize>> = Vec::new();
            let mut current_run: Vec<usize> = Vec::new();

            for &child_id in &node.children {
                let is_inline = self
                    .tree
                    .nodes
                    .get(child_id)
                    .map(|c| c.is_inline())
                    .unwrap_or(false);
                if is_inline {
                    current_run.push(child_id);
                } else if !current_run.is_empty() {
                    runs.push(std::mem::take(&mut current_run));
                }
            }
            if !current_run.is_empty() {
                runs.push(current_run);
            }

            if !runs.is_empty() {
                containers.push((id, runs));
            }
        }

        // Phase 2: Create anonymous boxes and reparent inline children
        for (parent_id, runs) in containers {
            let guard = self.tree.guard.clone();

            for run in runs {
                let first_child = run[0];
                // Look up position in CURRENT children list (handles multiple runs correctly)
                let first_pos = self.tree.nodes[parent_id]
                    .children
                    .iter()
                    .position(|&c| c == first_child)
                    .unwrap();

                // Create anonymous block box DOM node
                let anon_id = self.tree.nodes.vacant_key();
                let mut anon_node = Node::element(anon_id, "div", guard.clone());
                anon_node.is_anonymous_block_box = true;
                anon_node.display_mode = DisplayMode::Block;
                anon_node.parent = Some(parent_id);
                anon_node.children = run.clone();
                // Inherit computed style from parent for font properties
                anon_node.computed_style = self.tree.nodes[parent_id].computed_style.clone();

                // Create Taffy node for the anonymous box
                let anon_taffy = self
                    .tree
                    .taffy
                    .new_leaf(taffy::Style {
                        display: taffy::Display::Block,
                        ..Default::default()
                    })
                    .unwrap();
                anon_node.taffy_id = Some(anon_taffy);
                self.tree.taffy_map.insert(anon_taffy, anon_id);

                // Insert anonymous node into slab
                self.tree.nodes.insert(anon_node);

                // Update children's parent references
                for &child_id in &run {
                    if let Some(child) = self.tree.nodes.get_mut(child_id) {
                        child.parent = Some(anon_id);
                    }
                }

                // Replace inline children in parent's DOM children with anonymous box
                self.tree.nodes[parent_id]
                    .children
                    .retain(|c| !run.contains(c));
                let insert_pos = first_pos.min(self.tree.nodes[parent_id].children.len());
                self.tree.nodes[parent_id]
                    .children
                    .insert(insert_pos, anon_id);

                // Track for cleanup on next layout pass
                self.tree.anonymous_block_boxes.push(anon_id);
            }

            // Rebuild Taffy children for the parent and its anonymous boxes
            // from DOM order. This avoids remove_child panics when Taffy
            // children are out of sync with DOM (e.g., after IFC detached text nodes).
            if let Some(parent_taffy) = self.tree.nodes.get(parent_id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.set_children(parent_taffy, &[]);
                let dom_children: Vec<usize> = self.tree.nodes[parent_id].children.clone();
                for &child_id in &dom_children {
                    if let Some(child_taffy) =
                        self.tree.nodes.get(child_id).and_then(|n| n.taffy_id)
                    {
                        let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
                    }
                    // For anonymous boxes, also rebuild their Taffy children
                    if self
                        .tree
                        .nodes
                        .get(child_id)
                        .map(|n| n.is_anonymous_block_box)
                        .unwrap_or(false)
                    {
                        if let Some(anon_taffy) =
                            self.tree.nodes.get(child_id).and_then(|n| n.taffy_id)
                        {
                            let anon_children: Vec<usize> =
                                self.tree.nodes[child_id].children.clone();
                            for &anon_child_id in &anon_children {
                                if let Some(anon_child_taffy) = self
                                    .tree
                                    .nodes
                                    .get(anon_child_id)
                                    .and_then(|n| n.taffy_id)
                                {
                                    let _ = self
                                        .tree
                                        .taffy
                                        .add_child(anon_taffy, anon_child_taffy);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Detect IFC roots and mark inline children.
    ///
    /// An element is an IFC root if it's a block container that has any
    /// inline children (text nodes, inline elements, etc.). Even single
    /// text children use IFC — this avoids maintaining two measurement
    /// paths (standalone Taffy vs IFC) and the sync bugs that arise when
    /// elements transition between them during editing.
    pub(crate) fn setup_inline_formatting_contexts(&mut self) {
        // Clean up anonymous block boxes from previous layout pass,
        // then recreate them for the current DOM state.
        self.cleanup_anonymous_block_boxes();
        self.create_anonymous_block_boxes();

        let mut ifc_roots: Vec<usize> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if !node.is_element() {
                continue;
            }
            // Only block containers can be IFC roots — skip inline, inline-block, and flex
            if matches!(
                node.display_mode,
                DisplayMode::Inline | DisplayMode::InlineBlock | DisplayMode::Flex
            ) {
                continue;
            }

            let inline_children: Vec<usize> = node
                .children
                .iter()
                .filter(|&&child_id| {
                    self.tree
                        .nodes
                        .get(child_id)
                        .map(|c| c.is_inline())
                        .unwrap_or(false)
                })
                .copied()
                .collect();

            // Activate IFC for any block element with inline children.
            // Even a single text child uses IFC — it's a degenerate case with one
            // text range. This avoids needing two measurement paths (standalone vs IFC)
            // and the sync bugs that arise when elements transition between them.
            if !inline_children.is_empty() {
                ifc_roots.push(id);
            }
        }

        for root_id in ifc_roots {
            let children: Vec<usize> = self.tree.nodes[root_id].children.clone();
            let root_taffy = match self.tree.nodes[root_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove inline children from Taffy (Parley will handle their layout)
            for &child_id in &children {
                let child = match self.tree.nodes.get(child_id) {
                    Some(c) => c,
                    None => continue,
                };
                if child.is_inline() {
                    if let Some(child_taffy) = child.taffy_id
                        && let Ok(taffy_children) = self.tree.taffy.children(root_taffy)
                        && taffy_children.contains(&child_taffy)
                    {
                        let _ = self.tree.taffy.remove_child(root_taffy, child_taffy);
                    }
                    if let Some(c) = self.tree.nodes.get_mut(child_id) {
                        c.ifc_root = Some(root_id);
                    }
                }
            }

            // Set NodeContext::InlineRoot on the IFC root's Taffy node
            // so the measure function fires for it
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(root_taffy) {
                *ctx = NodeContext::InlineRoot(root_id);
            } else {
                // Element nodes don't have context by default — we need to set one.
                // Taffy only calls measure for nodes with context, so we must ensure it has one.
                let _ = self
                    .tree
                    .taffy
                    .set_node_context(root_taffy, Some(NodeContext::InlineRoot(root_id)));
            }
        }
    }

    /// Pre-compute layout for inline-block children that were detached from Taffy.
    ///
    /// Inline-block children are removed from their parent's Taffy tree (so the parent
    /// uses InlineRoot measurement), but they still need their own subtree computed
    /// so `walk_inline_children` can read their width/height for Parley InlineBox.
    pub(crate) fn compute_inline_block_layouts(&mut self) {
        // Collect inline-block children that belong to an IFC
        let mut ib_taffy_ids: Vec<taffy::NodeId> = Vec::new();
        for (_id, node) in &self.tree.nodes {
            if node.ifc_root.is_some()
                && node.display_mode == DisplayMode::InlineBlock
                && let Some(taffy_id) = node.taffy_id
            {
                ib_taffy_ids.push(taffy_id);
            }
        }

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;

        for taffy_id in ib_taffy_ids {
            let avail = taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            };
            let _ = self.tree.taffy.compute_layout_with_measure(
                taffy_id,
                avail,
                |known_dims, avail_space, _node_id, context, _style| {
                    let max_width = match avail_space.width {
                        taffy::AvailableSpace::Definite(w) => Some(w),
                        taffy::AvailableSpace::MaxContent => None,
                        taffy::AvailableSpace::MinContent => Some(0.0),
                    };
                    match context {
                        Some(NodeContext::Text(text)) => {
                            if text.content.is_empty() {
                                return taffy::Size::ZERO;
                            }
                            let mut builder =
                                layout_cx.ranged_builder(font_cx, &text.content, 1.0, true);
                            builder.push_default(parley::style::StyleProperty::FontSize(
                                text.font_size,
                            ));
                            if (text.font_weight - 400.0).abs() > 1.0 {
                                builder.push_default(parley::style::StyleProperty::FontWeight(
                                    parley::style::FontWeight::new(text.font_weight),
                                ));
                            }
                            if let Some(lh) =
                                layout::css_line_height_to_parley(&text.line_height_css)
                            {
                                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                            }
                            let font_stack = if !text.font_family.is_empty() {
                                std::borrow::Cow::Owned(text.font_family.clone())
                            } else {
                                std::borrow::Cow::Borrowed("sans-serif")
                            };
                            builder.push_default(parley::style::StyleProperty::FontStack(
                                parley::style::FontStack::Source(font_stack),
                            ));
                            // Apply overflow-wrap for emergency line-breaking
                            builder.push_default(parley::style::StyleProperty::OverflowWrap(
                                text.overflow_wrap.to_parley(),
                            ));
                            let mut layout = builder.build(&text.content);
                            // If no_wrap is set (white-space: nowrap), don't constrain width
                            let wrap_width = if text.no_wrap {
                                None
                            } else {
                                known_dims.width.or(max_width)
                            };
                            layout.break_all_lines(wrap_width);
                            taffy::Size {
                                width: known_dims.width.unwrap_or(layout.width()),
                                height: known_dims.height.unwrap_or(layout.height()),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            );

            // Read the computed layout back into the node
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let node_id = self.tree.taffy_map.get(&taffy_id).copied();
                if let Some(nid) = node_id
                    && let Some(node) = self.tree.nodes.get_mut(nid)
                {
                    node.layout.width = taffy_layout.size.width;
                    node.layout.height = taffy_layout.size.height;
                }
            }
        }
    }

    /// Build a Parley inline layout for an IFC root node.
    ///
    /// Walks the IFC root's children, collecting text nodes and inline elements
    /// into a single Parley TreeBuilder layout. Returns the InlineLayout.
    pub(crate) fn build_inline_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        max_width: Option<f32>,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> InlineLayout {
        // Get root style properties from typed ComputedStyle
        let root_computed = &nodes[root_id].computed_style;
        let root_font_size = root_computed.font_size * scale;
        let root_color = root_computed.color.unwrap_or_else(|| {
            peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(0, 0, 0, 255)
        });

        let font_family: std::borrow::Cow<'static, str> = if root_computed.font_family.is_empty() {
            "sans-serif".into()
        } else {
            root_computed.font_family.clone().into()
        };

        let mut root_text_style = parley::style::TextStyle {
            font_size: root_font_size,
            brush: Brush::Solid(root_color),
            font_stack: parley::style::FontStack::Source(font_family),
            ..Default::default()
        };

        // Apply font-weight from computed style
        root_text_style.font_weight = parley::style::FontWeight::new(root_computed.font_weight);

        // Apply font-style from computed style
        root_text_style.font_style = root_computed.font_style.to_parley();

        // Apply line-height from computed style
        if let Some(lh) = root_computed.line_height.to_parley() {
            root_text_style.line_height = lh;
        }

        // Apply text-decoration from computed style
        root_text_style.has_underline = root_computed.text_decoration.underline;
        root_text_style.has_strikethrough = root_computed.text_decoration.strikethrough;

        // Apply overflow-wrap from computed style
        root_text_style.overflow_wrap = root_computed.overflow_wrap.to_parley();

        let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);

        // Apply white-space mode from computed style.
        // Contenteditable elements always use Preserve (pre-wrap) to prevent
        // Parley from collapsing trailing whitespace, which would cause cursor
        // position mismatches (the DOM text has the space but the layout doesn't).
        use crate::computed_style::WhiteSpaceValue;
        let is_contenteditable = nodes[root_id]
            .attributes
            .contains_key("contenteditable");
        let collapse = if is_contenteditable {
            parley::style::WhiteSpaceCollapse::Preserve
        } else {
            match root_computed.white_space {
                WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap | WhiteSpaceValue::PreLine => {
                    parley::style::WhiteSpaceCollapse::Preserve
                }
                _ => parley::style::WhiteSpaceCollapse::Collapse,
            }
        };
        builder.set_white_space_mode(collapse);

        let mut child_positions = Vec::new();
        let mut text_ranges = Vec::new();
        let mut flat_pos = 0usize;

        // Walk children and build the Parley tree
        Self::walk_inline_children(nodes, root_id, &mut builder, &mut child_positions, &mut text_ranges, &mut flat_pos, scale, collapse);

        let (text_layout, text_content) = builder.build();
        let mut text_layout = text_layout;
        text_layout.break_all_lines(max_width);

        // Apply text-align from computed style
        let alignment = root_computed.text_align.to_parley();
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());

        InlineLayout {
            layout: text_layout,
            text_content,
            child_positions,
            text_ranges,
        }
    }

    /// Recursively walk inline children, pushing text and style spans into the TreeBuilder.
    pub(crate) fn walk_inline_children(
        nodes: &slab::Slab<Node>,
        parent_id: usize,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        child_positions: &mut Vec<(usize, LayoutResult)>,
        text_ranges: &mut Vec<crate::node::IfcTextRange>,
        flat_pos: &mut usize,
        scale: f32,
        collapse: parley::style::WhiteSpaceCollapse,
    ) {
        let children: Vec<usize> = nodes[parent_id].children.clone();
        for child_id in children {
            let child = match nodes.get(child_id) {
                Some(c) => c,
                None => continue,
            };
            match &child.kind {
                NodeKind::Text(text_data) => {
                    if !text_data.content.is_empty() {
                        let start = *flat_pos;
                        builder.push_text(&text_data.content);
                        *flat_pos += text_data.content.len();
                        text_ranges.push(crate::node::IfcTextRange {
                            flat_start: start,
                            flat_end: *flat_pos,
                            node_id: child_id,
                            node_offset: 0,
                            is_br: false,
                        });
                        // Record position placeholder — actual position comes from layout
                        child_positions.push((child_id, LayoutResult::default()));
                    }
                }
                NodeKind::Element(_)
                    if child.display_mode == DisplayMode::Inline
                        && child.tag() == Some("br") =>
                {
                    // <br> elements insert a hard line break.
                    let start = *flat_pos;
                    builder.set_white_space_mode(parley::style::WhiteSpaceCollapse::Preserve);
                    builder.push_text("\n");
                    *flat_pos += 1;
                    let no_props: [parley::style::StyleProperty<'_, Brush>; 0] = [];
                    builder.push_style_modification_span(no_props.iter());
                    builder.pop_style_span();
                    builder.set_white_space_mode(collapse);
                    text_ranges.push(crate::node::IfcTextRange {
                        flat_start: start,
                        flat_end: *flat_pos,
                        node_id: child_id,
                        node_offset: 0,
                        is_br: true,
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::Inline => {
                    // Push style span for inline element using typed ComputedStyle
                    let child_computed = &child.computed_style;
                    let mut props: Vec<parley::style::StyleProperty<'_, Brush>> = Vec::new();

                    // Font size (always apply scaled)
                    props.push(parley::style::StyleProperty::FontSize(
                        child_computed.font_size * scale,
                    ));

                    // Font weight
                    props.push(parley::style::StyleProperty::FontWeight(
                        parley::style::FontWeight::new(child_computed.font_weight),
                    ));

                    // Font style
                    props.push(parley::style::StyleProperty::FontStyle(
                        child_computed.font_style.to_parley(),
                    ));

                    // Color
                    if let Some(color) = child_computed.color {
                        props.push(parley::style::StyleProperty::Brush(Brush::Solid(color)));
                    }

                    // Text decoration
                    if child_computed.text_decoration.underline {
                        props.push(parley::style::StyleProperty::Underline(true));
                    }
                    if child_computed.text_decoration.strikethrough {
                        props.push(parley::style::StyleProperty::Strikethrough(true));
                    }

                    // Line height
                    if let Some(lh) = child_computed.line_height.to_parley() {
                        // Scale absolute line heights
                        let scaled_lh = match lh {
                            parley::style::LineHeight::Absolute(v) => {
                                parley::style::LineHeight::Absolute(v * scale)
                            }
                            other => other,
                        };
                        props.push(parley::style::StyleProperty::LineHeight(scaled_lh));
                    }

                    builder.push_style_modification_span(props.iter());
                    child_positions.push((child_id, LayoutResult::default()));

                    // Recurse into inline element's children
                    Self::walk_inline_children(nodes, child_id, builder, child_positions, text_ranges, flat_pos, scale, collapse);

                    builder.pop_style_span();
                }
                NodeKind::Element(_) if child.display_mode == DisplayMode::InlineBlock => {
                    // Inline-block: measure via Taffy first, then embed as InlineBox
                    let child_layout = &child.layout;
                    builder.push_inline_box(parley::InlineBox {
                        id: child_id as u64,
                        index: 0, // will be set by builder
                        width: child_layout.width * scale,
                        height: child_layout.height * scale,
                        kind: parley::InlineBoxKind::InFlow,
                    });
                    child_positions.push((child_id, LayoutResult::default()));
                }
                NodeKind::Comment(_) => {
                    // Skip comments in inline layout
                }
                _ => {
                    // Block children break inline flow — stop here
                    break;
                }
            }
        }
    }
}
