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
    pub(crate) fn extract_content_text(computed: &ComputedValues) -> String {
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
}
