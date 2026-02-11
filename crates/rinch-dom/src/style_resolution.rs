//! Style resolution: Stylo CSS cascade, Taffy sync, hover, and theme operations.

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
    /// Call this at startup to load theme and widget CSS.
    pub fn load_css(&mut self, css: &str) {
        self.load_stylo_css(css);
    }

    /// Create a DOM node from a parsed HTML node (recursive).
    pub(crate) fn create_node_from_parsed(&mut self, parsed: &crate::html_parser::ParsedNode) -> NodeId {
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
            // Recompute all styles since new CSS rules may affect existing nodes
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

    /// Resolve styles for all elements using Stylo's CSS cascade.
    ///
    /// This walks the DOM tree and computes styles for each element using:
    /// 1. Selector matching via `push_applicable_declarations()`
    /// 2. Rule tree construction via `compute_rule_node()`
    /// 3. Cascade via `cascade_style_and_visited()`
    ///
    /// The computed styles are stored in each node's `stylo_element_data` field.
    pub fn resolve_styles(&mut self) {
        use crate::stylo_impl::RinchNode;
        use style::shared_lock::StylesheetGuards;

        // Flush any pending stylesheet changes
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            self.stylist.flush::<RinchNode>(&guards, None, None);
        }

        // Start from the html element and traverse down
        let html_id = self.tree.html_id;
        self.resolve_styles_recursive(html_id, None);
    }

    /// Recursively resolve styles for a node and its descendants.
    fn resolve_styles_recursive(
        &mut self,
        node_id: usize,
        parent_style: Option<ServoArc<ComputedValues>>,
    ) {
        use selectors::matching::{
            IncludeStartingStyle, MatchingContext, MatchingForInvalidation, MatchingMode,
            NeedsSelectorFlags, SelectorCaches, VisitedHandlingMode,
        };
        use style::applicable_declarations::ApplicableDeclarationList;
        use style::context::CascadeInputs;
        use style::data::ElementData;
        use style::properties::FirstLineReparenting;
        use style::rule_cache::RuleCacheConditions;
        use style::shared_lock::StylesheetGuards;
        use style::stylist::RuleInclusion;

        use crate::stylo_impl::RinchNode;

        // Extract node info in a block to release borrows before recursion
        let (is_element, children, cached_style) = {
            let node = match self.tree.nodes.get(node_id) {
                Some(n) => n,
                None => return,
            };

            let is_element = node.is_element();
            let children: Vec<usize> = node.children.clone();

            // Check for cached style
            let cached_style = {
                let stylo_data = node.stylo_element_data.borrow();
                stylo_data.as_ref().and_then(|d| d.styles.primary.clone())
            };

            (is_element, children, cached_style)
        };

        // Skip non-element nodes
        if !is_element {
            // For text nodes, just recurse to children (shouldn't have any)
            for child_id in children {
                self.resolve_styles_recursive(child_id, parent_style.clone());
            }
            return;
        }

        // PERFORMANCE: Skip nodes that already have computed styles (cache hit)
        // When a node's style changes, its stylo_element_data is set to None,
        // causing it to be recomputed. Nodes with valid cached styles are skipped.
        if let Some(computed) = cached_style {
            for child_id in children {
                self.resolve_styles_recursive(child_id, Some(computed.clone()));
            }
            return;
        }

        // Compute styles in a block so borrows are dropped before recursion
        let (computed, children) = {
            // Create the RinchNode wrapper for Stylo
            let rinch_node = RinchNode::new(node_id, &self.tree);

            // Set up matching context
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);

            let mut selector_caches = SelectorCaches::default();
            let mut matching_context = MatchingContext::new_for_visited(
                MatchingMode::Normal,
                None, // bloom filter - could add for performance
                &mut selector_caches,
                VisitedHandlingMode::AllLinksUnvisited,
                IncludeStartingStyle::No,
                self.stylist.quirks_mode(),
                NeedsSelectorFlags::No,
                MatchingForInvalidation::No,
            );

            // Collect applicable declarations
            let mut applicable_declarations = ApplicableDeclarationList::new();

            // Get the style attribute (already parsed and cached)
            let style_attribute = rinch_node
                .node()
                .style_attribute_cache
                .as_ref()
                .map(|arc| arc.borrow_arc());

            self.stylist.push_applicable_declarations(
                rinch_node,
                None, // pseudo_element
                style_attribute,
                None,               // smil_override
                Default::default(), // animation_declarations
                RuleInclusion::All,
                &mut applicable_declarations,
                &mut matching_context,
            );

            // Build rule node from applicable declarations
            let rule_node = self
                .stylist
                .rule_tree()
                .compute_rule_node(&mut applicable_declarations, &guards);

            // Cascade to compute final styles
            let parent_style_ref = parent_style.as_deref();
            let mut rule_cache_conditions = RuleCacheConditions::default();

            let computed = self.stylist.cascade_style_and_visited(
                Some(rinch_node),
                None, // pseudo
                CascadeInputs {
                    rules: Some(rule_node),
                    visited_rules: None,
                    flags: matching_context.extra_data.cascade_input_flags,
                },
                &guards,
                parent_style_ref, // parent_style
                parent_style_ref, // layout_parent_style
                FirstLineReparenting::No,
                &Default::default(), // try_tactic (PositionTryFallbacksTryTactic)
                None,                // rule_cache
                &mut rule_cache_conditions,
            );

            // Store the computed style in the node's ElementData
            {
                let mut stylo_data = self.tree.nodes[node_id].stylo_element_data.borrow_mut();
                if stylo_data.is_none() {
                    *stylo_data = Some(ElementData::default());
                }
                if let Some(ref mut data) = *stylo_data {
                    data.styles.primary = Some(computed.clone());
                }
            }

            // Mark this node as needing Taffy sync (style was recomputed)
            self.tree.style_dirty_nodes.push(node_id);

            // Clone children list before returning
            let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

            (computed.clone(), children)
        };

        // Now we can recurse without holding borrows
        for child_id in children {
            self.resolve_styles_recursive(child_id, Some(computed.clone()));
        }
    }

    /// Update hover state: set the hovered node and its ancestors as hovered,
    /// clear previous hover, and recompute styles for affected nodes.
    /// Returns true if the hovered node changed (caller should repaint).
    pub fn update_hover(&mut self, new_hovered: Option<usize>) -> bool {
        let old_hovered = self.tree.hovered_node;
        if old_hovered == new_hovered {
            return false;
        }

        // Collect old hovered chain (node + ancestors)
        let mut old_chain = Vec::new();
        if let Some(old_id) = old_hovered {
            let mut current = Some(old_id);
            while let Some(id) = current {
                old_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Collect new hovered chain (node + ancestors)
        let mut new_chain = Vec::new();
        if let Some(new_id) = new_hovered {
            let mut current = Some(new_id);
            while let Some(id) = current {
                new_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Clear old hover state
        for &id in &old_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = false;
            }
        }

        // Set new hover state
        for &id in &new_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_hovered = true;
            }
        }

        self.tree.hovered_node = new_hovered;

        // Recompute styles for nodes that changed hover state
        // (nodes in old chain but not new, and vice versa)
        let mut dirty_nodes: Vec<usize> = Vec::new();
        for &id in &old_chain {
            if !new_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }
        for &id in &new_chain {
            if !old_chain.contains(&id) {
                dirty_nodes.push(id);
            }
        }

        // Recompute styles using Stylo for affected nodes
        // For simplicity, we recompute styles for the entire tree
        // (a more optimized approach would only restyle the affected subtrees)
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        for id in dirty_nodes {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
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
    /// PERFORMANCE: Only processes nodes in `style_dirty_nodes` (set by resolve_styles).
    pub fn apply_stylo_styles_to_taffy(&mut self) {
        // Take the dirty nodes list - only these need Taffy sync
        let dirty_node_ids = std::mem::take(&mut self.tree.style_dirty_nodes);

        // If no dirty nodes, nothing to do
        if dirty_node_ids.is_empty() {
            return;
        }

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
            let computed_style = ComputedStyle::from_stylo(&computed_values);

            // Update node's computed_style for paint operations
            self.tree.nodes[node_id].computed_style = computed_style.clone();

            // Sync display_mode from computed style
            let display_mode = match computed_style.display {
                crate::computed_style::DisplayValue::Inline => DisplayMode::Inline,
                crate::computed_style::DisplayValue::InlineBlock => DisplayMode::InlineBlock,
                crate::computed_style::DisplayValue::InlineFlex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Block => DisplayMode::Block,
                crate::computed_style::DisplayValue::Flex => DisplayMode::Flex,
                crate::computed_style::DisplayValue::Grid => DisplayMode::Block, // Grid treated as block for IFC
                crate::computed_style::DisplayValue::None => DisplayMode::Block,
                crate::computed_style::DisplayValue::Contents => DisplayMode::Block,
            };
            self.tree.nodes[node_id].display_mode = display_mode;

            // Convert to Taffy style
            let dd = self.default_display_for_node(node_id);
            let mut taffy_style = computed_style.to_taffy_style(dd);

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

            // Apply to Taffy node
            let _ = self.tree.taffy.set_style(taffy_id, taffy_style);
        }
    }
}
