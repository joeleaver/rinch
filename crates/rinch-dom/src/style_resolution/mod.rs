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

    /// Parse a CSS string into an author-origin Stylo stylesheet.
    fn parse_author_stylesheet(&self, css: &str) -> style::stylesheets::DocumentStyleSheet {
        use style::media_queries::MediaList;
        use style::stylesheets::{
            AllowImportRules, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
        };

        // Create a dummy URL for the stylesheet
        let url_data = UrlExtraData::from(
            ::url::Url::parse("about:blank").expect("about:blank is a valid URL"),
        );

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

        DocumentStyleSheet(ServoArc::new(stylesheet))
    }

    /// Load CSS into Stylo's stylesheet system.
    ///
    /// Parses the CSS string and appends it to the Stylist for CSS cascade.
    /// Appended sheets cascade after everything loaded before them, matching
    /// document source order.
    pub fn load_stylo_css(&mut self, css: &str) {
        use style::stylesheets::Origin;

        let doc_stylesheet = self.parse_author_stylesheet(css);

        // Add the stylesheet to the stylist
        let guard = self.tree.guard.read();
        self.stylist
            .append_stylesheet(doc_stylesheet.clone(), &guard);
        drop(guard);

        // Track app author sheets in insertion order so the theme sheet can be
        // re-inserted ahead of them (see `set_theme_css`).
        self.author_stylesheets.push(doc_stylesheet);

        // Mark stylesheets as changed so they'll be flushed on next style computation
        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());

        self.recompute_bare_focus_rules();
    }

    /// Recompute [`Self::has_bare_focus_rules`] over the theme sheet and every
    /// app author sheet. See the field's doc for why bare (unanchored) focus
    /// selectors defeat the `focus_sensitive` invalidation scheme: stylo's
    /// `SelectorMap` puts them in a bucket that is only consulted while the
    /// element already has focus state, so matching never reaches them on an
    /// unfocused node. (The UA sheet is not scanned — it carries no focus
    /// rules.)
    fn recompute_bare_focus_rules(&mut self) {
        let sheets: Vec<style::stylesheets::DocumentStyleSheet> = self
            .theme_stylesheet
            .iter()
            .chain(self.author_stylesheets.iter())
            .cloned()
            .collect();
        let guard = self.tree.guard.read();
        let found = sheets.iter().any(|sheet| {
            let contents = sheet.0.contents.read_with(&guard);
            Self::rules_have_bare_focus(contents.rules(&guard), &guard)
        });
        drop(guard);
        self.has_bare_focus_rules = found;
    }

    /// Whether any style rule in `rules` (recursing into `@media` / `@supports`)
    /// has a selector whose rightmost compound contains a focus pseudo-class and
    /// no tag/class/id/attribute anchor — the shape stylo buckets into the
    /// state-gated `rare_pseudo_classes` map.
    fn rules_have_bare_focus(
        rules: &[style::stylesheets::CssRule],
        guard: &style::shared_lock::SharedRwLockReadGuard,
    ) -> bool {
        use style::stylesheets::CssRule;
        rules.iter().any(|rule| match rule {
            CssRule::Style(locked) => {
                let style_rule = locked.read_with(guard);
                style_rule
                    .selectors
                    .slice()
                    .iter()
                    .any(Self::selector_is_bare_focus)
            }
            CssRule::Media(media) => {
                Self::rules_have_bare_focus(&media.rules.read_with(guard).0, guard)
            }
            CssRule::Supports(supports) => {
                Self::rules_have_bare_focus(&supports.rules.read_with(guard).0, guard)
            }
            _ => false,
        })
    }

    /// Whether a selector's rightmost compound contains a focus pseudo-class
    /// (`:focus`, `:focus-visible`, `:focus-within` — directly or inside
    /// `:not()`/`:is()`/`:where()`) without any tag/class/id/attribute/`:root`
    /// anchor. Over-matching here is safe (it only costs an extra invalidation
    /// on focus change); under-matching re-introduces the never-restyled ring.
    fn selector_is_bare_focus(
        selector: &selectors::parser::Selector<style::selector_parser::SelectorImpl>,
    ) -> bool {
        use selectors::parser::Component;
        use style::selector_parser::NonTSPseudoClass;

        fn component_has_focus(c: &Component<style::selector_parser::SelectorImpl>) -> bool {
            match c {
                Component::NonTSPseudoClass(pc) => matches!(
                    pc,
                    NonTSPseudoClass::Focus
                        | NonTSPseudoClass::FocusVisible
                        | NonTSPseudoClass::FocusWithin
                ),
                Component::Negation(list) | Component::Is(list) | Component::Where(list) => list
                    .slice()
                    .iter()
                    .any(|s| s.iter().any(component_has_focus)),
                _ => false,
            }
        }

        let mut anchored = false;
        let mut has_focus = false;
        // `iter()` walks the rightmost compound only (stops at the first
        // combinator), which is exactly the compound stylo buckets by.
        for component in selector.iter() {
            match component {
                Component::LocalName(_)
                | Component::ID(_)
                | Component::Class(_)
                | Component::AttributeInNoNamespaceExists { .. }
                | Component::AttributeInNoNamespace { .. }
                | Component::AttributeOther(_)
                | Component::Root => anchored = true,
                other => {
                    if component_has_focus(other) {
                        has_focus = true;
                    }
                }
            }
        }
        has_focus && !anchored
    }

    /// Install (or replace) the theme stylesheet.
    ///
    /// The theme sheet occupies a stable slot *before* every app stylesheet, so
    /// app CSS always cascades over it. This mirrors rinch-web, where the theme
    /// lives in a single `<style data-rinch-theme>` in `<head>` that is updated
    /// in place — its document position never changes.
    ///
    /// Appending a regenerated theme sheet instead would move it *after* the
    /// app's `<style>` rules, letting theme `:root` custom properties silently
    /// win over an app's `:root` overrides of the same variable.
    pub fn set_theme_css(&mut self, css: &str) {
        use style::stylesheets::Origin;

        let new_sheet = self.parse_author_stylesheet(css);

        let guard = self.tree.guard.read();

        // Drop the previous theme sheet, if any.
        if let Some(old) = self.theme_stylesheet.take() {
            self.stylist.remove_stylesheet(old, &guard);
        }

        // Re-insert ahead of the first app sheet so app CSS keeps overriding the
        // theme. With no app sheets yet, appending puts it first anyway.
        match self.author_stylesheets.first() {
            Some(first_app) => {
                self.stylist
                    .insert_stylesheet_before(new_sheet.clone(), first_app.clone(), &guard);
            }
            None => {
                self.stylist.append_stylesheet(new_sheet.clone(), &guard);
            }
        }
        drop(guard);

        self.theme_stylesheet = Some(new_sheet);

        self.stylist
            .force_stylesheet_origins_dirty(Origin::Author.into());

        self.recompute_bare_focus_rules();
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
    ///
    /// Replaces the theme sheet in its stable pre-app slot; see [`Self::set_theme_css`].
    pub fn update_theme_variables(&mut self, css: &str) {
        self.set_theme_css(css);
    }

    /// Recompute taffy styles for all element nodes, clearing cached style props
    /// so that CSS variables are re-resolved. Use this after `update_theme_variables()`.
    pub fn recompute_all_styles_full(&mut self) {
        // Clear cached Stylo element data so styles are recomputed.
        // Also clear text_layout so build_ifc_layouts() doesn't skip
        // IFC roots whose text content hasn't changed but whose style
        // (e.g. text color) has — the old Parley layout has stale brushes.
        let node_ids: Vec<usize> = self.tree.nodes.iter().map(|(id, _)| id).collect();
        for &nid in &node_ids {
            *self.tree.nodes[nid].stylo_element_data.borrow_mut() = None;
            self.tree.nodes[nid].text_layout = None;
        }
        // Disable transitions during full restyle — theme changes should apply
        // instantly. Without this, elements with `transition: color` would start
        // animating from old values, baking stale colors into Parley text layouts.
        let transitions_were_enabled = self.tree.transitions_enabled;
        self.tree.transitions_enabled = false;
        self.tree.active_transitions.clear();
        self.tree.active_animations.clear();
        // Clear roots to force full tree walk
        self.tree.style_roots.clear();
        // Resolve styles using Stylo
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();
        self.tree.transitions_enabled = transitions_were_enabled;
        // Force IFC rebuild so text layouts pick up new colors/fonts from
        // the updated computed styles (text brush is baked into Parley layout).
        // Also set layout_dirty so resolve_layout doesn't early-return before
        // reaching the ifc_dirty branch.
        self.tree.ifc_dirty = true;
        self.tree.layout_dirty = true;
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
    /// Recompute Stylo styles for a node and all its descendants, then apply
    /// to Taffy. Called after bulk DOM operations to batch style resolution
    /// into a single pass.
    pub fn recompute_node_styles_recursive(&mut self, node_id: usize) {
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
                Some("strong" | "b") if new_style.font_weight == 400.0 => {
                    new_style.font_weight = 700.0;
                }
                Some("em" | "i")
                    if new_style.font_style
                        == crate::computed_style::values::FontStyleValue::Normal =>
                {
                    new_style.font_style = crate::computed_style::values::FontStyleValue::Italic;
                }
                Some("u" | "ins") => new_style.text_decoration.underline = true,
                Some("s" | "strike" | "del") => new_style.text_decoration.strikethrough = true,
                Some("code" | "pre" | "kbd" | "samp") => {
                    new_style.user_select = crate::computed_style::UserSelectValue::Text;
                }
                _ => {}
            }

            // `<textarea rows=N>` maps to an intrinsic height of N lines, as
            // browsers do. A textarea holds its value in an attribute rather
            // than as a text child, so nothing else gives it a content height —
            // without this it collapses to a single line regardless of `rows`.
            // The HTML default is 2 rows.
            if node.tag() == Some("textarea") && new_style.height.is_auto() {
                let rows = node
                    .attributes
                    .get("rows")
                    .and_then(|r| r.trim().parse::<f32>().ok())
                    .filter(|r| *r >= 1.0)
                    .unwrap_or(2.0);
                let line_h = match new_style.line_height {
                    crate::computed_style::LineHeightValue::Normal => new_style.font_size * 1.2,
                    crate::computed_style::LineHeightValue::Relative(r) => new_style.font_size * r,
                    crate::computed_style::LineHeightValue::Absolute(px) => px,
                };
                // min-height is a border-box value (rinch sets a global
                // `box-sizing: border-box`, and Taffy defaults to it), so the
                // padding and border have to be added on top of the line boxes.
                let intrinsic = rows * line_h
                    + new_style.padding_top.to_px()
                    + new_style.padding_bottom.to_px()
                    + new_style.border_top_width.to_px()
                    + new_style.border_bottom_width.to_px();
                // An author `min-height` still wins when it is the larger of
                // the two, matching `max(rows, min-height)` in browsers.
                let author_min = match new_style.min_height {
                    crate::computed_style::DimensionValue::Length(px) => px,
                    _ => 0.0,
                };
                new_style.min_height =
                    crate::computed_style::DimensionValue::Length(intrinsic.max(author_min));
            }

            // A closed `<select>` shows one option's label, so browsers size it to
            // fit the *widest* option — the width stays stable when the selection
            // changes. Its `<option>` children are `display:none` and give it no
            // content, so without this the control collapses to its padding and
            // clips the label. Only applied when the author left `width: auto`; an
            // explicit width is respected and the label clips (the painter clips
            // too). The text width is estimated from the label length rather than
            // measured with Parley (not available at style-resolution time) —
            // erring wide is harmless since the painter clips to the content box.
            if node.tag() == Some("select") && new_style.width.is_auto() {
                let model = crate::select::resolve_select_model(&self.tree, node_id);
                let widest = model
                    .options
                    .iter()
                    .map(|o| o.label.chars().count())
                    .max()
                    .unwrap_or(0);
                if widest > 0 {
                    let text_w = widest as f32 * new_style.font_size * 0.62;
                    // border-box, and padding_right already reserves the arrow box.
                    let intrinsic = text_w
                        + new_style.padding_left.to_px()
                        + new_style.padding_right.to_px()
                        + new_style.border_left_width.to_px()
                        + new_style.border_right_width.to_px();
                    let author_min = match new_style.min_width {
                        crate::computed_style::DimensionValue::Length(px) => px,
                        _ => 0.0,
                    };
                    new_style.min_width =
                        crate::computed_style::DimensionValue::Length(intrinsic.max(author_min));
                }
            }

            // Check inline style for user-select override (Stylo servo build
            // doesn't handle this property).
            if let Some(style_str) = node.attributes.get("style") {
                for part in style_str.split(';') {
                    let part = part.trim();
                    if let Some((key, value)) = part.split_once(':') {
                        if key.trim() == "user-select" {
                            new_style.user_select =
                                crate::computed_style::UserSelectValue::parse(value);
                        }
                    }
                }
            }

            // Capture old display before transitions overwrite computed_style
            let old_display = self.tree.nodes[node_id].computed_style.display;

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

            // --- Animation logic ---
            // Extract animation specs from Stylo
            let animation_specs =
                crate::animation::AnimationSpec::extract_from_stylo(&computed_values);
            self.tree.nodes[node_id].animation_specs = animation_specs;

            if self.tree.transitions_enabled {
                let anim_specs = self.tree.nodes[node_id].animation_specs.clone();
                let base_style = self.tree.nodes[node_id].computed_style.clone();
                let guard = self.tree.guard.read();

                // Extract active_animations temporarily to avoid borrow conflict
                let mut active_animations = std::mem::take(&mut self.tree.active_animations);
                crate::animation::start_animations(
                    &mut active_animations,
                    node_id,
                    &anim_specs,
                    &base_style,
                    &self.stylist,
                    &guard,
                    &self.tree,
                    current_time_ms,
                );
                self.tree.active_animations = active_animations;
                drop(guard);

                // Apply current animation values on top of computed_style
                if let Some(animations) = self.tree.active_animations.get(&node_id) {
                    for anim in animations {
                        if let crate::animation::AnimationResult::Values(values) =
                            anim.values_at(current_time_ms)
                        {
                            for (prop, value) in &values {
                                apply_value_to_style(
                                    &mut self.tree.nodes[node_id].computed_style,
                                    *prop,
                                    value,
                                );
                            }
                        }
                    }
                }
            }

            // Mark node as styled so future changes can trigger transitions
            // Reset scroll offset when an element transitions from display:none
            // to visible. This prevents stale scroll positions from a previous
            // display cycle (e.g., a menu flyout that was scrolled while the
            // window was small, then the window grows and the flyout reopens).
            if matches!(old_display, crate::computed_style::DisplayValue::None)
                && !matches!(new_style.display, crate::computed_style::DisplayValue::None)
            {
                self.tree.nodes[node_id].scroll_offset = (0.0, 0.0);
            }

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

            // position:fixed elements use the viewport as their containing block.
            // Taffy treats fixed as absolute (relative to DOM parent), so we give
            // these nodes explicit viewport dimensions. This ensures their children
            // (e.g. drawer panels with top:0;bottom:0) get correct layout.
            if self.tree.nodes[node_id].computed_style.position
                == crate::computed_style::PositionValue::Fixed
                && self.tree.nodes[node_id].computed_style.display
                    != crate::computed_style::DisplayValue::None
            {
                let vw = self.tree.viewport.width;
                let vh = self.tree.viewport.height;
                let cs = &self.tree.nodes[node_id].computed_style;

                // Compute width: if both left and right insets are set with auto width,
                // use viewport minus insets. Otherwise use full viewport width.
                if taffy_style.size.width == taffy::Dimension::auto() {
                    let left = cs.left.resolve(vw);
                    let right = cs.right.resolve(vw);
                    let w = match (left, right) {
                        (Some(l), Some(r)) => (vw - l - r).max(0.0),
                        _ => vw,
                    };
                    taffy_style.size.width = taffy::Dimension::length(w);
                }

                // Compute height: same logic for top/bottom insets.
                if taffy_style.size.height == taffy::Dimension::auto() {
                    let top = cs.top.resolve(vh);
                    let bottom = cs.bottom.resolve(vh);
                    let h = match (top, bottom) {
                        (Some(t), Some(b)) => (vh - t - b).max(0.0),
                        _ => vh,
                    };
                    taffy_style.size.height = taffy::Dimension::length(h);
                }
            }

            // Collapsed block (virtualized contenteditable): override height
            // to the estimated value so Taffy doesn't need a measure callback.
            if let Some(est_h) = self.tree.nodes[node_id].estimated_height {
                taffy_style.size.height = taffy::Dimension::length(est_h);
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
        use web_time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
    }
}
