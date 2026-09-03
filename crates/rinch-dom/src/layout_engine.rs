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
        let t0 = web_time::Instant::now();

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
            // The viewport IS the root's available space. A size change must
            // force a Taffy recompute even when no node's Taffy *style* changed
            // (e.g. an all-`auto`/fixed tree): otherwise auto-sized content stays
            // laid out at the previous viewport width. Without this, the early
            // `if !layout_dirty { return }` below strands the tree at its old
            // size — visible as prose that keeps a narrow first-layout width
            // (often min-content) after the window grows.
            self.tree.layout_dirty = true;
        }

        // Drain completed image loads and update intrinsic dimensions.
        //
        // A newly decoded image changes a Taffy node's *context*, not its Taffy
        // style, so the `mark_dirty` inside `drain_pending_images` is invisible
        // to the `if !layout_dirty { return }` below and the whole compute is
        // skipped — leaving the `<img>` at the 0x0 intrinsic size it was
        // created with, laid out as nothing and painted as nothing, however
        // many frames follow. That is the same class of miss the viewport
        // branch above records, and an image landing is the other thing that
        // needs a recompute without any style having changed. (The drain
        // answers `false` for a `background-image`, which changes no box — it
        // publishes those by marking their users paint-dirty instead.)
        if self.drain_pending_images() {
            self.tree.layout_dirty = true;
        }

        // Resolve Stylo styles and apply to Taffy nodes (only if dirty)
        if self.tree.styles_dirty {
            let t = web_time::Instant::now();
            self.resolve_styles();
            if perf {
                eprintln!(
                    "  [PERF] resolve_styles: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }
            let t = web_time::Instant::now();
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
                let t = web_time::Instant::now();
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
            // Structural change — invalidate all cached IFC measures and
            // clear dirty_ifc_text_roots so build_ifc_layouts rebuilds ALL
            // IFC roots. Without this, stale entries from set_text_content
            // calls during rendering (before setup_inline_formatting_contexts
            // assigns correct ifc_root values) cause build_ifc_layouts to
            // skip newly created IFC roots — making their text invisible.
            self.tree.ifc_measure_cache.clear();
            self.tree.dirty_ifc_text_roots.clear();

            // Handle display:contents by rebuilding taffy children for affected nodes
            let t = web_time::Instant::now();
            self.sync_display_contents();
            if perf {
                eprintln!(
                    "  [PERF] sync_display_contents: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Detect and set up inline formatting contexts
            let t = web_time::Instant::now();
            self.setup_inline_formatting_contexts();
            if perf {
                eprintln!(
                    "  [PERF] setup_ifc: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Pre-compute layout for inline-block children that were detached from Taffy.
            // They need their own subtree measured so walk_inline_children can read dimensions.
            let t = web_time::Instant::now();
            self.compute_inline_block_layouts();
            if perf {
                eprintln!(
                    "  [PERF] inline_block_layouts: {:.2}ms",
                    t.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Sync font-size from parent elements to text node contexts
            let t = web_time::Instant::now();
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
            let t = web_time::Instant::now();
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

        let mut text_layout_cache = self.run_taffy_compute(root_taffy, available_space, perf);

        // #120: an inline-block with a percentage main size is pre-measured detached
        // from Taffy under `MaxContent` (see `compute_inline_block_layouts`), where it
        // has no containing block to resolve the percentage against — so it collapses
        // to min-content. Its containing block only has a width once the compute above
        // has run. Re-measure those inline-blocks against that width now, and if any
        // changed size, re-run the compute so the enclosing IFCs line-break against the
        // corrected boxes. Costs nothing when no percentage inline-block exists.
        if self.resolve_percentage_inline_blocks() {
            text_layout_cache = self.run_taffy_compute(root_taffy, available_space, perf);
        }

        // #278: a mixed `calc(%, px)` value has no Taffy representation (see
        // `calc_layout.rs`), so its style carries a seed until the containing
        // block has a size. Resolve every such value against the sizes the
        // compute above produced and re-run until nothing moves — a calc
        // container whose child is calc-sized converges one level per pass.
        // On the converged path no layout result is read (and nothing
        // painted) from a seed value. Percentage cycles are not what the cap
        // is for — those are broken the way browsers and Taffy break them,
        // by resolving a percentage against an *indefinite* basis as
        // zero/auto (`calc_axis_definite`). The cap bounds the residual
        // content-feedback corner (e.g. `min-size: auto` growing a nominally
        // definite axis): a capped run lays out from the last iterate — a
        // wrong but bounded answer after 8 extra computes — and says so on
        // stderr once per process rather than hiding it.
        let mut calc_passes = 0;
        while self.resolve_layout_calcs() {
            text_layout_cache = self.run_taffy_compute(root_taffy, available_space, perf);
            calc_passes += 1;
            if calc_passes >= 8 {
                static CAP_WARNING: std::sync::Once = std::sync::Once::new();
                CAP_WARNING.call_once(|| {
                    eprintln!(
                        "[rinch] calc() layout fixpoint hit its iteration cap; a mixed                          calc() in this document is feeding back into its own basis and                          its layout is approximate (reported once per process)"
                    );
                });
                break;
            }
        }

        // Read layout results back into nodes
        let t = web_time::Instant::now();
        self.read_layout_results(self.tree.root_id);
        if perf {
            eprintln!(
                "  [PERF] read_layout: {:.2}ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }

        // Clamp scroll offsets to valid range after layout.
        // When a scroll container shrinks (e.g., window resize makes max-height
        // smaller) or grows (content now fits), the old scroll offset may exceed
        // the new max. Clamping here ensures paint and hit-testing use valid values.
        self.clamp_scroll_offsets();

        // Build inline layouts for IFC roots (rebuild with final widths and store)
        // Temporarily take layout_cx out to avoid borrow conflict
        let t = web_time::Instant::now();
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
        let t = web_time::Instant::now();
        self.copy_cached_text_layouts(text_layout_cache);
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

    /// Run the root Taffy compute with the Parley measure function.
    ///
    /// Returns the text layouts built during measurement, keyed by
    /// `(node_id, wrap_width_bits)`, so paint can reuse the exact layouts that
    /// measurement produced. Safe to call more than once per frame — see the
    /// percentage inline-block second pass in `resolve_layout`.
    fn run_taffy_compute(
        &mut self,
        root_taffy: taffy::NodeId,
        available_space: taffy::Size<taffy::AvailableSpace>,
        perf: bool,
    ) -> HashMap<(usize, u32), parley::layout::Layout<Brush>> {
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

        let t = web_time::Instant::now();
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

                            // Skip Parley measurement for text in collapsed blocks
                            if nodes[text.node_id].estimated_height.is_some()
                                || nodes[text.node_id]
                                    .parent
                                    .is_some_and(|p| nodes[p].estimated_height.is_some())
                            {
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

                            // Collapsed block (virtualized) — return estimated size
                            // without doing any Parley work.
                            //
                            // This early return only runs at all because the
                            // node is a Taffy leaf — Taffy never consults a
                            // measure function on a node with children (the
                            // IFC leaf invariant, #466; see
                            // `NodeContext::InlineRoot`). A non-leaf
                            // virtualized root would silently get 0 from the
                            // block algorithm instead of its estimate.
                            if let Some(est_h) = nodes[root_id].estimated_height {
                                return taffy::Size {
                                    width: known_dims.width.unwrap_or(0.0),
                                    height: known_dims.height.unwrap_or(est_h),
                                };
                            }

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

        text_layout_cache.into_inner()
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

    /// Clamp scroll offsets for all scroll containers to their valid range.
    /// After layout changes (e.g., viewport resize), a container's content or
    /// visible area may have changed, making the old scroll offset too large.
    fn clamp_scroll_offsets(&mut self) {
        use crate::computed_style::OverflowValue;
        // Collect (node_id, max_scroll) for nodes that need clamping
        let mut clamps: Vec<(usize, f64)> = Vec::new();
        for (node_id, _) in self.tree.nodes.iter() {
            let node = &self.tree.nodes[node_id];
            if !matches!(
                node.computed_style.overflow_y,
                OverflowValue::Auto | OverflowValue::Scroll
            ) {
                continue;
            }
            if node.scroll_offset == (0.0, 0.0) {
                continue;
            }
            let cs = &node.computed_style;
            let content_top = (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64;
            let mut content_height: f64 = 0.0;
            for &child_id in &node.children {
                if let Some(child) = self.tree.nodes.get(child_id) {
                    let bottom = (child.layout.y + child.layout.height) as f64 - content_top;
                    if bottom > content_height {
                        content_height = bottom;
                    }
                }
            }
            let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64;
            let border_v = (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64;
            let visible_h = (node.layout.height as f64 - pad_v - border_v).max(0.0);
            let max_scroll = (content_height - visible_h).max(0.0);
            if node.scroll_offset.1 > max_scroll {
                clamps.push((node_id, max_scroll));
            }
        }
        for (node_id, max_scroll) in clamps {
            self.tree.nodes[node_id].scroll_offset.1 = max_scroll;
            // Queue a deferred scroll notification so the clamp isn't a silent
            // mutation (#144). Coalesce per node (last value wins): layout can
            // resolve more than once per frame, and a consumer must see one
            // event per drain.
            if let Some(pending) = self
                .tree
                .pending_scroll_clamps
                .iter_mut()
                .find(|(id, _)| *id == node_id)
            {
                pending.1 = max_scroll;
            } else {
                self.tree.pending_scroll_clamps.push((node_id, max_scroll));
            }
        }
    }

    /// Recursively read Taffy layout results into node LayoutResult fields.
    pub(crate) fn read_layout_results(&mut self, node_id: usize) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        // A `display: contents` element generates no box. Force its layout to the
        // origin so every parent-chain accumulation (paint tree-walk, hit-testing,
        // compute_absolute_position) treats it as fully transparent — its
        // children's positions are already relative to the nearest real ancestor.
        // `sync_display_contents` detaches the wrapper's Taffy node and marks it
        // display:none; that detached node can retain a stale non-zero `location`
        // (or make `taffy.layout()` return `Err`, skipping the read below), either
        // of which would otherwise be double-counted onto every descendant.
        if self.tree.nodes[node_id].computed_style.display
            == crate::computed_style::DisplayValue::Contents
        {
            let node = &mut self.tree.nodes[node_id];
            let zero = LayoutResult::default();
            if node.layout != zero {
                node.prev_layout = node.layout;
                node.layout = zero;
                self.tree.paint_dirty_nodes.push(node_id);
            }
            for child_id in children {
                self.read_layout_results(child_id);
            }
            return;
        }

        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id
            && let Ok(taffy_layout) = self.tree.taffy.layout(taffy_id)
        {
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
            {
                let node = &self.tree.nodes[node_id];
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
                    let width_auto = style.width.is_auto();
                    let height_auto = style.height.is_auto();

                    // Horizontal positioning
                    if let (Some(l), Some(r)) = (left, right) {
                        new_layout.x = l;
                        if width_auto {
                            new_layout.width = (vw - l - r).max(0.0);
                        }
                    } else if let Some(l) = left {
                        new_layout.x = l;
                    } else if let Some(r) = right {
                        new_layout.x = (vw - new_layout.width - r).max(0.0);
                    } else {
                        new_layout.x = 0.0;
                    }

                    // Vertical positioning
                    if let (Some(t), Some(b)) = (top, bottom) {
                        new_layout.y = t;
                        if height_auto {
                            new_layout.height = (vh - t - b).max(0.0);
                        }
                    } else if let Some(t) = top {
                        new_layout.y = t;
                        // Taffy may compute wrong height for fixed elements
                        // (their Taffy parent differs from the CSS containing
                        // block which should be the viewport).  Re-derive
                        // content height from children.
                        if height_auto {
                            let _ = node;
                            new_layout.height = self.compute_content_height(node_id, &new_layout);
                        }
                    } else if let Some(b) = bottom {
                        if height_auto {
                            let _ = node;
                            new_layout.height = self.compute_content_height(node_id, &new_layout);
                        }
                        new_layout.y = (vh - new_layout.height - b).max(0.0);
                    } else {
                        new_layout.y = 0.0;
                    }
                }
            }

            // An absolutely positioned box with no positioned ancestor resolves
            // against the initial containing block — the viewport at the origin
            // — not against its direct parent, which is the only containing
            // block Taffy knows (issue #204). Its *size* was already baked from
            // the viewport before layout (`out_of_flow`); this places it.
            //
            // The correction is written as a **parent-relative delta**, not as
            // a viewport-absolute coordinate the way `fixed` above is: with
            //     abs(node) = layout.x + abs(parent) - parent.scroll_offset.x
            // writing `target - abs(parent) + parent.scroll_offset.x` leaves
            // `LayoutResult` parent-relative, so every coordinate walk in the
            // codebase — paint, stacking, hit testing, ClickContext, the MCP
            // `absolute` contract — keeps working untouched, and layout agrees
            // with paint by construction because it reuses paint's own sum.
            {
                let node = &self.tree.nodes[node_id];
                if node.computed_style.position == crate::computed_style::PositionValue::Absolute
                    && (new_layout.width > 0.0 || new_layout.height > 0.0)
                    && crate::out_of_flow::out_of_flow_kind(&self.tree, node_id)
                        == Some(crate::out_of_flow::OutOfFlowKind::IcbAbsolute)
                {
                    let vw = self.tree.viewport.width;
                    let vh = self.tree.viewport.height;
                    let (parent_abs, parent_scroll) = match node.parent {
                        Some(parent_id) => {
                            let (px, py) =
                                crate::paint::compute_absolute_position(&self.tree, parent_id, 1.0);
                            let scroll = self.tree.nodes[parent_id].scroll_offset;
                            ((px as f32, py as f32), (scroll.0 as f32, scroll.1 as f32))
                        }
                        None => ((0.0, 0.0), (0.0, 0.0)),
                    };

                    let style = &node.computed_style;
                    let left = style.left.resolve(vw);
                    let right = style.right.resolve(vw);
                    let top = style.top.resolve(vh);
                    let bottom = style.bottom.resolve(vh);
                    // Percentage margins resolve against the containing block's
                    // *width* on both axes, per CSS.
                    let margin_left = style.margin_left.resolve(vw).unwrap_or(0.0);
                    let margin_right = style.margin_right.resolve(vw).unwrap_or(0.0);
                    let margin_top = style.margin_top.resolve(vw).unwrap_or(0.0);
                    let margin_bottom = style.margin_bottom.resolve(vw).unwrap_or(0.0);

                    // Only correct an axis that has a real inset. With both
                    // insets `auto` the target is `None` and the box keeps
                    // Taffy's static position — which CSS *does* take from the
                    // flow position in the DOM parent, so Taffy's answer is the
                    // right one there.
                    let target_x = match (left, right) {
                        (Some(l), _) => Some(l + margin_left),
                        (None, Some(r)) => Some(vw - r - margin_right - new_layout.width),
                        (None, None) => None,
                    };
                    if let Some(x) = target_x {
                        new_layout.x = x - parent_abs.0 + parent_scroll.0;
                    }
                    let target_y = match (top, bottom) {
                        (Some(t), _) => Some(t + margin_top),
                        (None, Some(b)) => Some(vh - b - margin_bottom - new_layout.height),
                        (None, None) => None,
                    };
                    if let Some(y) = target_y {
                        new_layout.y = y - parent_abs.1 + parent_scroll.1;
                    }
                }
            }

            // An inline-block child of an IFC has its *position* assigned by the IFC
            // (`write_inline_positions`), not Taffy: it is detached from its parent's
            // Taffy tree and measured standalone (Taffy location 0,0). Keep the IFC's
            // x/y here — only the size comes from the standalone measure. Without
            // this, a non-structural re-layout (which doesn't rebuild the IFC) snaps
            // every inline-block back to the line origin, collapsing e.g. a row of
            // inline-block buttons into a pile.
            {
                let node = &self.tree.nodes[node_id];
                if node.display_mode == crate::node::DisplayMode::InlineBlock
                    && node.ifc_root.is_some()
                {
                    new_layout.x = node.layout.x;
                    new_layout.y = node.layout.y;
                }
            }

            // Save previous layout for dirty region computation
            let node = &mut self.tree.nodes[node_id];
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

    /// Compute the intrinsic content height of a node from its children's
    /// Taffy-computed sizes. Used for position:fixed elements where Taffy's
    /// parent-relative sizing gives the wrong result.
    fn compute_content_height(&self, node_id: usize, _parent_layout: &LayoutResult) -> f32 {
        let node = &self.tree.nodes[node_id];
        let pad_top = node.computed_style.padding_top.to_px();
        let pad_bottom = node.computed_style.padding_bottom.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        let border_bottom = node.computed_style.border_bottom_width.to_px();
        let gap = node.computed_style.gap_row.to_px();
        // Children stack vertically (heights sum) when the container is a block
        // box or a flex column; a flex row lays them side by side (take the max).
        // Without the `Block` case a `position: fixed` block with auto height —
        // e.g. a popup/menu appended to <body> — collapses to one child's height.
        let is_column = matches!(
            node.computed_style.display,
            crate::computed_style::DisplayValue::Block
        ) || matches!(
            node.computed_style.flex_direction,
            crate::computed_style::FlexDirectionValue::Column
                | crate::computed_style::FlexDirectionValue::ColumnReverse
        );

        let mut content_h: f32 = 0.0;
        let child_count = node.children.len();

        for (i, &child_id) in node.children.iter().enumerate() {
            if let Some(child_taffy) = self.tree.nodes.get(child_id).and_then(|c| c.taffy_id) {
                if let Ok(child_layout) = self.tree.taffy.layout(child_taffy) {
                    let ch = child_layout.size.height;
                    if is_column {
                        content_h += ch;
                        if i > 0 && i < child_count {
                            content_h += gap;
                        }
                    } else {
                        content_h = content_h.max(ch);
                    }
                }
            }
        }

        let mut h = content_h + pad_top + pad_bottom + border_top + border_bottom;

        // Respect the element's own min/max-height (border-box, like the rest of
        // the box model here) so a fixed scroll container clamps and scrolls its
        // overflow instead of growing past its cap.
        use crate::computed_style::DimensionValue;
        let vh = self.tree.viewport.height;
        let resolve = |d: &DimensionValue| -> Option<f32> {
            match d {
                DimensionValue::Length(v) => Some(*v),
                DimensionValue::Percent(p) => Some(p * vh),
                DimensionValue::Calc { px, pct } => Some(px + pct * vh),
                DimensionValue::Auto => None,
            }
        };
        if let Some(max_h) = resolve(&node.computed_style.max_height) {
            h = h.min(max_h);
        }
        if let Some(min_h) = resolve(&node.computed_style.min_height) {
            h = h.max(min_h);
        }
        h
    }

    /// Handle display:contents nodes by reparenting their taffy children
    /// to the nearest non-display-contents ancestor in the taffy tree.
    ///
    /// This function is **idempotent**: it rebuilds the taffy children list from
    /// the DOM structure each time, so calling it multiple times produces the
    /// same result. Nested display:contents (e.g. from `else if` chains) are
    /// handled by recursively flattening.
    pub(crate) fn sync_display_contents(&mut self) {
        use crate::computed_style::values::DisplayValue;

        // Find all display:contents nodes and their nearest non-contents ancestors.
        // We rebuild the taffy children of each affected ancestor from scratch.
        //
        // The check uses the resolved `computed_style.display`, not the raw inline
        // `style` attribute, so that `display: contents` set via a CSS class (e.g.
        // `.rinch-context-menu { display: contents }`) is treated the same as the
        // inline form. Without this, class-based contents elements stay in the
        // Taffy tree as ordinary boxes and trap their children inside a 0×0 (or
        // 2×2 text-sized) parent — see issue #25.
        let mut affected_parents: Vec<usize> = Vec::new();
        let mut all_contents_nodes: Vec<usize> = Vec::new();

        for (id, node) in &self.tree.nodes {
            if node.computed_style.display == DisplayValue::Contents {
                all_contents_nodes.push(id);

                // Walk up to find nearest non-display-contents ancestor
                let mut ancestor = node.parent;
                while let Some(anc_id) = ancestor {
                    let anc_is_contents =
                        self.tree.nodes[anc_id].computed_style.display == DisplayValue::Contents;
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
        for parent_id in &affected_parents {
            let parent_taffy = match self.tree.nodes[*parent_id].taffy_id {
                Some(t) => t,
                None => continue,
            };

            let new_children = Self::collect_effective_taffy_children(&self.tree.nodes, *parent_id);
            let _ = self.tree.taffy.set_children(parent_taffy, &new_children);

            // Also mark each reparented child dirty so their own caches are
            // cleared (they may have stale entries from their old position).
            for &child_taffy in &new_children {
                let _ = self.tree.taffy.mark_dirty(child_taffy);
            }
        }

        // Force the root Taffy node dirty to guarantee a full recompute.
        // Taffy's mark_dirty propagation stops at already-empty ancestors,
        // which can leave stale cached layouts when available space hasn't
        // changed but children have been swapped.
        if !affected_parents.is_empty() {
            if let Some(root_taffy) = self.tree.nodes[self.tree.root_id].taffy_id {
                let _ = self.tree.taffy.mark_dirty(root_taffy);
            }
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
    ///
    /// `pub(crate)`, not private, because `remove_node` (in
    /// `dom_impl::dom_document_impl`) needs this same flattening on the
    /// removal path — see the comment there (card K48).
    pub(crate) fn collect_effective_taffy_children(
        nodes: &slab::Slab<crate::node::Node>,
        node_id: usize,
    ) -> Vec<taffy::NodeId> {
        use crate::computed_style::values::DisplayValue;

        let mut result = Vec::new();
        for &child_id in &nodes[node_id].children {
            let is_contents = nodes[child_id].computed_style.display == DisplayValue::Contents;

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
            // The measure may live on this root's measure leaf, which a mark
            // on the root does not reach — dirty propagates up, not down (#466).
            self.mark_ifc_measure_dirty(node_id);
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
        // The measure may live on this root's measure leaf, which the mark
        // above does not reach — dirty propagates up, not down (#466). Without
        // this, a text edit in a `text + absolute` container serves the leaf's
        // cached measure and the container's height never changes.
        self.mark_ifc_measure_dirty(parent_id);
        // NOTE: Do NOT clear ifc_root on children here. This function handles
        // text/style invalidation where the IFC structure is unchanged. Clearing
        // ifc_root would prevent build_ifc_layouts() from finding this IFC root
        // (it discovers roots by checking child.ifc_root == Some(parent_id)),
        // and setup_inline_formatting_contexts() won't re-assign them because
        // ifc_dirty is not set for text-only changes.
    }
}
