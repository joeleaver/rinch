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

        // Remove existing pseudo-element children before re-resolving styles
        // to avoid duplicates when styles are recomputed.
        {
            let children_to_remove: Vec<usize> = self.tree.nodes[node_id]
                .children
                .iter()
                .filter(|&&cid| {
                    self.tree
                        .nodes
                        .get(cid)
                        .is_some_and(|n| n.is_pseudo_element)
                })
                .copied()
                .collect();
            for cid in children_to_remove {
                // Remove from taffy parent
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[node_id].taffy_id,
                    self.tree.nodes[cid].taffy_id,
                ) {
                    let _ = self.tree.taffy.remove_child(parent_taffy, child_taffy);
                }
                // Remove the pseudo-element's subtree from the slab
                self.tree.remove_subtree(cid);
                // Remove from parent's children list
                self.tree.nodes[node_id].children.retain(|&c| c != cid);
            }
        }

        // Compute styles in a block so borrows are dropped before recursion
        let computed = {
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

            computed.clone()
        };

        // Check for ::before and ::after pseudo-elements
        use style::selector_parser::PseudoElement;
        self.resolve_pseudo_element(node_id, &computed, PseudoElement::Before);
        self.resolve_pseudo_element(node_id, &computed, PseudoElement::After);

        // Re-read children list since pseudo-element resolution may have added nodes
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        // Now we can recurse without holding borrows
        for child_id in children {
            self.resolve_styles_recursive(child_id, Some(computed.clone()));
        }
    }

    /// Clear cached `stylo_element_data` for a node and all its descendants,
    /// forcing `resolve_styles_recursive()` to re-resolve their CSS.
    ///
    /// This is needed when pseudo-class state changes (hover, focus, active)
    /// because the cache optimization in `resolve_styles_recursive()` would
    /// otherwise skip nodes that already have computed styles.
    fn invalidate_style_subtree(&mut self, node_id: usize) {
        if let Some(node) = self.tree.nodes.get_mut(node_id) {
            *node.stylo_element_data.borrow_mut() = None;
            let children: Vec<usize> = node.children.clone();
            for child_id in children {
                self.invalidate_style_subtree(child_id);
            }
        }
    }

    /// Resolve a pseudo-element (::before or ::after) for a given parent node.
    ///
    /// Queries Stylo for the pseudo-element's computed styles, extracts the `content`
    /// property text, and creates synthetic DOM nodes (a wrapper span + text child)
    /// inserted as first child (::before) or last child (::after).
    fn resolve_pseudo_element(
        &mut self,
        parent_id: usize,
        parent_style: &ServoArc<ComputedValues>,
        pseudo: style::selector_parser::PseudoElement,
    ) {
        use selectors::matching::{
            IncludeStartingStyle, MatchingContext, MatchingForInvalidation, MatchingMode,
            NeedsSelectorFlags, SelectorCaches, VisitedHandlingMode,
        };
        use style::applicable_declarations::ApplicableDeclarationList;
        use style::context::CascadeInputs;
        use style::properties::FirstLineReparenting;
        use style::rule_cache::RuleCacheConditions;
        use style::selector_parser::PseudoElement;
        use style::shared_lock::StylesheetGuards;
        use style::stylist::RuleInclusion;

        use crate::stylo_impl::RinchNode;

        let is_before = matches!(pseudo, PseudoElement::Before);

        // Query Stylo for pseudo-element declarations
        let pseudo_computed = {
            let rinch_node = RinchNode::new(parent_id, &self.tree);
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);

            let mut selector_caches = SelectorCaches::default();
            let mut matching_context =
                MatchingContext::<'_, style::selector_parser::SelectorImpl>::new_for_visited(
                    MatchingMode::ForStatelessPseudoElement,
                    None,
                    &mut selector_caches,
                    VisitedHandlingMode::AllLinksUnvisited,
                    IncludeStartingStyle::No,
                    self.stylist.quirks_mode(),
                    NeedsSelectorFlags::No,
                    MatchingForInvalidation::No,
                );
            matching_context.extra_data.originating_element_style = Some(parent_style);

            let mut applicable_declarations = ApplicableDeclarationList::new();

            // Get the style attribute from the parent element
            let style_attribute = rinch_node
                .node()
                .style_attribute_cache
                .as_ref()
                .map(|arc| arc.borrow_arc());

            self.stylist.push_applicable_declarations(
                rinch_node,
                Some(&pseudo),
                style_attribute,
                None,
                Default::default(),
                RuleInclusion::All,
                &mut applicable_declarations,
                &mut matching_context,
            );

            // If no declarations matched, no pseudo-element defined
            if applicable_declarations.is_empty() {
                return;
            }

            let rule_node = self
                .stylist
                .rule_tree()
                .compute_rule_node(&mut applicable_declarations, &guards);

            let mut rule_cache_conditions = RuleCacheConditions::default();

            self.stylist.cascade_style_and_visited(
                Some(RinchNode::new(parent_id, &self.tree)),
                Some(&pseudo),
                CascadeInputs {
                    rules: Some(rule_node),
                    visited_rules: None,
                    flags: matching_context.extra_data.cascade_input_flags,
                },
                &guards,
                Some(parent_style),
                Some(parent_style),
                FirstLineReparenting::No,
                &Default::default(),
                None,
                &mut rule_cache_conditions,
            )
        };

        // Check content property - if none/normal/empty, skip
        if pseudo_computed.ineffective_content_property() {
            return;
        }

        // Extract text content from the content property
        let text = Self::extract_content_text(&pseudo_computed);
        if text.is_empty() {
            return;
        }

        // Convert pseudo computed style to our ComputedStyle
        let pseudo_style = ComputedStyle::from_stylo(&pseudo_computed);

        // Create a wrapper span element for the pseudo-element
        use rinch_core::dom::DomDocument;
        let span_id = self.create_element("span");
        let text_node_id = self.create_text(&text);

        // Append the text node to the span (using raw tree manipulation
        // to avoid triggering style recomputation via DomDocument::append_child)
        {
            let span_raw = span_id.0;
            let text_raw = text_node_id.0;
            self.tree.nodes[text_raw].parent = Some(span_raw);
            self.tree.nodes[span_raw].children.push(text_raw);
            // Sync taffy
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[span_raw].taffy_id,
                self.tree.nodes[text_raw].taffy_id,
            ) {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }

        // Set computed style and mark as pseudo-element
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].computed_style = pseudo_style;
            self.tree.nodes[span_raw].is_pseudo_element = true;
            self.tree.style_dirty_nodes.push(span_raw);
        }

        // Insert as first child (::before) or last child (::after) of parent
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].parent = Some(parent_id);
            if is_before {
                // Insert at beginning of parent's children
                self.tree.nodes[parent_id].children.insert(0, span_raw);
                // Sync taffy: insert at index 0
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[parent_id].taffy_id,
                    self.tree.nodes[span_raw].taffy_id,
                ) {
                    let _ = self
                        .tree
                        .taffy
                        .insert_child_at_index(parent_taffy, 0, child_taffy);
                }
            } else {
                // Append at end of parent's children
                self.tree.nodes[parent_id].children.push(span_raw);
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[parent_id].taffy_id,
                    self.tree.nodes[span_raw].taffy_id,
                ) {
                    let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
                }
            }
        }
    }

    /// Extract text content from a Stylo ComputedValues `content` property.
    /// Only handles string content items; skips counter(), attr(), url(), etc.
    fn extract_content_text(computed: &ComputedValues) -> String {
        use style::values::generics::counters::{Content, ContentItem};

        let content = &computed.get_counters().content;
        match content {
            Content::Normal | Content::None => String::new(),
            Content::Items(items) => {
                let mut result = String::new();
                for item in items.items.iter() {
                    if let ContentItem::String(s) = item {
                        result.push_str(s);
                    }
                }
                result
            }
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

        // Clear cached styles for affected nodes and their descendants
        // so resolve_styles_recursive() will re-resolve their CSS
        for &id in &dirty_nodes {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles using Stylo for affected nodes
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        for id in dirty_nodes {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }

    /// Update focus state: set the focused node, clear previous focus,
    /// and recompute styles for affected nodes.
    /// Returns true if the focused node changed (caller should repaint).
    pub fn update_focus(&mut self, new_focused: Option<usize>) -> bool {
        let old_focused = self.tree.focused_node;
        if old_focused == new_focused {
            return false;
        }

        // Clear old focus state
        if let Some(old_id) = old_focused
            && let Some(node) = self.tree.nodes.get_mut(old_id)
        {
            node.is_focused = false;
        }

        // Set new focus state
        if let Some(new_id) = new_focused
            && let Some(node) = self.tree.nodes.get_mut(new_id)
        {
            node.is_focused = true;
        }

        self.tree.focused_node = new_focused;

        // Clear cached styles for affected nodes and their descendants
        if let Some(id) = old_focused {
            self.invalidate_style_subtree(id);
        }
        if let Some(id) = new_focused {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles
        self.tree.styles_dirty = true;
        self.resolve_styles();
        self.apply_stylo_styles_to_taffy();

        // Mark dirty nodes for repaint
        if let Some(id) = old_focused {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }
        if let Some(id) = new_focused {
            self.push_dirty_flags(id, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }

        true
    }

    /// Update active (mouse-pressed) state: set the active node and its
    /// ancestors, clear previous active, and recompute styles.
    /// Returns true if the active node changed (caller should repaint).
    pub fn update_active(&mut self, new_active: Option<usize>) -> bool {
        let old_active = self.tree.active_node;
        if old_active == new_active {
            return false;
        }

        // Collect old active chain (node + ancestors)
        let mut old_chain = Vec::new();
        if let Some(old_id) = old_active {
            let mut current = Some(old_id);
            while let Some(id) = current {
                old_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Collect new active chain (node + ancestors)
        let mut new_chain = Vec::new();
        if let Some(new_id) = new_active {
            let mut current = Some(new_id);
            while let Some(id) = current {
                new_chain.push(id);
                current = self.tree.nodes.get(id).and_then(|n| n.parent);
            }
        }

        // Clear old active state
        for &id in &old_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_active = false;
            }
        }

        // Set new active state
        for &id in &new_chain {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.is_active = true;
            }
        }

        self.tree.active_node = new_active;

        // Collect nodes whose active state changed (symmetric difference)
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

        // Clear cached styles for affected nodes and their descendants
        for &id in &dirty_nodes {
            self.invalidate_style_subtree(id);
        }

        // Recompute styles
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
            let new_style = ComputedStyle::from_stylo(&computed_values);

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
