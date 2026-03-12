//! Layout engine: Taffy layout resolution, text measurement, display:contents, and IFC invalidation.

use std::collections::HashMap;

use peniko::Brush;
use peniko::color::{AlphaColor, Srgb};

use crate::RinchDocument;
use crate::layout;
use crate::node::{LayoutResult, NodeContext, NodeKind};

impl RinchDocument {
    /// Resolve layout using Taffy.
    ///
    /// Computes layout for the entire tree given a viewport size,
    /// then reads layout results back into each node's `layout` field.
    /// Text nodes are measured using Parley for accurate text layout.
    pub fn resolve_layout(&mut self, width: f32, height: f32) {
        let perf = std::env::var("RINCH_PERF").is_ok();
        let t0 = std::time::Instant::now();

        let old_viewport = self.tree.viewport;
        self.tree.viewport = crate::layout::Viewport { width, height };

        // When viewport changes, update Stylo's Device and invalidate all cached styles
        // so that vh/vw units are recomputed with the new viewport dimensions
        if (old_viewport.width - width).abs() > 0.5 || (old_viewport.height - height).abs() > 0.5 {
            self.set_stylist_viewport(width, height);

            // Invalidate all cached stylo_element_data so styles are recomputed with new viewport
            for (node_id, _) in self.tree.nodes.iter() {
                *self.tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
            }
            self.tree.style_roots.clear(); // Force full tree walk
            self.tree.styles_dirty = true;
        }

        // Drain completed image loads and update intrinsic dimensions
        self.drain_pending_images();

        // Resolve Stylo styles and apply to Taffy nodes (only if dirty)
        if self.tree.styles_dirty {
            let t = std::time::Instant::now();
            self.resolve_styles();
            if perf {
                eprintln!(
                    "  [PERF] resolve_styles: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
            let t = std::time::Instant::now();
            self.apply_stylo_styles_to_taffy();
            if perf {
                eprintln!(
                    "  [PERF] apply_to_taffy: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
            self.tree.styles_dirty = false;
        }

        // Trigger loads for any background-image URLs not yet in the cache
        self.request_background_image_loads();

        // Skip full layout recompute when no layout-affecting properties changed.
        // Paint-only changes (background-color, opacity, cursor on hover) set
        // styles_dirty but NOT layout_dirty, so we resolve styles above but
        // skip the expensive Taffy compute + IFC rebuild below.
        //
        // However, text-affecting style changes (font-size, font-weight, font-style,
        // text-decoration) don't change Taffy styles so layout_dirty won't be set.
        // For these, skip Taffy but still rebuild the affected IFC text layouts.
        if !self.tree.layout_dirty {
            if !self.tree.dirty_ifc_text_roots.is_empty() {
                let t = std::time::Instant::now();
                self.sync_dirty_text_contexts();
                let mut temp_layout_cx = std::mem::take(&mut self.layout_cx);
                self.build_ifc_layouts(&mut temp_layout_cx);
                self.layout_cx = temp_layout_cx;
                self.tree.dirty_ifc_text_roots.clear();
                if perf {
                    eprintln!(
                        "  [PERF] layout SKIPPED (text-only IFC rebuild) {:.2}ms",
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                }
            } else if perf {
                eprintln!(
                    "  [PERF] layout SKIPPED (paint-only) {:.2}ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            return;
        }
        self.tree.layout_dirty = false;

        let root_taffy = match self.tree.nodes[self.tree.root_id].taffy_id {
            Some(id) => id,
            None => return,
        };

        // IFC setup passes only need to run when tree structure or display modes
        // changed. Text-only changes (e.g. slider label "50%" → "19%") preserve the
        // existing IFC structure and Taffy's internal cache — only the dirty text
        // node gets re-measured, avoiding the 80ms full-tree Parley rebuild.
        if self.tree.ifc_dirty {
            // Structural change — invalidate all cached IFC measures
            self.tree.ifc_measure_cache.clear();

            // Handle display:contents by rebuilding taffy children for affected nodes
            let t = std::time::Instant::now();
            self.sync_display_contents();
            if perf {
                eprintln!(
                    "  [PERF] sync_display_contents: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Detect and set up inline formatting contexts
            let t = std::time::Instant::now();
            self.setup_inline_formatting_contexts();
            if perf {
                eprintln!(
                    "  [PERF] setup_ifc: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Pre-compute layout for inline-block children that were detached from Taffy.
            // They need their own subtree measured so walk_inline_children can read dimensions.
            let t = std::time::Instant::now();
            self.compute_inline_block_layouts();
            if perf {
                eprintln!(
                    "  [PERF] inline_block_layouts: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Sync font-size from parent elements to text node contexts
            let t = std::time::Instant::now();
            self.sync_text_contexts();
            if perf {
                eprintln!(
                    "  [PERF] sync_text_contexts: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            self.tree.ifc_dirty = false;
        } else {
            // IFC structure unchanged — only sync text contexts for dirty nodes
            let t = std::time::Instant::now();
            self.sync_dirty_text_contexts();
            if perf {
                eprintln!(
                    "  [PERF] sync_dirty_text_contexts: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
        }

        let available_space = taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        };

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;
        let nodes = &self.tree.nodes;
        let dirty_ifc_text_roots = &self.tree.dirty_ifc_text_roots;

        // Cache for text layouts built during measurement.
        // Key: (node_id, wrap_width as bits) - wrap width is part of key since layout depends on it
        // Value: Parley layout
        use std::cell::RefCell;
        let text_layout_cache: RefCell<HashMap<(usize, u32), parley::layout::Layout<Brush>>> =
            RefCell::new(HashMap::new());
        // Persistent IFC measure cache — survives across frames.
        // Only invalidated when an IFC root's text content changes.
        let ifc_measure_cache = RefCell::new(std::mem::take(&mut self.tree.ifc_measure_cache));

        let t = std::time::Instant::now();
        self.tree
            .taffy
            .compute_layout_with_measure(
                root_taffy,
                available_space,
                |known_dims, avail_space, _node_id, context, _style| {
                    let max_width = match avail_space.width {
                        taffy::AvailableSpace::Definite(w) => Some(w),
                        taffy::AvailableSpace::MaxContent => None,
                        taffy::AvailableSpace::MinContent => Some(0.0),
                    };

                    match context {
                        Some(NodeContext::Text(text)) => {
                            if text.content.is_empty() {
                                return taffy::Size {
                                    width: 0.0,
                                    height: 0.0,
                                };
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
                            // Add brush so the cached layout can be rendered with color
                            builder.push_default(parley::style::StyleProperty::Brush(
                                Brush::Solid(text.color),
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

                            // Cache the layout for use during paint
                            // Use wrap_width bits as part of the key since layout depends on it
                            let wrap_bits = wrap_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);
                            text_layout_cache
                                .borrow_mut()
                                .insert((text.node_id, wrap_bits), layout);

                            taffy::Size {
                                width: known_dims.width.unwrap_or_else(|| {
                                    text_layout_cache
                                        .borrow()
                                        .get(&(text.node_id, wrap_bits))
                                        .map(|l| l.width())
                                        .unwrap_or(0.0)
                                }),
                                height: known_dims.height.unwrap_or_else(|| {
                                    text_layout_cache
                                        .borrow()
                                        .get(&(text.node_id, wrap_bits))
                                        .map(|l| l.height())
                                        .unwrap_or(0.0)
                                }),
                            }
                        }
                        Some(NodeContext::Image { width, height, .. }) => {
                            let iw = *width as f32;
                            let ih = *height as f32;
                            if iw == 0.0 || ih == 0.0 {
                                // Image still loading — return zero size
                                return taffy::Size::ZERO;
                            }
                            let aspect = iw / ih;
                            // Use intrinsic dimensions as default, but respect
                            // CSS width/height if set (via known_dims from Taffy style).
                            // Maintain aspect ratio when only one dimension is constrained.
                            let w = match (known_dims.width, known_dims.height) {
                                (Some(kw), _) => kw,
                                (None, Some(kh)) => kh * aspect,
                                (None, None) => iw,
                            };
                            let h = match (known_dims.height, known_dims.width) {
                                (Some(kh), _) => kh,
                                (None, Some(kw)) => kw / aspect,
                                (None, None) => ih,
                            };
                            taffy::Size {
                                width: w,
                                height: h,
                            }
                        }
                        Some(NodeContext::InlineRoot(root_id)) => {
                            let root_id = *root_id;

                            // Use wrap_width bits as cache key
                            let wrap_bits = max_width.map(|w| w.to_bits()).unwrap_or(u32::MAX);

                            // Check persistent IFC measure cache — skip expensive
                            // Parley rebuild if this root's text hasn't changed.
                            if !dirty_ifc_text_roots.contains(&root_id) {
                                if let Some(&(cached_w, cached_h)) =
                                    ifc_measure_cache.borrow().get(&(root_id, wrap_bits))
                                {
                                    return taffy::Size {
                                        width: known_dims.width.unwrap_or(cached_w),
                                        height: known_dims.height.unwrap_or(cached_h),
                                    };
                                }
                            }

                            // Full Parley rebuild (text changed or cache miss)
                            let inline_layout = Self::build_inline_layout(
                                nodes, root_id, max_width, 1.0, font_cx, layout_cx,
                            );
                            let w = inline_layout.layout.width();
                            let h = inline_layout.layout.height();

                            // Store in persistent cache
                            ifc_measure_cache
                                .borrow_mut()
                                .insert((root_id, wrap_bits), (w, h));

                            // Measure callback for IFC root
                            taffy::Size {
                                width: known_dims.width.unwrap_or(w),
                                height: known_dims.height.unwrap_or(h),
                            }
                        }
                        _ => taffy::Size::ZERO,
                    }
                },
            )
            .unwrap();

        // Restore the persistent IFC measure cache (dirty_ifc_text_roots cleared after build_ifc)
        self.tree.ifc_measure_cache = ifc_measure_cache.into_inner();

        if perf {
            eprintln!(
                "  [PERF] taffy_compute: {:.2}ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Read layout results back into nodes
        let t = std::time::Instant::now();
        self.read_layout_results(self.tree.root_id);
        if perf {
            eprintln!(
                "  [PERF] read_layout: {:.2}ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Build inline layouts for IFC roots (rebuild with final widths and store)
        // Temporarily take layout_cx out to avoid borrow conflict
        let t = std::time::Instant::now();
        let mut temp_layout_cx = std::mem::take(&mut self.layout_cx);
        self.build_ifc_layouts(&mut temp_layout_cx);
        self.layout_cx = temp_layout_cx;
        self.tree.dirty_ifc_text_roots.clear();
        if perf {
            eprintln!(
                "  [PERF] build_ifc: {:.2}ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Copy cached text layouts to nodes (use the exact layouts from measurement)
        let t = std::time::Instant::now();
        self.copy_cached_text_layouts(text_layout_cache.into_inner());
        if perf {
            eprintln!(
                "  [PERF] copy_text_layouts: {:.2}ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }

        if perf {
            eprintln!(
                "  [PERF] resolve_layout TOTAL: {:.2}ms",
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Enable transitions after first layout completes (prevents transitions on page load)
        if !self.tree.transitions_enabled {
            self.tree.transitions_enabled = true;
        }
    }

    /// Incremental version of `sync_text_contexts` — only processes text nodes
    /// that are in `dirty_nodes`. Used when IFC structure is unchanged (ifc_dirty=false)
    /// to avoid walking all text nodes.
    #[allow(clippy::type_complexity)]
    pub(crate) fn sync_dirty_text_contexts(&mut self) {
        use crate::computed_style::{OverflowValue, WhiteSpaceValue};
        let mut updates: Vec<(
            taffy::NodeId,
            usize,
            f32,
            f32,
            String,
            String,
            AlphaColor<Srgb>,
            bool,
            crate::computed_style::OverflowWrapValue,
            crate::computed_style::TextOverflowValue,
            bool,
        )> = Vec::new();

        for &id in &self.tree.dirty_nodes {
            let node = match self.tree.nodes.get(id) {
                Some(n) => n,
                None => continue,
            };
            if !matches!(&node.kind, NodeKind::Text(_)) {
                continue;
            }
            let taffy_id = match node.taffy_id {
                Some(t) => t,
                None => continue,
            };

            let (
                font_size,
                font_weight,
                font_family,
                line_height_css,
                color,
                no_wrap,
                overflow_wrap,
                text_overflow,
                parent_overflow_hidden,
            ) = node
                .parent
                .and_then(|p| self.tree.nodes.get(p))
                .map(|parent| {
                    let font_size = parent.computed_style.font_size;
                    let font_weight = parent.computed_style.font_weight;
                    let font_family = if parent.computed_style.font_family.is_empty() {
                        "sans-serif".to_string()
                    } else {
                        parent.computed_style.font_family.clone()
                    };
                    let line_height_css = match &parent.computed_style.line_height {
                        crate::computed_style::LineHeightValue::Normal => String::new(),
                        crate::computed_style::LineHeightValue::Absolute(v) => {
                            format!("{}px", v)
                        }
                        crate::computed_style::LineHeightValue::Relative(v) => v.to_string(),
                    };
                    let color = parent
                        .computed_style
                        .color
                        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
                    let no_wrap = matches!(
                        parent.computed_style.white_space,
                        WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                    );
                    let overflow_wrap = parent.computed_style.overflow_wrap;
                    let text_overflow = parent.computed_style.text_overflow;
                    let parent_overflow_hidden = matches!(
                        parent.computed_style.overflow_x,
                        OverflowValue::Hidden | OverflowValue::Clip
                    );
                    (
                        font_size,
                        font_weight,
                        font_family,
                        line_height_css,
                        color,
                        no_wrap,
                        overflow_wrap,
                        text_overflow,
                        parent_overflow_hidden,
                    )
                })
                .unwrap_or((
                    16.0,
                    400.0,
                    "sans-serif".to_string(),
                    String::new(),
                    AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255),
                    false,
                    crate::computed_style::OverflowWrapValue::default(),
                    crate::computed_style::TextOverflowValue::default(),
                    false,
                ));

            updates.push((
                taffy_id,
                id,
                font_size,
                font_weight,
                font_family,
                line_height_css,
                color,
                no_wrap,
                overflow_wrap,
                text_overflow,
                parent_overflow_hidden,
            ));
        }

        for (
            taffy_id,
            node_id,
            font_size,
            font_weight,
            font_family,
            line_height_css,
            color,
            no_wrap,
            overflow_wrap,
            text_overflow,
            parent_overflow_hidden,
        ) in updates
        {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                && let NodeContext::Text(tm) = ctx
            {
                tm.font_size = font_size;
                tm.font_weight = font_weight;
                tm.font_family = font_family;
                tm.line_height_css = line_height_css;
                tm.node_id = node_id;
                tm.color = color;
                tm.no_wrap = no_wrap;
                tm.overflow_wrap = overflow_wrap;
                tm.text_overflow = text_overflow;
                tm.parent_overflow_hidden = parent_overflow_hidden;
            }
        }
    }

    /// Sync font-size from parent elements into text node contexts.
    ///
    /// Walks all text nodes and updates their `TextMeasure.font_size`
    /// from the parent element's computed style.
    #[allow(clippy::type_complexity)]
    pub(crate) fn sync_text_contexts(&mut self) {
        use crate::computed_style::{OverflowValue, WhiteSpaceValue};
        let mut updates: Vec<(
            taffy::NodeId,
            usize,
            f32,
            f32,
            String,
            String,
            AlphaColor<Srgb>,
            bool,
            crate::computed_style::OverflowWrapValue,
            crate::computed_style::TextOverflowValue,
            bool,
        )> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if let NodeKind::Text(_) = &node.kind {
                let taffy_id = match node.taffy_id {
                    Some(t) => t,
                    None => continue,
                };

                // Read from parent's parsed computed_style instead of parsing CSS strings
                let (
                    font_size,
                    font_weight,
                    font_family,
                    line_height_css,
                    color,
                    no_wrap,
                    overflow_wrap,
                    text_overflow,
                    parent_overflow_hidden,
                ) = node
                    .parent
                    .and_then(|p| self.tree.nodes.get(p))
                    .map(|parent| {
                        let font_size = parent.computed_style.font_size;
                        let font_weight = parent.computed_style.font_weight;
                        let font_family = if parent.computed_style.font_family.is_empty() {
                            "sans-serif".to_string()
                        } else {
                            parent.computed_style.font_family.clone()
                        };
                        let line_height_css = match &parent.computed_style.line_height {
                            crate::computed_style::LineHeightValue::Normal => String::new(),
                            crate::computed_style::LineHeightValue::Absolute(v) => {
                                format!("{}px", v)
                            }
                            crate::computed_style::LineHeightValue::Relative(v) => v.to_string(),
                        };
                        let color = parent
                            .computed_style
                            .color
                            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
                        // Check if white-space prevents wrapping
                        let no_wrap = matches!(
                            parent.computed_style.white_space,
                            WhiteSpaceValue::NoWrap | WhiteSpaceValue::Pre
                        );
                        let overflow_wrap = parent.computed_style.overflow_wrap;
                        let text_overflow = parent.computed_style.text_overflow;
                        let parent_overflow_hidden = matches!(
                            parent.computed_style.overflow_x,
                            OverflowValue::Hidden | OverflowValue::Clip
                        );
                        (
                            font_size,
                            font_weight,
                            font_family,
                            line_height_css,
                            color,
                            no_wrap,
                            overflow_wrap,
                            text_overflow,
                            parent_overflow_hidden,
                        )
                    })
                    .unwrap_or((
                        16.0,
                        400.0,
                        "sans-serif".to_string(),
                        String::new(),
                        AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255),
                        false,
                        crate::computed_style::OverflowWrapValue::default(),
                        crate::computed_style::TextOverflowValue::default(),
                        false,
                    ));

                updates.push((
                    taffy_id,
                    id,
                    font_size,
                    font_weight,
                    font_family,
                    line_height_css,
                    color,
                    no_wrap,
                    overflow_wrap,
                    text_overflow,
                    parent_overflow_hidden,
                ));
            }
        }

        for (
            taffy_id,
            node_id,
            font_size,
            font_weight,
            font_family,
            line_height_css,
            color,
            no_wrap,
            overflow_wrap,
            text_overflow,
            parent_overflow_hidden,
        ) in updates
        {
            if let Some(ctx) = self.tree.taffy.get_node_context_mut(taffy_id)
                && let NodeContext::Text(tm) = ctx
            {
                tm.font_size = font_size;
                tm.font_weight = font_weight;
                tm.font_family = font_family;
                tm.line_height_css = line_height_css;
                tm.node_id = node_id;
                tm.color = color;
                tm.no_wrap = no_wrap;
                tm.overflow_wrap = overflow_wrap;
                tm.text_overflow = text_overflow;
                tm.parent_overflow_hidden = parent_overflow_hidden;
            }
        }
    }

    /// Recursively read Taffy layout results into node LayoutResult fields.
    pub(crate) fn read_layout_results(&mut self, node_id: usize) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id
            && let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id)
        {
            let node = &mut self.tree.nodes[node_id];
            let mut new_layout = LayoutResult {
                x: taffy_layout.location.x,
                y: taffy_layout.location.y,
                width: taffy_layout.size.width,
                height: taffy_layout.size.height,
            };

            // position: fixed elements use the viewport as their containing block.
            // Taffy treats them as absolute (relative to parent), so we override
            // their layout to be viewport-relative with proper sizing from insets.
            // Skip when:
            //   - display is none (element itself is hidden)
            //   - Taffy computed 0x0 size (ancestor has display:none — the node's
            //     own display may be Block but it's inside a hidden subtree)
            if node.computed_style.position == crate::computed_style::PositionValue::Fixed
                && !matches!(
                    node.computed_style.display,
                    crate::computed_style::DisplayValue::None
                )
                && (new_layout.width > 0.0 || new_layout.height > 0.0)
            {
                let vw = self.tree.viewport.width;
                let vh = self.tree.viewport.height;
                let style = &node.computed_style;

                // Resolve insets (top/right/bottom/left)
                let top = style.top.resolve(vh);
                let right = style.right.resolve(vw);
                let bottom = style.bottom.resolve(vh);
                let left = style.left.resolve(vw);

                new_layout.x = left.unwrap_or(0.0);
                new_layout.y = top.unwrap_or(0.0);

                // If both horizontal insets are set, compute width from viewport
                if let (Some(l), Some(r)) = (left, right) {
                    if style.width.is_auto() {
                        new_layout.width = (vw - l - r).max(0.0);
                    }
                }
                // If both vertical insets are set, compute height from viewport
                if let (Some(t), Some(b)) = (top, bottom) {
                    if style.height.is_auto() {
                        new_layout.height = (vh - t - b).max(0.0);
                    }
                }
            }

            // Save previous layout for dirty region computation
            node.prev_layout = node.layout;
            if node.layout != new_layout {
                node.layout = new_layout;
                self.tree.paint_dirty_nodes.push(node_id);
            }
        }

        for child_id in children {
            self.read_layout_results(child_id);
        }
    }

    /// Handle display:contents nodes by reparenting their taffy children
    /// to the nearest non-display-contents ancestor in the taffy tree.
    ///
    /// This function is **idempotent**: it rebuilds the taffy children list from
    /// the DOM structure each time, so calling it multiple times produces the
    /// same result. Nested display:contents (e.g. from `else if` chains) are
    /// handled by recursively flattening.
    pub(crate) fn sync_display_contents(&mut self) {
        // Find all display:contents nodes and their nearest non-contents ancestors.
        // We rebuild the taffy children of each affected ancestor from scratch.
        let mut affected_parents: Vec<usize> = Vec::new();
        let mut all_contents_nodes: Vec<usize> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if let Some(style_str) = node.attributes.get("style")
                && layout::is_display_contents(style_str)
            {
                all_contents_nodes.push(id);

                // Walk up to find nearest non-display-contents ancestor
                let mut ancestor = node.parent;
                while let Some(anc_id) = ancestor {
                    let anc_is_contents = self.tree.nodes[anc_id]
                        .attributes
                        .get("style")
                        .map(|s| layout::is_display_contents(s))
                        .unwrap_or(false);
                    if !anc_is_contents {
                        if !affected_parents.contains(&anc_id) {
                            affected_parents.push(anc_id);
                        }
                        break;
                    }
                    ancestor = self.tree.nodes[anc_id].parent;
                }
            }
        }

        // For each affected parent, rebuild its taffy children by flattening
        // display:contents nodes recursively.
        for parent_id in affected_parents {
            let parent_taffy = match self.tree.nodes[parent_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            let new_children = Self::collect_effective_taffy_children(&self.tree.nodes, parent_id);
            let _ = self.tree.taffy.set_children(parent_taffy, &new_children);
        }

        // Set all display:contents nodes' taffy to display:none so they don't
        // participate in layout themselves.
        for node_id in all_contents_nodes {
            if let Some(node_taffy) = self.tree.nodes[node_id].taffy_id {
                let _ = self.tree.taffy.set_style(
                    node_taffy,
                    taffy::Style {
                        display: taffy::Display::None,
                        ..Default::default()
                    },
                );
            }
        }
    }

    /// Recursively collect the effective Taffy children for a node,
    /// flattening any `display:contents` children so their grandchildren
    /// appear directly in the parent's child list.
    fn collect_effective_taffy_children(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> Vec<taffy::NodeId> {
        let mut result = Vec::new();
        for &child_id in &nodes[node_id].children {
            let is_contents = nodes[child_id]
                .attributes
                .get("style")
                .map(|s| layout::is_display_contents(s))
                .unwrap_or(false);

            if is_contents {
                // Recursively flatten: add grandchildren directly
                result.extend(Self::collect_effective_taffy_children(nodes, child_id));
            } else if let Some(child_taffy) = nodes[child_id].taffy_id {
                result.push(child_taffy);
            }
        }
        result
    }

    /// Invalidate the IFC that owns a node (if any).
    ///
    /// Clears the IFC root's cached text_layout so it rebuilds on next layout pass.
    /// Also checks the parent's text_layout as a fallback when ifc_root hasn't been
    /// set yet (before the first layout pass).
    pub(crate) fn invalidate_ifc_for_node(&mut self, node_id: usize) {
        if let Some(ifc_root_id) = self.tree.nodes.get(node_id).and_then(|n| n.ifc_root) {
            if let Some(root) = self.tree.nodes.get_mut(ifc_root_id) {
                root.text_layout = None;
            }
            // Invalidate IFC measure cache so style changes (e.g., font-size) trigger re-measurement
            self.tree.dirty_ifc_text_roots.insert(ifc_root_id);
            self.tree
                .ifc_measure_cache
                .retain(|&(root_id, _), _| root_id != ifc_root_id);
        } else if self
            .tree
            .nodes
            .get(node_id)
            .map(|n| n.text_layout.is_some())
            .unwrap_or(false)
        {
            // The node itself IS the IFC root (block element containing inline text)
            self.tree.nodes[node_id].text_layout = None;
            self.tree.dirty_ifc_text_roots.insert(node_id);
            self.tree
                .ifc_measure_cache
                .retain(|&(root_id, _), _| root_id != node_id);
            // Mark Taffy dirty so it re-measures this node (triggering IFC rebuild)
            if let Some(taffy_id) = self.tree.nodes.get(node_id).and_then(|n| n.taffy_id) {
                let _ = self.tree.taffy.mark_dirty(taffy_id);
            }
        } else {
            // Fallback: walk ancestors to find one with text_layout (the IFC root)
            let mut cur = self.tree.nodes.get(node_id).and_then(|n| n.parent);
            while let Some(pid) = cur {
                if self
                    .tree
                    .nodes
                    .get(pid)
                    .map(|p| p.text_layout.is_some())
                    .unwrap_or(false)
                {
                    self.tree.nodes[pid].text_layout = None;
                    // Invalidate IFC measure cache for this root
                    self.tree.dirty_ifc_text_roots.insert(pid);
                    self.tree
                        .ifc_measure_cache
                        .retain(|&(root_id, _), _| root_id != pid);
                    break;
                }
                cur = self.tree.nodes.get(pid).and_then(|n| n.parent);
            }
        }
    }

    /// Safely remove a child from a Taffy parent, checking membership first.
    /// Taffy's `remove_child` panics if the child isn't actually a child of the parent,
    /// which can happen when inline children were detached by `setup_inline_formatting_contexts`.
    pub(crate) fn taffy_remove_child_safe(
        &mut self,
        parent_taffy: taffy::NodeId,
        child_taffy: taffy::NodeId,
    ) {
        if let Ok(children) = self.tree.taffy.children(parent_taffy)
            && children.contains(&child_taffy)
        {
            let _ = self.tree.taffy.remove_child(parent_taffy, child_taffy);
        }
    }

    /// Clear ifc_root on a node and all its descendants.
    pub(crate) fn clear_ifc_root_recursive(&mut self, node_id: usize) {
        // Use iterative approach to avoid stack overflow
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.ifc_root = None;
                stack.extend(node.children.iter().copied());
            }
        }
    }

    /// Invalidate IFC state for a parent element.
    /// Clears text_layout on the parent and ifc_root on all its inline children.
    /// Also marks the Taffy node dirty so the measure callback re-fires.
    pub(crate) fn invalidate_parent_ifc(&mut self, parent_id: usize) {
        if let Some(parent) = self.tree.nodes.get_mut(parent_id) {
            parent.text_layout = None;
        }
        // Invalidate IFC measure cache so style changes trigger re-measurement
        self.tree.dirty_ifc_text_roots.insert(parent_id);
        self.tree
            .ifc_measure_cache
            .retain(|&(root_id, _), _| root_id != parent_id);
        if let Some(taffy_id) = self.tree.nodes.get(parent_id).and_then(|n| n.taffy_id) {
            let _ = self.tree.taffy.mark_dirty(taffy_id);
        }
        // NOTE: Do NOT clear ifc_root on children here. This function handles
        // text/style invalidation where the IFC structure is unchanged. Clearing
        // ifc_root would prevent build_ifc_layouts() from finding this IFC root
        // (it discovers roots by checking child.ifc_root == Some(parent_id)),
        // and setup_inline_formatting_contexts() won't re-assign them because
        // ifc_dirty is not set for text-only changes.
    }
}
