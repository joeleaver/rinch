//! DomDocument implementation for rinch-dom.

use std::collections::HashMap;

use peniko::Brush;
use servo_arc::Arc as ServoArc;

// Stylo CSS engine imports
use euclid::Scale;
use style::context::QuirksMode;
use style::font_metrics::FontMetrics;
use style::media_queries::{Device, MediaType};
use style::properties::style_structs::Font as StyloFont;
use style::properties::{ComputedValues, PropertyDeclarationBlock};
use style::queries::values::PrefersColorScheme;
use style::stylist::Stylist;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, Length};
use style::values::specified::font::QueryFontMetricsFlags;
use stylo_config as style_config;
// CSSPixel and DevicePixel are used via euclid::Size2D type parameters

use crate::node::{DirtyFlags, NodeTree};

mod dom_document_impl;

/// A simple FontMetricsProvider that returns default/fixed values.
/// This is used by the Stylist's Device to resolve font-relative units.
#[derive(Debug)]
pub(crate) struct SimpleFontMetricsProvider;

impl style::servo::media_queries::FontMetricsProvider for SimpleFontMetricsProvider {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        _font: &StyloFont,
        _base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        // Return sensible defaults - these will be used for font-relative units
        // like ex, ch, cap, ic when we don't have actual font metrics
        FontMetrics::default()
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        // Default base font size (16px for most generics)
        Length::new(16.0)
    }
}

/// The primary document type for rinch-dom.
///
/// Implements [`DomDocument`] using a slab-allocated node tree.
/// In later phases, this will integrate Taffy for layout,
/// Parley for text, and Vello for painting.
pub struct RinchDocument {
    /// Process-unique document identity (see [`DomDocument::doc_key`]) — scopes
    /// per-node-id state in thread-local registries so two documents on one
    /// thread never collide (issue #134).
    pub(crate) doc_key: u64,
    /// The node tree.
    pub tree: NodeTree,
    /// Parley font context for text shaping.
    pub font_cx: parley::FontContext,
    /// Parley layout context for text measurement.
    pub layout_cx: parley::LayoutContext<Brush>,
    /// Stylo CSS engine stylist for CSS cascade and selector matching.
    pub stylist: Stylist,
    /// The theme stylesheet, held in a stable slot before every app sheet so app
    /// CSS always cascades over it. Managed by `set_theme_css`.
    pub(crate) theme_stylesheet: Option<style::stylesheets::DocumentStyleSheet>,
    /// App author stylesheets, in insertion (source) order.
    pub(crate) author_stylesheets: Vec<style::stylesheets::DocumentStyleSheet>,
    /// Whether any loaded stylesheet has a rule whose rightmost compound is a
    /// bare focus pseudo-class (`:focus` / `:focus-visible` / `:focus-within`
    /// with no tag/class/id/attribute anchor) — e.g. the theme's
    /// `:focus-visible { outline: ... }`. Stylo buckets such rules into a
    /// state-gated `rare_pseudo_classes` map that is only consulted while the
    /// element ALREADY has focus state, so `focus_sensitive` can never be set
    /// on an unfocused node by them; focus changes must then invalidate
    /// unconditionally (see `node_is_focus_sensitive`). Recomputed whenever a
    /// stylesheet is loaded or the theme sheet is replaced.
    pub(crate) has_bare_focus_rules: bool,
}

impl Default for RinchDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RinchDocument {
    fn drop(&mut self) {
        // A torn-down document can never drain its queued image decodes —
        // purge them so they don't strand in the process-global pending
        // queue forever (issue #137).
        crate::image_cache::purge_pending(self.doc_key);
    }
}

impl RinchDocument {
    /// Process-unique document identity (see
    /// [`DomDocument::doc_key`](rinch_core::dom::DomDocument::doc_key)) —
    /// inherent accessor so callers holding a concrete `RinchDocument` don't
    /// need the trait in scope.
    pub fn doc_key(&self) -> u64 {
        self.doc_key
    }

    /// Create a new document with root and body nodes.
    pub fn new() -> Self {
        // Enable CSS Grid and text-overflow support in Stylo
        // This must be called before any CSS parsing happens
        style_config::set_bool("layout.grid.enabled", true);
        style_config::set_bool("layout.unimplemented", true);

        // Create the Stylo Device with default viewport and settings
        let viewport_size = euclid::Size2D::new(800.0, 600.0);
        let device_pixel_ratio = Scale::new(1.0);
        let font_metrics_provider = Box::new(SimpleFontMetricsProvider);
        let default_font = StyloFont::initial_values();
        let default_computed_values =
            ComputedValues::initial_values_with_font_override(default_font);

        let device = Device::new(
            MediaType::screen(),
            QuirksMode::NoQuirks,
            viewport_size,
            device_pixel_ratio,
            font_metrics_provider,
            default_computed_values,
            PrefersColorScheme::Light,
        );

        let stylist = Stylist::new(device, QuirksMode::NoQuirks);

        let mut doc = Self {
            doc_key: rinch_core::dom::next_doc_key(),
            tree: NodeTree::new(),
            font_cx: crate::fonts::new_font_context(),
            layout_cx: parley::LayoutContext::new(),
            stylist,
            theme_stylesheet: None,
            author_stylesheets: Vec::new(),
            has_bare_focus_rules: false,
        };

        // Set up default file-based image loader
        doc.tree.image_loader = Some(std::sync::Arc::new(crate::image_cache::FileImageLoader));

        // Load User-Agent stylesheet with default display values for HTML elements
        doc.load_ua_stylesheet();

        doc
    }

    /// Load the User-Agent stylesheet with default display values for HTML elements.
    /// Without this, all elements default to display: inline in Stylo.
    fn load_ua_stylesheet(&mut self) {
        use style::media_queries::MediaList;
        use style::stylesheets::{
            AllowImportRules, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
        };

        // Basic UA stylesheet defining block-level elements
        // Note: Stylo's initial border-width is 'medium' (3px), so we reset it to 0
        let ua_css = r#"
            * {
                border-width: 0;
            }

            html, body, div, section, article, aside, header, footer, main, nav,
            h1, h2, h3, h4, h5, h6, p, blockquote, pre, figure, figcaption,
            ul, ol, li, dl, dt, dd, table, form, fieldset, legend, hr,
            address, details, summary {
                display: block;
            }

            head, style, script, link, meta, title, noscript {
                display: none;
            }

            span, a, em, strong, b, i, u, s, sub, sup, small, mark, abbr, cite,
            code, kbd, samp, var, q, dfn, time, label, br, wbr {
                display: inline;
            }

            strong, b {
                font-weight: bold;
            }

            em, i {
                font-style: italic;
            }

            u, ins {
                text-decoration-line: underline;
            }

            s, strike, del {
                text-decoration-line: line-through;
            }

            img, input, button, select, textarea {
                display: inline-block;
            }

            /* A closed <select> shows the selected option's label (painted by the
               backend) plus a dropdown arrow — its <option>/<optgroup> children are
               not laid out. Reserve room on the right for the arrow, and a
               min-width so an unstyled control doesn't collapse to its padding.
               The interactive popup is drawn by the app/shell layer (issue #121). */
            option, optgroup {
                display: none;
            }

            select {
                padding: 4px 24px 4px 8px;
                min-width: 60px;
                white-space: nowrap;
                overflow: hidden;
            }

            /* Default list indentation (matches browser default) */
            ul, ol {
                padding-left: 40px;
            }

            /* Default body margin - set to 0 for GUI apps */
            /* overflow-y: auto enables viewport scrolling like browsers */
            body {
                margin: 0;
                overflow-y: auto;
            }

            /* A `data-viewport` node is a compositing hole: the game/video frame
               shows through it (find_viewport_rects punches the background) and a
               pointer landing on it belongs to whatever renders into it, not to
               the rinch UI. That routing is decided by hit-testing the hole, so
               the hole must be HITTABLE — `pointer-events: none` on it makes the
               UI claim the mouse everywhere instead (issue #207). Declaring it
               here also beats a `pointer-events: none` inherited from a HUD root
               (issue #195) while still losing to any author declaration on the
               hole itself, so an app can still opt out. */
            [data-viewport] {
                pointer-events: auto;
                background: transparent;
            }
        "#;

        let url_data = UrlExtraData::from(url::Url::parse("about:ua-stylesheet").unwrap());
        let media = ServoArc::new(self.tree.guard.wrap(MediaList::empty()));

        let stylesheet = Stylesheet::from_str(
            ua_css,
            url_data,
            Origin::UserAgent, // Use UserAgent origin for lowest priority
            media,
            self.tree.guard.clone(),
            None, // stylesheet_loader
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::No,
        );

        let doc_stylesheet = DocumentStyleSheet(ServoArc::new(stylesheet));
        let guard = self.tree.guard.read();
        self.stylist.append_stylesheet(doc_stylesheet, &guard);
        self.stylist
            .force_stylesheet_origins_dirty(Origin::UserAgent.into());
    }

    /// Mark a node and its ancestors as needing layout.
    fn mark_dirty_up(&mut self, node_id: usize, flags: DirtyFlags) {
        let mut current = Some(node_id);
        while let Some(id) = current {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.dirty.insert(flags);
                current = node.parent;
            } else {
                break;
            }
        }
    }

    /// Push a node to the dirty list with layout+paint flags.
    fn push_dirty(&mut self, node_id: usize) {
        self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
    }

    /// Push a node to the dirty list with specific flags.
    pub(crate) fn push_dirty_flags(&mut self, node_id: usize, flags: DirtyFlags) {
        if self.tree.contains(node_id) {
            self.tree.nodes[node_id].dirty.insert(flags);
            self.tree.push_dirty(node_id);
            if flags.contains(DirtyFlags::LAYOUT) {
                // Don't set layout_dirty here — it's set by:
                // 1. apply_stylo_styles_to_taffy() when Taffy style actually changes
                // 2. Structural changes (append_child, remove_child, etc.)
                // This avoids full Taffy recompute for paint-only changes like transform.
                self.mark_dirty_up(node_id, DirtyFlags::LAYOUT);
            }
        }
    }

    /// Cache a parsed inline `style` block on the node for Stylo's next
    /// cascade of it. Pair with [`parse_inline_style`].
    pub(crate) fn cache_inline_style(&mut self, node_id: usize, pdb: PropertyDeclarationBlock) {
        self.tree.nodes[node_id].style_attribute_cache =
            Some(ServoArc::new(self.tree.guard.wrap(pdb)));
    }

    /// Mark a node and its entire subtree as paint-dirty for removal.
    ///
    /// Used before removing nodes so the dirty region includes the old layout
    /// positions, ensuring borders, backgrounds, and other visuals are cleared.
    /// Saves the absolute rect of each node because the nodes will be deleted
    /// from the tree before `compute_dirty_region` runs.
    pub(crate) fn mark_subtree_paint_dirty(&mut self, node_id: usize) {
        if self.tree.contains(node_id) {
            // Save the node's absolute rect before it's removed from the tree.
            // compute_dirty_region won't be able to look up deleted nodes.
            let node = &self.tree.nodes[node_id];
            let w = node.layout.width as f64;
            let h = node.layout.height as f64;
            if w > 0.0 && h > 0.0 {
                let (ax, ay) = crate::paint::compute_absolute_position(&self.tree, node_id, 1.0);
                self.tree.paint_dirty_removed_rects.push((ax, ay, w, h));
            }
            let children = self.tree.nodes[node_id].children.clone();
            for child_id in children {
                self.mark_subtree_paint_dirty(child_id);
            }
        }
    }

    /// Mark a node and its entire subtree as paint-dirty for insertion.
    ///
    /// Adds node IDs to `paint_dirty_nodes` so `compute_dirty_region` can
    /// read their rects after layout. Unlike `mark_subtree_paint_dirty`,
    /// this doesn't save absolute rects (the nodes haven't been laid out yet).
    pub(crate) fn mark_subtree_paint_dirty_ids(&mut self, node_id: usize) {
        if self.tree.contains(node_id) {
            self.tree.paint_dirty_nodes.push(node_id);
            let children = self.tree.nodes[node_id].children.clone();
            for child_id in children {
                self.mark_subtree_paint_dirty_ids(child_id);
            }
        }
    }

    /// Invalidate cached Stylo element data for all descendants of `node_id`.
    /// Used when a parent's class or interaction state changes, since descendant
    /// selectors (e.g. `.parent--active .child`) require descendants to be
    /// re-resolved against the updated ancestor.
    pub(crate) fn invalidate_descendant_styles(&mut self, node_id: usize) {
        // Collect children first to avoid borrow issues
        let children: Vec<usize> = self
            .tree
            .nodes
            .get(node_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();

        for child_id in children {
            if let Some(node) = self.tree.nodes.get_mut(child_id) {
                // Drop the cached match result so the descendant re-resolves against
                // the new ancestor state (class / attribute).
                *node.stylo_element_data.borrow_mut() = None;
                // …and mark it paint-dirty: a re-resolved style that isn't repainted
                // leaves the software renderer's dirty-region cache showing the stale
                // pixels (e.g. a dark-mode toggle re-colored the tree but only the
                // toggled node repainted). Layout is driven by the ancestor's LAYOUT
                // flag (set by the caller), so STYLE | PAINT suffices here.
                node.dirty.insert(DirtyFlags::STYLE | DirtyFlags::PAINT);
            }
            self.tree.dirty_nodes.insert(child_id);
            // Text color (and other inline properties) is baked into the cached
            // Parley `text_layout`; drop it so the descendant's text re-lays with the
            // re-resolved color rather than rendering the stale brush.
            self.invalidate_ifc_for_node(child_id);
            self.invalidate_descendant_styles(child_id);
        }
    }

    /// Advance all active CSS transitions by one frame.
    /// Returns true if any transitions are still active (caller should keep polling).
    pub fn tick_transitions(&mut self) -> bool {
        use web_time::SystemTime;
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        let any_active = crate::transition::tick_transitions(&mut self.tree, current_time_ms);

        // For layout-affecting transitions, we need to re-sync Taffy styles
        // from the updated computed_style values.
        // Collect nodes that had LAYOUT dirty set by tick_transitions.
        let layout_dirty: Vec<usize> = self
            .tree
            .active_transitions
            .keys()
            .copied()
            .chain(
                // Also check nodes that just had transitions complete
                self.tree.dirty_nodes.iter().copied(),
            )
            .collect();

        for node_id in layout_dirty {
            // Skip root and html nodes — their Taffy styles are manually set
            // during NodeTree construction and must not be overwritten from
            // computed_style (which has default width:auto instead of 100%).
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }
            if !self.tree.contains(node_id) {
                continue;
            }
            let node = &self.tree.nodes[node_id];
            if !node.dirty.contains(DirtyFlags::LAYOUT) {
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let dd = self.default_display_for_node(node_id);
                let mut taffy_style = node.computed_style.to_taffy_style(dd);

                // HTML element must fill the viewport and clip horizontal overflow
                if node_id == self.tree.html_id {
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                    if taffy_style.size.height == taffy::Dimension::auto() {
                        taffy_style.size.height = taffy::Dimension::percent(1.0);
                    }
                    if taffy_style.overflow.x == taffy::Overflow::Visible {
                        taffy_style.overflow.x = taffy::Overflow::Clip;
                    }
                }

                // Body node needs the same overrides as apply_stylo_styles_to_taffy
                if node_id == self.tree.body_id {
                    if taffy_style.flex_grow == 0.0 {
                        taffy_style.flex_grow = 1.0;
                    }
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                }

                // Collapsed block (virtualized contenteditable): keep the
                // estimated height apply_stylo_styles_to_taffy would have set.
                if let Some(est_h) = node.estimated_height {
                    taffy_style.size.height = taffy::Dimension::length(est_h);
                }

                // This rebuilds the Taffy style from the computed values, so it
                // drops the childless-block line floor exactly the way
                // apply_stylo_styles_to_taffy used to — re-apply it here or a
                // transition frame collapses a blockified `<input>` to nothing.
                crate::ifc::apply_empty_block_line_floor(node, &mut taffy_style);

                // Only call set_style if the Taffy style actually changed.
                // set_style() internally calls mark_dirty() which propagates up
                // the entire ancestor chain — unconditional calls here were causing
                // 70%+ of Taffy nodes to lose their cache on every frame with
                // active transitions, even when only paint-only properties changed.
                if let Ok(old_taffy_style) = self.tree.taffy.style(taffy_id) {
                    if old_taffy_style != &taffy_style {
                        let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    }
                } else {
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                }
            }
        }

        any_active
    }

    /// Advance all active CSS animations by one frame.
    /// Returns true if any animations are still active (caller should keep polling).
    pub fn tick_animations(&mut self) -> bool {
        use web_time::SystemTime;
        let current_time_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0;

        let any_active = crate::animation::tick_animations(&mut self.tree, current_time_ms);

        // For layout-affecting animations, re-sync Taffy styles.
        let layout_dirty: Vec<usize> = self
            .tree
            .active_animations
            .keys()
            .copied()
            .chain(self.tree.dirty_nodes.iter().copied())
            .collect();

        for node_id in layout_dirty {
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }
            if !self.tree.contains(node_id) {
                continue;
            }
            let node = &self.tree.nodes[node_id];
            if !node.dirty.contains(DirtyFlags::LAYOUT) {
                continue;
            }
            if let Some(taffy_id) = node.taffy_id {
                let dd = self.default_display_for_node(node_id);
                let mut taffy_style = node.computed_style.to_taffy_style(dd);

                // Body node needs the same overrides as apply_stylo_styles_to_taffy
                if node_id == self.tree.body_id {
                    if taffy_style.flex_grow == 0.0 {
                        taffy_style.flex_grow = 1.0;
                    }
                    if taffy_style.size.width == taffy::Dimension::auto() {
                        taffy_style.size.width = taffy::Dimension::percent(1.0);
                    }
                }

                // Collapsed block (virtualized contenteditable): keep the
                // estimated height apply_stylo_styles_to_taffy would have set.
                if let Some(est_h) = node.estimated_height {
                    taffy_style.size.height = taffy::Dimension::length(est_h);
                }

                // Same rebuild-from-computed-values hazard as tick_transitions:
                // without this an animation frame drops the childless-block line
                // floor and the element collapses to zero height.
                crate::ifc::apply_empty_block_line_floor(node, &mut taffy_style);

                if let Ok(old_taffy_style) = self.tree.taffy.style(taffy_id) {
                    if old_taffy_style != &taffy_style {
                        let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    }
                } else {
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                }
            }
        }

        any_active
    }

    /// Request an image load for a node's `src` attribute.
    ///
    /// If the image is already decoded in the cache, updates intrinsic dimensions
    /// immediately. Otherwise kicks off an async load on a background thread.
    pub(crate) fn request_image_load_for_node(&mut self, node_id: usize, src: &str) {
        if src.is_empty() {
            return;
        }

        // Data URIs are decoded synchronously and inserted directly into the
        // image cache (not the pending queue). This ensures that when a signal
        // triggers a re-render and creates a new img element, the cache lookup
        // in the "already in cache" path below finds the Decoded entry immediately.
        if src.starts_with("data:") {
            if !self.tree.image_cache.contains(src)
                && let Some(bytes) = crate::image_cache::decode_data_uri(src)
            {
                match image::load_from_memory(&bytes) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = (rgba.width(), rgba.height());
                        self.tree.image_cache.insert_decoded(
                            src.to_string(),
                            crate::image_cache::DecodedImage {
                                data: rgba.into_raw(),
                                width: w,
                                height: h,
                            },
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode data URI image: {}", e);
                    }
                }
            }
            // Fall through to the "already in cache" path to set NodeContext
            // dimensions on this node (works for both first render and re-renders).
        }

        // Check if already in cache
        if let Some(img) = self.tree.image_cache.get(src) {
            // Already decoded — update intrinsic dimensions on the Taffy node
            let (iw, ih) = (img.width, img.height);
            if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
                let _ = self.tree.taffy.set_node_context(
                    taffy_id,
                    Some(crate::node::NodeContext::Image {
                        src: src.to_string(),
                        width: iw,
                        height: ih,
                    }),
                );
                let _ = self.tree.taffy.mark_dirty(taffy_id);
            }
            self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            return;
        }

        // If already loading, don't re-request
        if self.tree.image_cache.contains(src) {
            return;
        }

        // Mark as loading and kick off background load
        let Some(loader) = self.tree.image_loader.clone() else {
            return;
        };

        self.tree.image_cache.mark_loading(src.to_string());

        // Update NodeContext with src (0x0 dims while loading)
        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
            let _ = self.tree.taffy.set_node_context(
                taffy_id,
                Some(crate::node::NodeContext::Image {
                    src: src.to_string(),
                    width: 0,
                    height: 0,
                }),
            );
        }

        // Kick off async load — result goes to PENDING_IMAGES static queue,
        // tagged with this document's identity (#137)
        crate::image_cache::request_image_load(self.doc_key, src.to_string(), loader);
    }

    /// Scan for background-image URLs that need loading and trigger async loads.
    pub fn request_background_image_loads(&mut self) {
        let Some(loader) = self.tree.image_loader.clone() else {
            return;
        };

        // Collect URLs that need loading
        let urls_to_load: Vec<String> = self
            .tree
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                if let crate::computed_style::BackgroundValue::Image { url } =
                    &node.computed_style.background
                    && !self.tree.image_cache.contains(url)
                {
                    return Some(url.clone());
                }
                None
            })
            .collect();

        for url in urls_to_load {
            self.tree.image_cache.mark_loading(url.clone());
            crate::image_cache::request_image_load(self.doc_key, url, loader.clone());
        }
    }

    /// Drain completed image loads and update Taffy nodes with intrinsic dimensions.
    ///
    /// Called before layout to pick up newly decoded images.
    /// Returns true if any images were newly decoded (needs re-layout).
    pub fn drain_pending_images(&mut self) -> bool {
        let newly_decoded = self.tree.image_cache.drain_pending(self.doc_key);
        if newly_decoded.is_empty() {
            return false;
        }
        // For each newly decoded image, find <img> nodes referencing it
        // and update their intrinsic dimensions
        for src in &newly_decoded {
            let img_dims = self.tree.image_cache.get(src).map(|i| (i.width, i.height));
            let Some((iw, ih)) = img_dims else {
                continue;
            };

            // Scan all nodes for img elements with matching src
            let node_ids: Vec<usize> = self
                .tree
                .nodes
                .iter()
                .filter_map(|(id, node)| {
                    if node.tag() == Some("img")
                        && node.attributes.get("src").map(|s| s.as_str()) == Some(src)
                    {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect();

            for node_id in node_ids {
                if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id {
                    let _ = self.tree.taffy.set_node_context(
                        taffy_id,
                        Some(crate::node::NodeContext::Image {
                            src: src.clone(),
                            width: iw,
                            height: ih,
                        }),
                    );
                    let _ = self.tree.taffy.mark_dirty(taffy_id);
                }
                self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
            }
        }

        true
    }
}

impl RinchDocument {
    /// Simple recursive query selector.
    fn query_recursive(&self, node_id: usize, selector: &str) -> Option<usize> {
        let node = self.tree.nodes.get(node_id)?;

        // Match by #id
        if let Some(id) = selector.strip_prefix('#') {
            if node.attributes.get("id").map(|v| v.as_str()) == Some(id) {
                return Some(node_id);
            }
        }
        // Match by .class
        else if let Some(class) = selector.strip_prefix('.') {
            if let Some(classes) = node.attributes.get("class")
                && classes.split_whitespace().any(|c| c == class)
            {
                return Some(node_id);
            }
        }
        // Match by attribute selector [attr] or [attr=value]
        else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
        {
            if let Some((attr_name, attr_value)) = attr_sel.split_once('=') {
                // [attr=value]
                let value = attr_value.trim_matches('"').trim_matches('\'');
                if node.attributes.get(attr_name).map(|v| v.as_str()) == Some(value) {
                    return Some(node_id);
                }
            } else {
                // [attr]
                if node.attributes.contains_key(attr_sel) {
                    return Some(node_id);
                }
            }
        }
        // Match by tag name
        else if node.tag() == Some(selector) {
            return Some(node_id);
        }

        // Search children
        let children: Vec<_> = node.children.clone();
        for child in children {
            if let Some(found) = self.query_recursive(child, selector) {
                return Some(found);
            }
        }
        None
    }

    /// Query all nodes matching a selector.
    fn query_all_recursive(&self, node_id: usize, selector: &str, results: &mut Vec<usize>) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return;
        };

        let matches = if let Some(id) = selector.strip_prefix('#') {
            node.attributes.get("id").map(|v| v.as_str()) == Some(id)
        } else if let Some(class) = selector.strip_prefix('.') {
            node.attributes
                .get("class")
                .map(|classes| classes.split_whitespace().any(|c| c == class))
                .unwrap_or(false)
        } else if let Some(attr_sel) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
        {
            if let Some((attr_name, attr_value)) = attr_sel.split_once('=') {
                let value = attr_value.trim_matches('"').trim_matches('\'');
                node.attributes.get(attr_name).map(|v| v.as_str()) == Some(value)
            } else {
                node.attributes.contains_key(attr_sel)
            }
        } else {
            node.tag() == Some(selector)
        };

        if matches {
            results.push(node_id);
        }

        // Search all children
        let children: Vec<_> = node.children.clone();
        for child in children {
            self.query_all_recursive(child, selector, results);
        }
    }

    /// Query all nodes matching a selector, returning a vector of NodeIds.
    pub fn query_selector_all(&self, selector: &str) -> Vec<rinch_core::dom::NodeId> {
        let mut results = Vec::new();
        self.query_all_recursive(self.tree.root_id, selector, &mut results);
        results.into_iter().map(rinch_core::dom::NodeId).collect()
    }

    /// Recursively collect text content from a node and its descendants.
    pub(crate) fn collect_text_content(&self, node_id: usize, result: &mut String) {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return;
        };
        match &node.kind {
            crate::node::NodeKind::Text(data) => result.push_str(&data.content),
            crate::node::NodeKind::Element(_) => {
                for &child_id in &node.children {
                    self.collect_text_content(child_id, result);
                }
            }
            _ => {}
        }
    }
}

/// Parse a CSS style string like "display: flex; gap: 8px" into key-value pairs.
pub(super) fn parse_style_string(style: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for part in style.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once(':') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    result
}

/// Parse an inline `style` attribute value into Stylo's declaration block the
/// way author content wants it: `about:blank`, no quirks, no error reporting.
/// One place, so `set_attribute("style")`, `set_styles` and the inset fast
/// path can't drift.
pub(super) fn parse_inline_style(css: &str) -> PropertyDeclarationBlock {
    style::properties::parse_style_attribute(
        css,
        &crate::layout::BLANK_URL_DATA,
        None,
        QuirksMode::NoQuirks,
        style::stylesheets::CssRuleType::Style,
    )
}
