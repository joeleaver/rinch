//! Pseudo-element resolution (::before, ::after).

use servo_arc::Arc as ServoArc;

use style::properties::ComputedValues;

use crate::RinchDocument;
use crate::computed_style::ComputedStyle;

impl RinchDocument {
    /// Resolve a pseudo-element (::before or ::after) for a given parent node.
    ///
    /// Queries Stylo for the pseudo-element's computed styles, extracts the `content`
    /// property text, and creates synthetic DOM nodes (a wrapper span + text child)
    /// inserted as first child (::before) or last child (::after).
    pub(crate) fn resolve_pseudo_element(
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

        // Compute counter values for this element (needed for content: counter(...))
        let counter_values = self.compute_list_item_counters(parent_id);

        // Extract text content from the content property
        let text = Self::extract_content_text_with_counters(&pseudo_computed, &counter_values);
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
    /// Handles string content items and counter() references.
    /// For counter() values, uses the provided counter_values map.
    pub(crate) fn extract_content_text_with_counters(
        computed: &ComputedValues,
        counter_values: &std::collections::HashMap<String, i32>,
    ) -> String {
        use style::values::generics::counters::{Content, ContentItem};

        let content = &computed.get_counters().content;
        match content {
            Content::Normal | Content::None => String::new(),
            Content::Items(items) => {
                let mut result = String::new();
                for item in items.items.iter() {
                    match item {
                        ContentItem::String(s) => {
                            result.push_str(s);
                        }
                        ContentItem::Counter(name, _style) => {
                            let name_str = name.0.to_string();
                            if let Some(&value) = counter_values.get(&name_str) {
                                result.push_str(&value.to_string());
                            } else {
                                result.push('0');
                            }
                        }
                        ContentItem::Counters(name, separator, _style) => {
                            let name_str = name.0.to_string();
                            if let Some(&value) = counter_values.get(&name_str) {
                                result.push_str(&value.to_string());
                            } else {
                                result.push('0');
                            }
                            let _ = separator; // TODO: nested counter separator support
                        }
                        _ => {
                            // Skip other content items (attr, image, quotes, etc.)
                        }
                    }
                }
                result
            }
        }
    }

    /// Compute the CSS counter value for `list-item` on an `<li>` element.
    ///
    /// Walks siblings to determine the ordinal position of this `<li>`
    /// within its parent `<ol>` or `<ul>`. Returns a map with the
    /// `list-item` counter set to the appropriate value.
    pub(crate) fn compute_list_item_counters(
        &self,
        node_id: usize,
    ) -> std::collections::HashMap<String, i32> {
        let mut counters = std::collections::HashMap::new();

        let node = match self.tree.nodes.get(node_id) {
            Some(n) => n,
            None => return counters,
        };

        // Only compute for <li> elements
        if node.tag() != Some("li") {
            return counters;
        }

        let parent_id = match node.parent {
            Some(p) => p,
            None => return counters,
        };

        let parent = match self.tree.nodes.get(parent_id) {
            Some(n) => n,
            None => return counters,
        };

        // Check start attribute on parent <ol>
        let start = if parent.tag() == Some("ol") {
            parent
                .attributes
                .get("start")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(1)
        } else {
            1
        };

        // Count this <li>'s position among <li> siblings
        let mut count = start;
        for &sibling_id in &parent.children {
            if sibling_id == node_id {
                break;
            }
            if let Some(sibling) = self.tree.nodes.get(sibling_id) {
                if sibling.tag() == Some("li") {
                    // Check for value attribute override
                    if let Some(val) = sibling.attributes.get("value") {
                        if let Ok(v) = val.parse::<i32>() {
                            count = v + 1;
                            continue;
                        }
                    }
                    count += 1;
                }
            }
        }

        // Check if this <li> has a value attribute
        if let Some(val) = node.attributes.get("value") {
            if let Ok(v) = val.parse::<i32>() {
                count = v;
            }
        }

        counters.insert("list-item".to_string(), count);
        counters
    }

    /// Generate a list marker pseudo-element for `<li>` elements.
    ///
    /// If the `<li>` doesn't already have a `::before` pseudo-element from CSS,
    /// this creates a marker span with the appropriate text:
    /// - Ordered lists (`<ol>`): "1. ", "2. ", etc.
    /// - Unordered lists (`<ul>`): bullet character
    pub(crate) fn resolve_list_marker(&mut self, node_id: usize) {
        let node = match self.tree.nodes.get(node_id) {
            Some(n) => n,
            None => return,
        };

        // Only process <li> elements
        if node.tag() != Some("li") {
            return;
        }

        // Check Stylo computed list-style-type — skip if "none"
        // (list-style-type is inherited, so checking the <li> covers both
        // inline `list-style: none` and inherited from parent <ul>/<ol>)
        {
            use style::properties::longhands::list_style_type::computed_value::T as ListStyleType;
            let stylo_data = node.stylo_element_data.borrow();
            if let Some(ref data) = *stylo_data {
                if let Some(ref primary) = data.styles.primary {
                    if primary.get_list().list_style_type == ListStyleType::None {
                        return;
                    }
                }
            }
        }

        // Skip if already has a pseudo-element child (from CSS ::before)
        let has_pseudo = node.children.iter().any(|&cid| {
            self.tree
                .nodes
                .get(cid)
                .is_some_and(|n| n.is_pseudo_element)
        });
        if has_pseudo {
            return;
        }

        let parent_id = match node.parent {
            Some(p) => p,
            None => return,
        };

        let parent_tag = self
            .tree
            .nodes
            .get(parent_id)
            .and_then(|n| n.tag())
            .unwrap_or("");

        // Determine marker text based on parent list type.
        // Use en-space (\u{2002}) after the marker — wider than a normal space
        // and not subject to whitespace collapsing in the IFC.
        let marker_text = if parent_tag == "ol" {
            let counters = self.compute_list_item_counters(node_id);
            let num = counters.get("list-item").copied().unwrap_or(1);
            format!("{}.\u{2002}", num)
        } else if parent_tag == "ul" {
            "\u{2022}\u{2002}".to_string() // bullet + en-space
        } else {
            return; // Not inside a list
        };

        // Create the marker pseudo-element (span + text node)
        use rinch_core::dom::DomDocument;
        let span_id = self.create_element("span");
        let text_node_id = self.create_text(&marker_text);

        // Append text node to span
        {
            let span_raw = span_id.0;
            let text_raw = text_node_id.0;
            self.tree.nodes[text_raw].parent = Some(span_raw);
            self.tree.nodes[span_raw].children.push(text_raw);
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[span_raw].taffy_id,
                self.tree.nodes[text_raw].taffy_id,
            ) {
                let _ = self.tree.taffy.add_child(parent_taffy, child_taffy);
            }
        }

        // Mark as pseudo-element, inherit parent's computed style
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].computed_style =
                self.tree.nodes[node_id].computed_style.clone();
            self.tree.nodes[span_raw].is_pseudo_element = true;
            self.tree.style_dirty_nodes.push(span_raw);
        }

        // Insert as first child of the <li>
        {
            let span_raw = span_id.0;
            self.tree.nodes[span_raw].parent = Some(node_id);
            self.tree.nodes[node_id].children.insert(0, span_raw);
            if let (Some(parent_taffy), Some(child_taffy)) = (
                self.tree.nodes[node_id].taffy_id,
                self.tree.nodes[span_raw].taffy_id,
            ) {
                let _ = self
                    .tree
                    .taffy
                    .insert_child_at_index(parent_taffy, 0, child_taffy);
            }
        }
    }
}
