//! Style resolution: Stylo CSS cascade, Taffy sync, hover, and theme operations.

mod pseudo;
mod resolve;
mod state_tracking;

use servo_arc::Arc as ServoArc;

use euclid::Scale;
use rinch_core::dom::NodeId;
use style::context::QuirksMode;
use style::media_queries::{Device, MediaType};
use style::properties::ComputedValues;
use style::properties::style_structs::Font as StyloFont;
use style::queries::values::PrefersColorScheme;

use crate::RinchDocument;
use crate::computed_style::ComputedStyle;
use crate::layout;
use crate::node::{DirtyFlags, DisplayMode, NodeTree};

use super::dom_impl::SimpleFontMetricsProvider;

impl RinchDocument {
    /// Load CSS into the document's stylesheet.
    ///
    /// Parses the CSS string and merges rules/variables into the existing stylesheet.
    /// Call this at startup to load theme and component CSS.
    pub fn load_css(&mut self, css: &str) {
        self.load_stylo_css(css);
    }

    /// Create a DOM node from a parsed HTML node (recursive).
    pub(crate) fn create_node_from_parsed(
        &mut self,
        parsed: &crate::html_parser::ParsedNode,
    ) -> NodeId {
        use crate::html_parser::ParsedNode;
        use rinch_core::dom::DomDocument;

        match parsed {
            ParsedNode::Element {
                tag,
                attrs,
                children,
            } => {
                let node_id = self.create_element(tag);

                // Set attributes
                for (name, value) in attrs {
                    self.set_attribute(node_id, name, value);
                }

                // Recursively create and append children
                for child in children {
                    let child_id = self.create_node_from_parsed(child);
                    self.append_child(node_id, child_id);
                }

                node_id
            }
            ParsedNode::Text(text) => self.create_text(text),
        }
    }

    /// If the given node is a `<style>` element, extract its text children's content
    /// and load it into the stylesheet.
    pub(crate) fn maybe_load_style_css(&mut self, node_id: usize) {
        let is_style = self
            .tree
            .nodes
            .get(node_id)
            .and_then(|n| n.tag())
            .map(|t| t == "style")
            .unwrap_or(false);
        if !is_style {
            return;
        }

        // Collect text content from children
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();
        let mut css = String::new();
        for child_id in children {
            if let Some(text) = self.tree.nodes.get(child_id).and_then(|n| n.text_content()) {
                css.push_str(text);
            }
        }
        if !css.is_empty() {
            self.load_stylo_css(&css);
            // New CSS rules may affect any existing node — invalidate all caches
            // and clear style_roots to force a full tree walk.
            for (nid, _) in self.tree.nodes.iter() {
                *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
            }
            self.tree.style_roots.clear();
            self.tree.styles_dirty = true;
            self.resolve_styles();
            self.apply_stylo_styles_to_taffy();
        }
    }

    /// Set viewport dimensions for resolving vh/vw CSS units.
    /// This also updates the Stylo Device so that vh/vw units resolve correctly.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.set_stylist_viewport(width, height);
    }

    /// Set viewport dimensions for the Stylo Device.
    /// Call this when the window is resized to update media queries and viewport units.
    pub fn set_stylist_viewport(&mut self, width: f32, height: f32) {
        use style::shared_lock::StylesheetGuards;
        use style::stylesheets::Origin;

        // Update our internal viewport tracking
        self.tree.viewport = crate::layout::Viewport { width, height };

        // Create a new Device with the updated viewport
        let viewport_size = euclid::Size2D::new(width, height);
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

        // Update the stylist's device using StylesheetGuards
        let guard = self.tree.guard.read();
        let guards = StylesheetGuards::same(&guard);
        self.stylist.set_device(device, &guards);

        // Mark all stylesheet origins as dirty to force style recomputation with new viewport
        self.stylist
            .force_stylesheet_origins_dirty(Origin::UserAgent.into());
        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());
    }

    /// Load CSS into Stylo's stylesheet system.
    ///
    /// Parses the CSS string and adds it to the Stylist for CSS cascade.
    /// This is the Stylo-based replacement for the old stylesheet system.
    pub fn load_stylo_css(&mut self, css: &str) {
        use style::media_queries::MediaList;
        use style::stylesheets::{
            AllowImportRules, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
        };

        // Create a dummy URL for the stylesheet
        let url_data = UrlExtraData::from(
            ::url::Url::parse("about:blank").expect("about:blank is a valid URL"),
        );

        // Parse the CSS into a stylesheet
        let media = ServoArc::new(self.tree.guard.wrap(MediaList::empty()));
        let stylesheet = Stylesheet::from_str(
            css,
            url_data,
            Origin::Author,
            media,
            self.tree.guard.clone(),
            None, // stylesheet_loader
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );

        // Wrap in DocumentStyleSheet for the Stylist
        let doc_stylesheet = DocumentStyleSheet(ServoArc::new(stylesheet));

        // Add the stylesheet to the stylist
        let guard = self.tree.guard.read();
        self.stylist.append_stylesheet(doc_stylesheet, &guard);

        // Mark stylesheets as changed so they'll be flushed on next style computation
        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());
    }

    /// Get the default display type for a node based on its tag.
    pub(crate) fn default_display_for_node(&self, node_id: usize) -> layout::DefaultDisplay {
        match self.tree.nodes[node_id].display_mode {
            crate::node::DisplayMode::Inline | crate::node::DisplayMode::InlineBlock => {
                layout::DefaultDisplay::Inline
            }
            _ => layout::DefaultDisplay::Block,
        }
    }

    /// Update theme CSS variables without duplicating non-`:root` rules.
    /// After calling this, call `recompute_all_styles_full()` to apply the new variables.
    pub fn update_theme_variables(&mut self, css: &str) {
        // Load CSS variables into Stylo
        self.load_stylo_css(css);
    }

    /// Recompute taffy styles for all element nodes, clearing cached style props
    /// so that CSS variables are re-resolved. Use this after `update_theme_variables()`.
    pub fn recompute_all_styles_full(&mut self) {
        // Clear cached Stylo element data so styles are recomputed
        let node_ids: Vec<usize> = self.tree.nodes.iter().map(|(id, _)| id).collect();
        for &nid in &node_ids {
            *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
        }
        // Clear roots to force full tree walk
        self.tree.style_roots.clear();
        // Resolve styles using Stylo
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
    }

    /// Recompute taffy styles for all element nodes.
    /// Called when viewport dimensions change to update vh/vw-dependent styles.
    #[allow(dead_code)]
    pub(crate) fn recompute_all_styles(&mut self) {
        // When viewport changes, Stylo needs to know about it to recalculate vh/vw units
        // For now, just resolve styles and apply to Taffy
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
    }

    /// Recompute styles recursively for a node and all its descendants.
    /// This is needed when a node is inserted into a new parent, as ancestor-based
    /// CSS selectors (like `.parent .child`) need to be re-evaluated with the new ancestor chain.
    pub(crate) fn recompute_node_styles_recursive(&mut self, node_id: usize) {
        if !self.tree.nodes[node_id].is_element() {
            return;
        }

        // Invalidate cached Stylo data for this node and its descendants
        fn invalidate_recursive(tree: &mut NodeTree, node_id: usize) {
            *tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
            let children = tree.nodes[node_id].children.clone();
            for &child_id in &children {
                invalidate_recursive(tree, child_id);
            }
        }
        self.tree.style_roots.push(node_id);
        invalidate_recursive(&mut self.tree, node_id);

        // Resolve styles using Stylo
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.push_dirty_flags(
            node_id,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    /// Compute the taffy child index for a DOM child at the given position.
    /// This counts only children that have taffy IDs (skipping comments).
    pub(crate) fn compute_taffy_child_index(&self, parent_id: usize, dom_index: usize) -> usize {
        let children = &self.tree.nodes[parent_id].children;
        let mut taffy_idx = 0;
        for i in 0..dom_index {
            if i < children.len() && self.tree.nodes[children[i]].taffy_id.is_some() {
                taffy_idx += 1;
            }
        }
        taffy_idx
    }

    /// Apply Stylo computed styles to Taffy layout nodes.
    ///
    /// This reads from each element's `stylo_element_data` and sets the corresponding
    /// Taffy style. It also updates our `ComputedStyle` for paint operations.
    ///
    /// When transitions are enabled, property changes are intercepted and animated
    /// instead of applied immediately.
    ///
    /// PERFORMANCE: Only processes nodes in `style_dirty_nodes` (set by resolve_styles).
    pub fn apply_stylo_styles_to_taffy(&mut self) {
        use crate::transition::{
            TransitionSpec, apply_value_to_style, diff_animatable, start_transitions,
        };

        // Take the dirty nodes list - only these need Taffy sync
        let dirty_node_ids = std::mem::take(&mut self.tree.style_dirty_nodes);

        // If no dirty nodes, nothing to do
        if dirty_node_ids.is_empty() {
            return;
        }
        let perf = std::env::var("RINCH_PERF").is_ok();
        let style_dirty_count = dirty_node_ids.len();
        let taffy_style_changed_count = std::cell::Cell::new(0u32);

        // Get current time for transition start timestamps
        let current_time_ms = self.current_time_ms();

        for node_id in dirty_node_ids {
            // Skip root and html nodes - their Taffy styles are manually set
            if node_id == self.tree.root_id || node_id == self.tree.html_id {
                continue;
            }

            let node = match self.tree.nodes.get(node_id) {
                Some(n) => n,
                None => continue,
            };

            if !node.is_element() {
                continue;
            }

            // Skip elements that should never participate in layout
            if matches!(
                node.tag(),
                Some("style" | "script" | "head" | "meta" | "link" | "title")
            ) {
                continue;
            }

            // Get Taffy node ID
            let taffy_id = match node.taffy_id {
                Some(id) => id,
                None => continue,
            };

            // Get computed values from Stylo
            let stylo_data = node.stylo_element_data.borrow();
            let computed_values = match stylo_data.as_ref().and_then(|d| d.styles.get_primary()) {
                Some(cv) => cv.clone(),
                None => continue,
            };
            drop(stylo_data);

            // Convert Stylo ComputedValues to our ComputedStyle
            let mut new_style = ComputedStyle::from_stylo(&computed_values);

            // Apply HTML presentational defaults that Stylo doesn't handle.
            // Stylo (servo build) doesn't always apply UA stylesheet defaults
            // for font-weight, font-style, and text-decoration on semantic HTML
            // elements. Apply them based on tag name as browsers do.
            match node.tag() {
                Some("strong" | "b") => {
                    if new_style.font_weight == 400.0 {
                        new_style.font_weight = 700.0;
                    }
                }
                Some("em" | "i") => {
                    if new_style.font_style == crate::computed_style::values::FontStyleValue::Normal
                    {
                        new_style.font_style =
                            crate::computed_style::values::FontStyleValue::Italic;
                    }
                }
                Some("u" | "ins") => new_style.text_decoration.underline = true,
                Some("s" | "strike" | "del") => new_style.text_decoration.strikethrough = true,
                _ => {}
            }

            // Extract transition specs from Stylo
            let transition_specs = TransitionSpec::extract_from_stylo(&computed_values);
            self.tree.nodes[node_id].transition_specs = transition_specs;

            // --- Transition logic ---
            let specs = &self.tree.nodes[node_id].transition_specs;
            let node_has_been_styled = self.tree.nodes[node_id].has_been_styled;
            if self.tree.transitions_enabled && node_has_been_styled && !specs.is_empty() {
                let old_style = &self.tree.nodes[node_id].computed_style;
                let diffs = diff_animatable(old_style, &new_style);

                if !diffs.is_empty() {
                    // Clone specs for borrow-checker (specs borrows from tree.nodes)
                    let specs_clone: Vec<TransitionSpec> = specs.clone();

                    let transitions_map = self.tree.active_transitions.entry(node_id).or_default();

                    let transitioning =
                        start_transitions(transitions_map, &specs_clone, &diffs, current_time_ms);

                    // Apply new_style to computed_style, but for transitioning
                    // properties, keep the current interpolated value
                    self.tree.nodes[node_id].computed_style = new_style.clone();

                    // Overwrite transitioning properties with their current interpolated values
                    for prop in &transitioning {
                        if let Some(trans_map) = self.tree.active_transitions.get(&node_id)
                            && let Some(transition) = trans_map.get(prop)
                            && let Some(value) = transition.value_at(current_time_ms)
                        {
                            apply_value_to_style(
                                &mut self.tree.nodes[node_id].computed_style,
                                *prop,
                                &value,
                            );
                        }
                    }
                } else {
                    // No animatable diffs — apply directly
                    self.tree.nodes[node_id].computed_style = new_style.clone();
                }
            } else {
                // No transitions — apply directly (current behavior)
                self.tree.nodes[node_id].computed_style = new_style.clone();
            }

            // Mark node as styled so future changes can trigger transitions
            self.tree.nodes[node_id].has_been_styled = true;

            // Sync display_mode from computed style (always from new_style target)
            let display_mode = match new_style.display {
                crate::computed_style::DisplayValue::Inline => DisplayMode::Inline,
                crate::computed_style::DisplayValue::InlineBlock => DisplayMode::InlineBlock,
                crate::computed_style::DisplayValue::InlineFlex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Block => DisplayMode::Block,
                crate::computed_style::DisplayValue::Flex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Grid => DisplayMode::Block,
                crate::computed_style::DisplayValue::None => DisplayMode::Block,
                crate::computed_style::DisplayValue::Contents => DisplayMode::Block,
            };
            self.tree.nodes[node_id].display_mode = display_mode;

            // Convert to Taffy style (from current computed_style which may have transition values)
            let dd = self.default_display_for_node(node_id);
            let mut taffy_style = self.tree.nodes[node_id].computed_style.to_taffy_style(dd);

            // HTML element must fill the viewport and clip horizontal overflow
            // (mirrors browser behavior where the viewport constrains content width).
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

            // Body node needs flex_grow: 1 and height: auto to fill the viewport
            if node_id == self.tree.body_id {
                if taffy_style.flex_grow == 0.0 {
                    taffy_style.flex_grow = 1.0;
                }
                if taffy_style.size.height == taffy::Dimension::auto() {
                    taffy_style.size.height = taffy::Dimension::auto();
                }
                if taffy_style.size.width == taffy::Dimension::auto() {
                    taffy_style.size.width = taffy::Dimension::percent(1.0);
                }
            }

            // Only call set_style if the Taffy style actually changed.
            // This avoids marking the Taffy tree dirty for paint-only changes
            // (e.g. background-color on hover) which don't affect layout.
            if let Ok(old_taffy_style) = self.tree.taffy.style(taffy_id) {
                if old_taffy_style != &taffy_style {
                    // Only set ifc_dirty when display changes — that's what affects
                    // IFC structure (inline/block mixing, display:contents, display:none).
                    // Other layout changes (width, padding, margin) don't need IFC rebuild.
                    if old_taffy_style.display != taffy_style.display {
                        self.tree.ifc_dirty = true;
                    }
                    let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                    self.tree.layout_dirty = true;
                    taffy_style_changed_count.set(taffy_style_changed_count.get() + 1);
                }
            } else {
                let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
                self.tree.layout_dirty = true;
                self.tree.ifc_dirty = true;
                taffy_style_changed_count.set(taffy_style_changed_count.get() + 1);
            }
        }
        if perf {
            eprintln!(
                "  [PERF] apply_to_taffy: style_dirty_nodes={} taffy_changed={}",
                style_dirty_count,
                taffy_style_changed_count.get()
            );
        }
    }

    /// Get a monotonic timestamp in milliseconds for transition timing.
    fn current_time_ms(&self) -> f64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
    }
}
