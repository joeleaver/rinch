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
        // Discover all IFC roots (cheap O(n) walk — just checks a field per node).
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

        // Only rebuild Parley TextLayouts for dirty IFC roots (the expensive part).
        // When dirty_ifc_text_roots is empty, rebuild all (structural IFC change).
        let rebuild_all = self.tree.dirty_ifc_text_roots.is_empty();

        for root_id in ifc_roots {
            // Skip collapsed blocks (virtualized) — no Parley work needed.
            // Drop any existing text_layout to free memory.
            if self.tree.nodes[root_id].estimated_height.is_some() {
                self.tree.nodes[root_id].text_layout = None;
                continue;
            }

            // Skip IFC roots that aren't dirty (scoped rebuild).
            // This turns O(all_roots) Parley work into O(dirty_roots).
            if !rebuild_all && !self.tree.dirty_ifc_text_roots.contains(&root_id) {
                continue;
            }

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
                    // An auto-width element is sized to its content: its box width is
                    // the text's max-content width (+ padding/border) measured with NO
                    // wrap, then ROUNDED DOWN to an integer pixel — losing strictly
                    // less than 1px of the text's true width. So the paint layout here
                    // must allow up to 1px more than `content_width`, or it re-wraps
                    // the text at a space inside a box that was sized for one line
                    // (the box says one line, the glyphs render two). A 0.5px
                    // tolerance can't absorb a full 1px floor; 1.0px provably can
                    // (`natural - content_width == frac(natural) < 1.0`). Explicit-
                    // width elements get no tolerance — they should wrap at their width.
                    let tolerance =
                        if matches!(cs.width, crate::computed_style::DimensionValue::Auto) {
                            1.0
                        } else {
                            0.0
                        };
                    Some(content_width + tolerance)
                } else {
                    None
                }
            };

            // Skip Parley rebuild if text hasn't changed and the layout already
            // exists with the same max_width. The existing text_layout is still valid.
            if !self.tree.dirty_ifc_text_roots.contains(&root_id) {
                if let Some(existing) = &node.text_layout {
                    let old_max_width = existing.max_width;
                    let new_max_width = max_width.unwrap_or(f32::INFINITY);
                    if (old_max_width - new_max_width).abs() < 0.01 {
                        continue; // Skip — text_layout is still valid
                    }
                }
            }

            let mut inline_layout = Self::build_inline_layout(
                &self.tree.nodes,
                root_id,
                max_width,
                1.0,
                &mut self.font_cx,
                paint_layout_cx,
            );

            // text-overflow: ellipsis — if text overflows the container, truncate and add "…"
            {
                use crate::computed_style::{OverflowValue, TextOverflowValue, WhiteSpaceValue};
                let cs = &self.tree.nodes[root_id].computed_style;
                let container_width = max_width.unwrap_or(f32::INFINITY);
                if matches!(cs.text_overflow, TextOverflowValue::Ellipsis)
                    && matches!(
                        cs.white_space,
                        WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                    )
                    && matches!(cs.overflow_x, OverflowValue::Hidden | OverflowValue::Clip)
                    && inline_layout.layout.width() > container_width
                    && container_width > 0.0
                {
                    // Collect text content from the inline layout
                    let full_text = inline_layout.text_content.clone();
                    if !full_text.is_empty() {
                        inline_layout = Self::build_ellipsis_layout(
                            &self.tree.nodes,
                            root_id,
                            &full_text,
                            container_width,
                            1.0,
                            &mut self.font_cx,
                            paint_layout_cx,
                        );
                    }
                }
            }

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
    /// Also handles text-overflow: ellipsis by truncating text and appending "…"
    /// when the parent has overflow: hidden + white-space: nowrap + text-overflow: ellipsis.
    pub(crate) fn copy_cached_text_layouts(
        &mut self,
        cache: HashMap<(usize, u32), parley::layout::Layout<Brush>>,
    ) {
        use crate::computed_style::{OverflowValue, TextOverflowValue, WhiteSpaceValue};

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

        // Collect ellipsis rebuild requests: (node_id, text_content, available_width)
        let mut ellipsis_rebuilds: Vec<(usize, String, f32)> = Vec::new();

        // Apply the updates with alignment
        for (id, mut layout) in updates {
            // Get text-align from parent's computed style
            let parent_id = self.tree.nodes[id].parent;
            let alignment = parent_id
                .and_then(|p| self.tree.nodes.get(p))
                .map(|p| p.computed_style.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);

            // Check if text-overflow: ellipsis applies
            let needs_ellipsis =
                parent_id
                    .and_then(|p| self.tree.nodes.get(p))
                    .is_some_and(|parent| {
                        matches!(
                            parent.computed_style.text_overflow,
                            TextOverflowValue::Ellipsis
                        ) && matches!(
                            parent.computed_style.white_space,
                            WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                        ) && matches!(
                            parent.computed_style.overflow_x,
                            OverflowValue::Hidden | OverflowValue::Clip
                        )
                    });

            if needs_ellipsis {
                let parent_width = parent_id
                    .and_then(|p| self.tree.nodes.get(p))
                    .map(|p| p.layout.width)
                    .unwrap_or(f32::INFINITY);

                // Check if text overflows its parent
                if layout.width() > parent_width && parent_width > 0.0 {
                    if let NodeKind::Text(text_data) = &self.tree.nodes[id].kind {
                        ellipsis_rebuilds.push((id, text_data.content.clone(), parent_width));
                        continue; // Skip normal caching; will be rebuilt below
                    }
                }
            }

            // Apply alignment before caching
            layout.align(alignment, parley::layout::AlignmentOptions::default());

            self.tree.nodes[id].cached_text_parley = Some(Box::new(layout));
        }

        // Rebuild layouts for text nodes that need ellipsis truncation
        for (id, content, available_width) in ellipsis_rebuilds {
            let parent_id = self.tree.nodes[id].parent;
            let parent = parent_id.and_then(|p| self.tree.nodes.get(p));
            let font_size = parent.map(|p| p.computed_style.font_size).unwrap_or(16.0);
            let font_weight = parent
                .map(|p| p.computed_style.font_weight)
                .unwrap_or(400.0);
            let font_family = parent
                .map(|p| {
                    if p.computed_style.font_family.is_empty() {
                        "sans-serif".to_string()
                    } else {
                        p.computed_style.font_family.clone()
                    }
                })
                .unwrap_or_else(|| "sans-serif".to_string());
            let color = parent
                .and_then(|p| p.computed_style.color)
                .unwrap_or_else(|| {
                    peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(0, 0, 0, 255)
                });
            let line_height = parent.and_then(|p| p.computed_style.line_height.to_parley());
            let alignment = parent
                .map(|p| p.computed_style.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);

            // Measure the ellipsis "…" width first
            let ellipsis = "…";
            let ellipsis_width = {
                let mut builder =
                    self.layout_cx
                        .ranged_builder(&mut self.font_cx, ellipsis, 1.0, true);
                builder.push_default(parley::style::StyleProperty::FontSize(font_size));
                if (font_weight - 400.0).abs() > 1.0 {
                    builder.push_default(parley::style::StyleProperty::FontWeight(
                        parley::style::FontWeight::new(font_weight),
                    ));
                }
                builder.push_default(parley::style::StyleProperty::FontStack(
                    parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family.clone())),
                ));
                let mut layout = builder.build(ellipsis);
                layout.break_all_lines(None);
                layout.width()
            };

            let target_width = available_width - ellipsis_width;
            if target_width <= 0.0 {
                // Not even room for the ellipsis — just show ellipsis
                let mut builder =
                    self.layout_cx
                        .ranged_builder(&mut self.font_cx, ellipsis, 1.0, true);
                builder.push_default(parley::style::StyleProperty::FontSize(font_size));
                builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
                builder.push_default(parley::style::StyleProperty::FontStack(
                    parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
                ));
                if (font_weight - 400.0).abs() > 1.0 {
                    builder.push_default(parley::style::StyleProperty::FontWeight(
                        parley::style::FontWeight::new(font_weight),
                    ));
                }
                if let Some(lh) = line_height {
                    builder.push_default(parley::style::StyleProperty::LineHeight(lh));
                }
                let mut layout = builder.build(ellipsis);
                layout.break_all_lines(None);
                layout.align(alignment, parley::layout::AlignmentOptions::default());
                self.tree.nodes[id].cached_text_parley = Some(Box::new(layout));
                continue;
            }

            // Binary search for the longest prefix that fits within target_width
            let chars: Vec<char> = content.chars().collect();
            let mut lo: usize = 0;
            let mut hi: usize = chars.len();
            let mut best_len = 0;

            while lo <= hi {
                let mid = (lo + hi) / 2;
                if mid == 0 {
                    lo = 1;
                    continue;
                }
                let prefix: String = chars[..mid].iter().collect();
                let mut builder =
                    self.layout_cx
                        .ranged_builder(&mut self.font_cx, &prefix, 1.0, true);
                builder.push_default(parley::style::StyleProperty::FontSize(font_size));
                if (font_weight - 400.0).abs() > 1.0 {
                    builder.push_default(parley::style::StyleProperty::FontWeight(
                        parley::style::FontWeight::new(font_weight),
                    ));
                }
                builder.push_default(parley::style::StyleProperty::FontStack(
                    parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family.clone())),
                ));
                let mut layout = builder.build(&prefix);
                layout.break_all_lines(None);

                if layout.width() <= target_width {
                    best_len = mid;
                    lo = mid + 1;
                } else {
                    if mid == 0 {
                        break;
                    }
                    hi = mid - 1;
                }
            }

            // Build final layout with truncated text + ellipsis
            let truncated: String = chars[..best_len]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + ellipsis;
            let mut builder =
                self.layout_cx
                    .ranged_builder(&mut self.font_cx, &truncated, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(font_size));
            builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
            builder.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
            ));
            if (font_weight - 400.0).abs() > 1.0 {
                builder.push_default(parley::style::StyleProperty::FontWeight(
                    parley::style::FontWeight::new(font_weight),
                ));
            }
            if let Some(lh) = line_height {
                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
            }
            let mut layout = builder.build(&truncated);
            layout.break_all_lines(None);
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
                && let Some(child) = self.tree.nodes.get_mut(child_id)
            {
                child.layout.x = 0.0;
                child.layout.y = 0.0;
                child.layout.width = root_layout.width;
                child.layout.height = ifc_content_height;
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
                    .map(|n| n.is_inline() && !n.is_comment())
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
                let child = self.tree.nodes.get(child_id);
                let is_comment = child.map(|c| c.is_comment()).unwrap_or(false);
                // Skip comments — they have no Taffy node and should not
                // trigger anonymous block box creation.
                if is_comment {
                    continue;
                }
                let is_inline = child.map(|c| c.is_inline()).unwrap_or(false);
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
                        && let Some(anon_taffy) =
                            self.tree.nodes.get(child_id).and_then(|n| n.taffy_id)
                    {
                        let anon_children: Vec<usize> = self.tree.nodes[child_id].children.clone();
                        for &anon_child_id in &anon_children {
                            if let Some(anon_child_taffy) =
                                self.tree.nodes.get(anon_child_id).and_then(|n| n.taffy_id)
                            {
                                let _ = self.tree.taffy.add_child(anon_taffy, anon_child_taffy);
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
            // A `display: contents` element generates no box, so it can never
            // establish an inline formatting context — its inline children belong
            // to the nearest real (block or flex) ancestor. In a flex container
            // its children blockify into flex items (issue #41); in a block
            // container that block establishes the IFC over the flattened inline
            // content (issue #61, handled below). Either way the contents node is
            // never an IFC root itself.
            if node.computed_style.display == crate::computed_style::values::DisplayValue::Contents
            {
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
            //
            // `contents_wraps_only_inline` also activates the IFC when the only
            // inline content lives *behind* `display:contents` wrapper(s) — as
            // rsx `if`/`match` emit — so a block parent flows that wrapped text
            // itself instead of leaving it stranded on the phantom wrapper (#61).
            if !inline_children.is_empty() || Self::contents_wraps_only_inline(&self.tree.nodes, id)
            {
                ifc_roots.push(id);
            } else if node.children.is_empty() {
                // CSS spec: an empty block container that would establish an IFC
                // has height equal to its line-height. Without this, empty <p></p>
                // elements collapse to zero height.
                // Skip elements that already have an explicit height — e.g. separators
                // with `height: 1px` should not be inflated to line-height.
                if let Some(taffy_id) = node.taffy_id {
                    let line_h = match node.computed_style.line_height {
                        crate::computed_style::LineHeightValue::Normal => {
                            node.computed_style.font_size * 1.2
                        }
                        crate::computed_style::LineHeightValue::Relative(r) => {
                            node.computed_style.font_size * r
                        }
                        crate::computed_style::LineHeightValue::Absolute(px) => px,
                    };
                    if let Ok(style) = self.tree.taffy.style(taffy_id) {
                        let mut style = style.clone();
                        // Only inflate to line-height for auto-height elements.
                        // Elements with explicit height (e.g. separators with
                        // height: 1px) keep their size. Always call set_style
                        // for consistent Taffy invalidation.
                        // The line-height floor must not stomp an author
                        // `min-height` — it is a *floor*, not an override. A
                        // childless block (a `<textarea>`, an empty spacer div)
                        // otherwise collapses to one line no matter what the
                        // author asked for.
                        if style.size.height.is_auto() {
                            if let Some(author_min) = style.min_size.height.into_option() {
                                // An explicit length: the floor is the larger of
                                // the two.
                                style.min_size.height =
                                    taffy::Dimension::length(line_h.max(author_min));
                            } else if style.min_size.height.is_auto() {
                                style.min_size.height = taffy::Dimension::length(line_h);
                            }
                            // A percentage/calc min-height is left untouched so
                            // Taffy can resolve it against the containing block
                            // (it could not before the 0.12 upgrade, which is why
                            // this used to flatten it to `line_h`). Note the
                            // consequence: if the containing block's height is
                            // indefinite the percentage resolves to zero per CSS,
                            // so such a block collapses rather than keeping the
                            // one-line floor. That matches browsers, and an empty
                            // block with no min-height at all still gets the floor.
                        }
                        let _ = self.tree.taffy.set_style(taffy_id, style);
                    }
                }
            }
        }

        for root_id in ifc_roots {
            let root_taffy = match self.tree.nodes[root_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            // Remove inline children from Taffy (Parley handles their layout),
            // flattening through any `display:contents` wrappers so their inline
            // grandchildren join this root's IFC (issue #61).
            self.mark_inline_descendants(root_id, root_id, root_taffy);

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

    /// Whether `root_id`'s only inline-level content lives behind one or more
    /// `display:contents` wrappers (with no block-level content mixed in).
    ///
    /// `display:contents` is transparent, so a block container whose children
    /// are contents wrappers full of inline text must establish the IFC itself
    /// (issue #61). This is only consulted when `root_id` has no *direct* inline
    /// children. Returns false when any block-level element is found among the
    /// flattened content — mixed inline+block behind `display:contents` is out of
    /// scope here (it needs anonymous-block-box handling) and is left untouched
    /// rather than regressed.
    fn contents_wraps_only_inline(nodes: &slab::Slab<Node>, root_id: usize) -> bool {
        let mut found_contents_inline = false;
        if Self::scan_contents_children(nodes, root_id, &mut found_contents_inline) {
            found_contents_inline
        } else {
            false
        }
    }

    /// Whether a `display:contents` node is *transparent to the surrounding
    /// inline formatting context* — i.e. it wraps no block-level box, so every
    /// box it flattens into the ancestor belongs to that ancestor's IFC.
    ///
    /// `display:contents` is common in rsx output (`Vec<NodeHandle>` children,
    /// reactive text spans), and such a wrapper very often holds *block*
    /// content. A wrapper like that is NOT part of the ancestor's IFC: its
    /// blocks are ordinary in-flow boxes that the paint tree-walk must descend
    /// into. Marking it with `ifc_root` (which tells paint "the IFC draws this,
    /// skip it") would silently drop the whole subtree from the scene (#61
    /// regression: rows kept their layout boxes but were never drawn).
    ///
    /// Unlike [`Self::contents_wraps_only_inline`] this does not require that
    /// inline content actually be found: an empty or comment-only wrapper has
    /// nothing to paint either way, and treating it as transparent keeps
    /// document order intact for the inline content around it.
    fn contents_is_inline_transparent(nodes: &slab::Slab<Node>, node_id: usize) -> bool {
        let mut found_inline = false;
        Self::scan_contents_children(nodes, node_id, &mut found_inline)
    }

    /// Recursively classify `node_id`'s children, descending only through
    /// `display:contents` wrappers. Sets `found_inline` when inline content is
    /// seen under a contents wrapper. Returns false as soon as a block-level
    /// (non-contents) element is encountered.
    fn scan_contents_children(
        nodes: &slab::Slab<Node>,
        node_id: usize,
        found_inline: &mut bool,
    ) -> bool {
        use crate::computed_style::values::DisplayValue;
        for &child_id in &nodes[node_id].children {
            let child = match nodes.get(child_id) {
                Some(c) => c,
                None => continue,
            };
            if child.is_comment() {
                continue;
            }
            if child.computed_style.display == DisplayValue::Contents {
                if !Self::scan_contents_children(nodes, child_id, found_inline) {
                    return false;
                }
            } else if child.is_inline() {
                *found_inline = true;
            } else {
                // A real block-level box — mixed content, not our case.
                return false;
            }
        }
        true
    }

    /// Detach the inline descendants of an IFC root from Taffy and mark their
    /// `ifc_root`, flattening through `display:contents` wrappers.
    ///
    /// Direct inline children behave exactly as before (removed from the root's
    /// Taffy node so Parley lays them out, `ifc_root` set). A `display:contents`
    /// wrapper generates no box, so it is transparent: it is marked with this
    /// root's id (so IFC discovery finds this container and paint skips the
    /// wrapper in the normal tree walk) and recursed into, so its inline
    /// grandchildren — which `sync_display_contents` reparented into `root_taffy`
    /// — are detached and joined to this IFC too (issue #61).
    fn mark_inline_descendants(
        &mut self,
        root_id: usize,
        node_id: usize,
        root_taffy: taffy::NodeId,
    ) {
        use crate::computed_style::values::DisplayValue;
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();
        for child_id in children {
            let (is_contents, is_inline, child_taffy) = match self.tree.nodes.get(child_id) {
                Some(c) => (
                    c.computed_style.display == DisplayValue::Contents,
                    c.is_inline(),
                    c.taffy_id,
                ),
                None => continue,
            };
            if is_contents {
                // Only a wrapper that holds *no block-level box* is part of
                // this IFC. One that wraps blocks (an rsx `Vec<NodeHandle>`
                // child, a component's subtree) keeps `ifc_root == None` so the
                // paint tree-walk still descends into it — marking it would
                // make paint skip the wrapper and every box beneath it.
                if !Self::contents_is_inline_transparent(&self.tree.nodes, child_id) {
                    continue;
                }
                if let Some(c) = self.tree.nodes.get_mut(child_id) {
                    c.ifc_root = Some(root_id);
                }
                self.mark_inline_descendants(root_id, child_id, root_taffy);
            } else if is_inline {
                if let Some(child_taffy) = child_taffy
                    && let Ok(taffy_children) = self.tree.taffy.children(root_taffy)
                    && taffy_children.contains(&child_taffy)
                {
                    let _ = self.tree.taffy.remove_child(root_taffy, child_taffy);
                }
                if let Some(c) = self.tree.nodes.get_mut(child_id) {
                    c.ifc_root = Some(root_id);
                }
            }
            // Block-level children are left in place (existing behavior).
        }
    }

    /// Pre-compute layout for inline-block children that were detached from Taffy.
    ///
    /// Inline-block children are removed from their parent's Taffy tree (so the parent
    /// uses InlineRoot measurement), but they still need their own subtree computed
    /// so `walk_inline_children` can read their width/height for Parley InlineBox.
    pub(crate) fn compute_inline_block_layouts(&mut self) {
        // Collect inline-block children that belong to an IFC
        let mut ib_taffy_ids: Vec<(taffy::NodeId, Option<f32>)> = Vec::new();
        for (_id, node) in &self.tree.nodes {
            if node.ifc_root.is_some()
                && node.display_mode == DisplayMode::InlineBlock
                && let Some(taffy_id) = node.taffy_id
            {
                ib_taffy_ids.push((taffy_id, None));
            }
        }
        self.measure_inline_blocks(&ib_taffy_ids);
    }

    /// Measure a set of detached inline-blocks.
    ///
    /// `available_width` is the definite inner width of the inline-block's
    /// containing block, where it is known. Taffy resolves a *root* node's
    /// percentage sizes against its available space — `compute_root_layout` turns
    /// `AvailableSpace` into the `parent_size` basis — so passing `Some(w)` is what
    /// lets a `width: 50%` inline-block resolve at all. `None` measures at
    /// max-content, which is right for auto and definite sizes but leaves a
    /// percentage with no basis, collapsing it to min-content (issue #120).
    fn measure_inline_blocks(&mut self, targets: &[(taffy::NodeId, Option<f32>)]) {
        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;

        for &(taffy_id, available_width) in targets {
            let avail = taffy::Size {
                width: match available_width {
                    Some(w) => taffy::AvailableSpace::Definite(w),
                    None => taffy::AvailableSpace::MaxContent,
                },
                height: taffy::AvailableSpace::MaxContent,
            };
            // Split-borrow so the closure captures only `self.tree.nodes`,
            // leaving `self.tree.nodes.get_mut` free after the call returns.
            let nodes = &self.tree.nodes;
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
                        Some(NodeContext::InlineRoot(root_id)) => {
                            let root_id = *root_id;
                            if let Some(est_h) = nodes[root_id].estimated_height {
                                return taffy::Size {
                                    width: known_dims.width.unwrap_or(0.0),
                                    height: known_dims.height.unwrap_or(est_h),
                                };
                            }
                            let inline_layout = Self::build_inline_layout(
                                nodes, root_id, max_width, 1.0, font_cx, layout_cx,
                            );
                            taffy::Size {
                                width: known_dims.width.unwrap_or(inline_layout.layout.width()),
                                height: known_dims.height.unwrap_or(inline_layout.layout.height()),
                            }
                        }
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
                        Some(NodeContext::Image { width, height, .. }) => {
                            let iw = *width as f32;
                            let ih = *height as f32;
                            if iw == 0.0 || ih == 0.0 {
                                return taffy::Size::ZERO;
                            }
                            taffy::Size {
                                width: known_dims.width.unwrap_or(iw),
                                height: known_dims.height.unwrap_or_else(|| {
                                    if let Some(kw) = known_dims.width {
                                        ih * (kw / iw)
                                    } else {
                                        ih
                                    }
                                }),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            );

            // Read the computed layout back into the node.
            // Re-borrow `self.tree` now that the closure (and its borrow of
            // `self.tree.nodes` / `self.tree.taffy`) has been dropped.
            if let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id) {
                let layout_size = taffy_layout.size;
                let node_id = self.tree.taffy_map.get(&taffy_id).copied();
                if let Some(nid) = node_id
                    && let Some(node) = self.tree.nodes.get_mut(nid)
                {
                    node.layout.width = layout_size.width;
                    node.layout.height = layout_size.height;
                }
            }
        }
    }

    /// Whether any of this style's inline-axis sizes is a percentage, and so needs
    /// a containing-block width to resolve against.
    fn has_percentage_inline_size(style: &crate::computed_style::ComputedStyle) -> bool {
        use crate::computed_style::DimensionValue::Percent;
        matches!(style.width, Percent(_))
            || matches!(style.min_width, Percent(_))
            || matches!(style.max_width, Percent(_))
    }

    /// Re-measure inline-blocks whose inline size is a percentage, now that their
    /// containing block has a computed width (issue #120).
    ///
    /// `compute_inline_block_layouts` runs *before* the root Taffy compute, so a
    /// percentage width has nothing to resolve against and collapses to
    /// min-content. This runs *after* that compute, when the containing block's
    /// width is real, and re-measures against it.
    ///
    /// Returns `true` if any inline-block changed size — the caller must then
    /// re-run the root compute so the enclosing IFCs line-break against the
    /// corrected boxes. Returns `false` (doing no work) when no inline-block has a
    /// percentage inline size, which is the overwhelmingly common case.
    ///
    /// Only the *inline* axis is corrected. A percentage height on an inline-block
    /// resolves against a containing-block height that is itself usually content-
    /// derived, so there is no non-circular basis to feed back here.
    pub(crate) fn resolve_percentage_inline_blocks(&mut self) -> bool {
        // What to re-measure, and what to compare against afterwards:
        // targets  = (taffy id, containing block inner width)
        // affected = (node id, IFC root id, width before, height before)
        let mut targets: Vec<(taffy::NodeId, Option<f32>)> = Vec::new();
        let mut affected: Vec<(usize, usize, f32, f32)> = Vec::new();

        for (id, node) in &self.tree.nodes {
            let (Some(root_id), Some(taffy_id)) = (node.ifc_root, node.taffy_id) else {
                continue;
            };
            if node.display_mode != DisplayMode::InlineBlock
                || !Self::has_percentage_inline_size(&node.computed_style)
            {
                continue;
            }
            // The IFC root is this inline-block's containing block by
            // construction: IFC roots are never inline, inline-block or flex.
            let Some(cb_taffy) = self.tree.nodes[root_id].taffy_id else {
                continue;
            };
            let Ok(cb) = self.tree.taffy.layout(cb_taffy) else {
                continue;
            };
            let inner_width = cb.size.width
                - cb.padding.left
                - cb.padding.right
                - cb.border.left
                - cb.border.right;
            if !inner_width.is_finite() || inner_width <= 0.0 {
                continue;
            }
            targets.push((taffy_id, Some(inner_width)));
            affected.push((id, root_id, node.layout.width, node.layout.height));
        }

        if targets.is_empty() {
            return false;
        }

        // Taffy caches per (node, available space); the first pass measured these
        // under MaxContent, so mark them dirty to force a real re-measure.
        for &(taffy_id, _) in &targets {
            let _ = self.tree.taffy.mark_dirty(taffy_id);
        }
        self.measure_inline_blocks(&targets);

        let mut changed = false;
        for &(id, root_id, prev_w, prev_h) in &affected {
            let node = &self.tree.nodes[id];
            if (node.layout.width - prev_w).abs() > 0.5 || (node.layout.height - prev_h).abs() > 0.5
            {
                changed = true;
                // This IFC root's inline layout was measured against the stale box.
                if let Some(root_taffy) = self.tree.nodes[root_id].taffy_id {
                    let _ = self.tree.taffy.mark_dirty(root_taffy);
                }
            }
        }

        if changed {
            // Measures cached under the stale inline-block sizes would be served
            // straight back on the second pass, re-introducing the collapse.
            self.tree.ifc_measure_cache.clear();
        }
        changed
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

        // Apply text-underline-offset from computed style
        if let Some(offset) = root_computed.text_underline_offset {
            root_text_style.underline_offset = Some(offset);
        }

        // Apply overflow-wrap from computed style
        root_text_style.overflow_wrap = root_computed.overflow_wrap.to_parley();

        let mut builder = layout_cx.tree_builder(font_cx, scale, true, &root_text_style);

        // Apply white-space mode from computed style.
        // Contenteditable elements always use Preserve (pre-wrap) to prevent
        // Parley from collapsing trailing whitespace, which would cause cursor
        // position mismatches (the DOM text has the space but the layout doesn't).
        use crate::computed_style::WhiteSpaceValue;
        let is_contenteditable = {
            let mut nid = Some(root_id);
            let mut found = false;
            while let Some(id) = nid {
                if nodes[id].attributes.contains_key("contenteditable") {
                    found = true;
                    break;
                }
                nid = nodes[id].parent;
            }
            found
        };
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
        let mut background_spans = Vec::new();
        let mut flat_pos = 0usize;

        // Walk children and build the Parley tree
        Self::walk_inline_children(
            nodes,
            root_id,
            &mut builder,
            &mut child_positions,
            &mut text_ranges,
            &mut background_spans,
            &mut flat_pos,
            scale,
            collapse,
        );

        let (text_layout, text_content) = builder.build();
        let mut text_layout = text_layout;
        // white-space: nowrap/pre prevents line wrapping — use infinite width
        let effective_max_width = match root_computed.white_space {
            WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre => None,
            _ => max_width,
        };
        text_layout.break_all_lines(effective_max_width);

        // Apply text-align from computed style
        let alignment = root_computed.text_align.to_parley();
        text_layout.align(alignment, parley::layout::AlignmentOptions::default());

        InlineLayout {
            layout: text_layout,
            text_content,
            child_positions,
            text_ranges,
            background_spans,
            max_width: max_width.unwrap_or(f32::INFINITY),
        }
    }

    /// Build an IFC layout with ellipsis truncation.
    ///
    /// Binary-searches for the longest text prefix that fits within `container_width`
    /// when combined with an ellipsis character, then rebuilds the layout.
    #[allow(clippy::too_many_arguments)]
    fn build_ellipsis_layout(
        nodes: &slab::Slab<Node>,
        root_id: usize,
        full_text: &str,
        container_width: f32,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<Brush>,
    ) -> InlineLayout {
        let root_computed = &nodes[root_id].computed_style;
        let font_size = root_computed.font_size * scale;
        let font_family: std::borrow::Cow<'static, str> = if root_computed.font_family.is_empty() {
            "sans-serif".into()
        } else {
            root_computed.font_family.clone().into()
        };
        let font_weight = parley::style::FontWeight::new(root_computed.font_weight);
        let color = root_computed.color.unwrap_or_else(|| {
            peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(0, 0, 0, 255)
        });
        let line_height = root_computed.line_height.to_parley();
        let alignment = root_computed.text_align.to_parley();

        let ellipsis = "\u{2026}";

        // Measure ellipsis width
        let ellipsis_width = {
            let mut b = layout_cx.ranged_builder(font_cx, ellipsis, scale, true);
            b.push_default(parley::style::StyleProperty::FontSize(font_size));
            b.push_default(parley::style::StyleProperty::FontWeight(font_weight));
            b.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Source(font_family.clone()),
            ));
            let mut l = b.build(ellipsis);
            l.break_all_lines(None);
            l.width()
        };

        let target_width = container_width - ellipsis_width;
        let chars: Vec<char> = full_text.chars().collect();
        let mut best_len = 0;

        if target_width > 0.0 {
            let mut lo: usize = 0;
            let mut hi: usize = chars.len();
            while lo <= hi {
                let mid = (lo + hi) / 2;
                if mid == 0 {
                    lo = 1;
                    continue;
                }
                let prefix: String = chars[..mid].iter().collect();
                let mut b = layout_cx.ranged_builder(font_cx, &prefix, scale, true);
                b.push_default(parley::style::StyleProperty::FontSize(font_size));
                b.push_default(parley::style::StyleProperty::FontWeight(font_weight));
                b.push_default(parley::style::StyleProperty::FontStack(
                    parley::style::FontStack::Source(font_family.clone()),
                ));
                let mut l = b.build(&prefix);
                l.break_all_lines(None);
                if l.width() <= target_width {
                    best_len = mid;
                    lo = mid + 1;
                } else {
                    if mid == 0 {
                        break;
                    }
                    hi = mid - 1;
                }
            }
        }

        // Build final layout: truncated text + ellipsis
        let truncated: String = chars[..best_len]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + ellipsis;

        let mut b = layout_cx.ranged_builder(font_cx, &truncated, scale, true);
        b.push_default(parley::style::StyleProperty::FontSize(font_size));
        b.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
        b.push_default(parley::style::StyleProperty::FontWeight(font_weight));
        b.push_default(parley::style::StyleProperty::FontStack(
            parley::style::FontStack::Source(font_family),
        ));
        if let Some(lh) = line_height {
            b.push_default(parley::style::StyleProperty::LineHeight(lh));
        }
        let mut layout = b.build(&truncated);
        layout.break_all_lines(None);
        layout.align(alignment, parley::layout::AlignmentOptions::default());

        InlineLayout {
            layout,
            text_content: truncated,
            child_positions: Vec::new(),
            text_ranges: Vec::new(),
            background_spans: Vec::new(),
            max_width: container_width,
        }
    }

    /// Recursively walk inline children, pushing text and style spans into the TreeBuilder.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn walk_inline_children(
        nodes: &slab::Slab<Node>,
        parent_id: usize,
        builder: &mut parley::TreeBuilder<'_, Brush>,
        child_positions: &mut Vec<(usize, LayoutResult)>,
        text_ranges: &mut Vec<crate::node::IfcTextRange>,
        background_spans: &mut Vec<crate::node::InlineBackgroundSpan>,
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
                        // Apply text-transform from parent's computed style
                        let parent_transform = &nodes[parent_id].computed_style.text_transform;
                        let raw =
                            if let Some(transformed) = parent_transform.apply(&text_data.content) {
                                transformed
                            } else {
                                text_data.content.clone()
                            };
                        let dom_text_len = raw.len();
                        // Expand tabs to 4 spaces for Parley (which has no tab stop support)
                        let has_tabs = raw.contains('\t');
                        let display = if has_tabs {
                            raw.replace('\t', "    ")
                        } else {
                            raw
                        };
                        builder.push_text(&display);
                        *flat_pos += display.len();
                        text_ranges.push(crate::node::IfcTextRange {
                            flat_start: start,
                            flat_end: *flat_pos,
                            node_id: child_id,
                            node_offset: 0,
                            is_br: false,
                            dom_text_len,
                            dom_text: if has_tabs {
                                text_data.content.clone()
                            } else {
                                String::new()
                            },
                        });
                        // Record position placeholder — actual position comes from layout
                        child_positions.push((child_id, LayoutResult::default()));
                    }
                }
                NodeKind::Element(_)
                    if child.display_mode == DisplayMode::Inline && child.tag() == Some("br") =>
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
                        dom_text_len: 1,
                        dom_text: String::new(),
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
                    // Underline offset
                    if let Some(offset) = child_computed.text_underline_offset {
                        props.push(parley::style::StyleProperty::UnderlineOffset(Some(offset)));
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

                    // Record background span start position
                    let bg_start = *flat_pos;
                    let has_bg = child_computed.background_color().is_some();

                    builder.push_style_modification_span(props.iter());
                    child_positions.push((child_id, LayoutResult::default()));

                    // Recurse into inline element's children
                    Self::walk_inline_children(
                        nodes,
                        child_id,
                        builder,
                        child_positions,
                        text_ranges,
                        background_spans,
                        flat_pos,
                        scale,
                        collapse,
                    );

                    builder.pop_style_span();

                    // Record background span if the inline element has a visible background
                    if has_bg && *flat_pos > bg_start {
                        let bg_color = child_computed.background_color().unwrap();
                        // Skip transparent backgrounds (alpha == 0)
                        if bg_color.components[3] > 0.0 {
                            background_spans.push(crate::node::InlineBackgroundSpan {
                                start: bg_start,
                                end: *flat_pos,
                                color: bg_color,
                                padding_left: child_computed.padding_left.to_px(),
                                padding_right: child_computed.padding_right.to_px(),
                                padding_top: child_computed.padding_top.to_px(),
                                padding_bottom: child_computed.padding_bottom.to_px(),
                                border_radius: child_computed.border_radius_top_left.to_px(),
                            });
                        }
                    }
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
                NodeKind::Element(_)
                    if child.computed_style.display
                        == crate::computed_style::values::DisplayValue::Contents
                        && Self::contents_is_inline_transparent(nodes, child_id) =>
                {
                    // `display:contents` generates no box — it is transparent.
                    // Recurse without pushing a style span so its inline
                    // descendants flow into this IFC in document order (#61).
                    //
                    // A wrapper holding block-level content is *not* transparent
                    // to the IFC: it falls through to the `_` arm below and
                    // breaks the inline flow, exactly as the block it wraps
                    // would if it were a direct child.
                    Self::walk_inline_children(
                        nodes,
                        child_id,
                        builder,
                        child_positions,
                        text_ranges,
                        background_spans,
                        flat_pos,
                        scale,
                        collapse,
                    );
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
