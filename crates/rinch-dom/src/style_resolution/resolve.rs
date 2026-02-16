//! Style resolution: Stylo CSS cascade for computing element styles.

use servo_arc::Arc as ServoArc;

use style::properties::ComputedValues;

use crate::RinchDocument;

impl RinchDocument {
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
    pub(crate) fn resolve_styles_recursive(
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
    pub(crate) fn invalidate_style_subtree(&mut self, node_id: usize) {
        if let Some(node) = self.tree.nodes.get_mut(node_id) {
            *node.stylo_element_data.borrow_mut() = None;
            let children: Vec<usize> = node.children.clone();
            for child_id in children {
                self.invalidate_style_subtree(child_id);
            }
        }
    }
}
