//! Style resolution: Stylo CSS cascade for computing element styles.

use servo_arc::Arc as ServoArc;

use style::properties::ComputedValues;

use crate::RinchDocument;

impl RinchDocument {
    /// Resolve styles for elements that need it.
    ///
    /// When `style_roots` is populated (the common case for incremental
    /// updates), only those subtrees are visited — O(changed_subtree)
    /// instead of O(tree).  Falls back to a full tree walk when no
    /// roots are tracked (initial render, stylesheet reload, viewport
    /// resize).
    pub fn resolve_styles(&mut self) {
        use crate::stylo_impl::RinchNode;
        use style::shared_lock::StylesheetGuards;

        // Flush any pending stylesheet changes
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            self.stylist.flush::<RinchNode>(&guards, None, None);
        }

        let roots = std::mem::take(&mut self.tree.style_roots);

        // Full tree walk when:
        // - No specific roots tracked (stylesheet reload, viewport resize)
        // - First layout hasn't completed yet (DOM still being constructed,
        //   parent classes may not be resolved when children are appended)
        if roots.is_empty() || !self.tree.transitions_enabled {
            let html_id = self.tree.html_id;
            self.resolve_styles_recursive(html_id, None);
            return;
        }

        // Targeted resolution: only visit the invalidated subtrees.
        //
        // Entries whose node is **not connected to the document are dropped**
        // (#651, #668). "What is this element's style?" is a question CSS only
        // answers for elements in a document, and the answer this path was
        // inventing for the rest was wrong in both directions: with no ancestor
        // chain, `find_parent_computed_style` answers `None`, so every
        // inherited property cascades to its *initial* value and no descendant
        // selector can match.
        //
        // - **Unmount (#668).** A removed subtree holding a pending entry
        //   recascaded to `font-family: serif` / `color: black`, and
        //   `build_ifc_layouts` — which collects its roots from the whole slab
        //   (#628) — then reshaped its text from those values, every layout.
        // - **Mount (#651).** A component's child is classed before it is
        //   spliced in; anything that resolves in the window between drains
        //   that entry, cascades the child parentless, and sets
        //   `has_been_styled`. Its first *attached* resolution is then a
        //   **change** on an already-styled node, which is exactly what
        //   `transition` waits for — a visible wrong animation on every mount
        //   of a component sized by a modifier class on its wrapper
        //   (`Checkbox`, `Switch`, `Select`). Skipping leaves the node
        //   untouched by `apply_stylo_styles_to_taffy` as well, so
        //   `has_been_styled` stays `false` and the splice is its first style,
        //   which is the web's own rule.
        //
        // A dropped entry is not a lost resolution: every route that connects a
        // node — `append_child`, `insert_before`, `insert_child`,
        // `replace_node` — ends in `recompute_node_styles_recursive`, which
        // invalidates the whole inserted subtree and pushes it as a root of its
        // own. And connectivity is asked **here**, at resolve time, not where
        // the entry was pushed, so a node classed while detached and spliced in
        // before the next resolve is still carried by that same entry.
        //
        // The list is not filtered on removal. `remove_node` could drop the
        // subtree's entries eagerly, but it is one of four detach routes
        // (`remove_child`, a reparenting `append_child`/`insert_before` and
        // `replace_node`'s implicit detach of `old` are the others), so an
        // eager drop there would be a partial cure that reads as a complete
        // one — and it can remove no work this skip does not already remove.
        //
        // Sort by depth (shallowest first) so that if both a parent and
        // child appear, the parent is resolved first and the child can
        // be skipped (it will be covered by the parent's subtree walk).
        let mut sorted: Vec<(usize, usize)> = roots
            .into_iter()
            .filter_map(|id| Some((id, self.depth_if_connected(id)?)))
            .collect();
        sorted.sort_unstable_by_key(|&(_, depth)| depth);
        sorted.dedup_by_key(|entry| entry.0);

        // Track which roots have been resolved so we can skip
        // descendants that are already covered.
        let mut resolved_roots: Vec<usize> = Vec::with_capacity(sorted.len());

        for (root_id, _depth) in sorted {
            // If this root is a descendant of an already-resolved root,
            // it was already handled by that root's subtree walk.
            if self.is_ancestor_in(&resolved_roots, root_id) {
                continue;
            }

            let parent_style = self.find_parent_computed_style(root_id);
            self.resolve_styles_recursive(root_id, parent_style);
            resolved_roots.push(root_id);
        }
    }

    /// Walk up to find the nearest ancestor with a valid computed style.
    fn find_parent_computed_style(&self, node_id: usize) -> Option<ServoArc<ComputedValues>> {
        let mut current = self.tree.nodes.get(node_id)?.parent;
        while let Some(pid) = current {
            let node = self.tree.nodes.get(pid)?;
            let data = node.stylo_element_data.borrow();
            if let Some(style) = data.as_ref().and_then(|d| d.styles.primary.clone()) {
                return Some(style);
            }
            current = node.parent;
        }
        None
    }

    /// Check whether `node_id` is a descendant of any node in `ancestors`.
    fn is_ancestor_in(&self, ancestors: &[usize], node_id: usize) -> bool {
        let mut current = self.tree.nodes.get(node_id).and_then(|n| n.parent);
        while let Some(pid) = current {
            if ancestors.contains(&pid) {
                return true;
            }
            current = self.tree.nodes.get(pid).and_then(|n| n.parent);
        }
        false
    }

    /// The node's depth below the document node (0 = the document node
    /// itself), or `None` when the node is **not connected to it**.
    ///
    /// One walk answers both questions, which is the whole reason they share a
    /// function: the depth is only wanted for a node that has one.
    ///
    /// The anchor is `tree.root_id` — the document node, `<html>`'s parent —
    /// rather than `html_id`, so this refuses nothing the targeted path used to
    /// resolve. A node parented directly to the document node is outside the
    /// full-tree walk (which starts at `html_id`) and inside this one; that
    /// asymmetry is unchanged from before the connectivity test existed.
    /// `a_sibling_of_html_is_connected_because_the_anchor_is_the_document_node`
    /// is the pin, and it fails against an `html_id` anchor.
    ///
    /// The `node_id == root_id` self-case is not decoration: without it the walk
    /// starts at the document node's parent, finds `None`, and answers
    /// "detached" for the one node that *is* the document — so an entry for it
    /// would be dropped and the recascade it asks for would not happen.
    /// `the_document_nodes_own_entry_is_resolved` is the pin.
    ///
    /// Both of those shapes are reachable only by handing a DOM method the
    /// document node's own id, which nothing in rinch does. They are pinned
    /// because they are the only things that tell the two anchors apart.
    ///
    /// A `None` also covers an id that is no longer in the slab, or whose
    /// ancestor chain leaves it — a `style_roots` entry outlives the node it
    /// names.
    fn depth_if_connected(&self, node_id: usize) -> Option<usize> {
        if node_id == self.tree.root_id {
            return self.tree.nodes.get(node_id).map(|_| 0);
        }
        let mut depth = 0;
        let mut current = self.tree.nodes.get(node_id)?.parent;
        while let Some(pid) = current {
            depth += 1;
            if pid == self.tree.root_id {
                return Some(depth);
            }
            current = self.tree.nodes.get(pid)?.parent;
        }
        None
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
                // Remove from taffy parent (use safe version — the child may
                // have already been detached by setup_inline_formatting_contexts)
                if let (Some(parent_taffy), Some(child_taffy)) = (
                    self.tree.nodes[node_id].taffy_id,
                    self.tree.nodes[cid].taffy_id,
                ) {
                    self.taffy_remove_child_safe(parent_taffy, child_taffy);
                }
                // Remove the pseudo-element's subtree from the slab
                self.tree.remove_subtree(cid);
                // Remove from parent's children list
                self.tree.nodes[node_id].children.retain(|&c| c != cid);
            }
        }

        // Clear sensitivity flags before re-resolution so Stylo's matching
        // can re-set them accurately for the current class/selector state.
        self.tree.nodes[node_id].hover_sensitive.set(false);
        self.tree.nodes[node_id].active_sensitive.set(false);
        self.tree.nodes[node_id].focus_sensitive.set(false);

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

        // The root element's computed font-size is the basis every `rem`
        // length resolves against. Stylo feeds it to the Device itself in
        // `finish_restyle`, but that lives in stylo's own traversal and
        // rinch-dom hand-rolls the cascade, so it never runs — do it here
        // whenever the root is (re)cascaded (issue #279). This must happen
        // before recursing into children so descendants cascade against the
        // fresh value.
        if node_id == self.tree.html_id {
            self.sync_root_font_size(&computed);
        }

        // Check for ::before and ::after pseudo-elements
        use style::selector_parser::PseudoElement;
        self.resolve_pseudo_element(node_id, &computed, PseudoElement::Before);
        self.resolve_pseudo_element(node_id, &computed, PseudoElement::After);

        // Generate list markers for <li> elements (if no CSS ::before exists)
        self.resolve_list_marker(node_id);

        // Re-read children list since pseudo-element resolution may have added nodes
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();

        // Now we can recurse without holding borrows
        for child_id in children {
            self.resolve_styles_recursive(child_id, Some(computed.clone()));
        }
    }

    /// Feed the root (`<html>`) element's computed font-size back to the
    /// Stylo `Device` as the `rem` basis, mirroring what stylo's own
    /// `finish_restyle` does for documents it traverses (issue #279).
    ///
    /// When the basis actually changes, every cached descendant style may
    /// hold a `rem` length resolved against the old value, so all descendant
    /// caches are cleared; the caller is mid-walk at the root, so this same
    /// walk recascades them. (Stylo gates the recascade on
    /// `Device::used_root_font_size()`; we skip that optimization because the
    /// flag resets to `false` with every Device rebuild — a missed recascade
    /// would leave stale `rem` layout, while a spurious one only costs time
    /// on an event as rare as a root font-size change.)
    fn sync_root_font_size(&mut self, root_style: &ServoArc<ComputedValues>) {
        let device = self.stylist.device();
        // Keep the root style pointer fresh too — root font metrics
        // (rex/rch/ric) resolve through it.
        device.set_root_style(root_style);

        let size = root_style
            .effective_zoom
            .unzoom(root_style.get_font().clone_font_size().computed_size().px());
        if size == self.device_params.root_font_size {
            return;
        }
        self.device_params.root_font_size = size;
        device.set_root_font_size(size);

        // Descendants cached before this change resolved `rem` against the
        // old basis — clear them so the walk we're inside recascades them.
        let html_id = self.tree.html_id;
        for (node_id, _) in self.tree.nodes.iter() {
            if node_id == html_id {
                continue;
            }
            *self.tree.nodes[node_id].stylo_element_data.borrow_mut() = None;
        }
        // A recascaded `rem` length is a Taffy style change; make sure the
        // relayout isn't skipped when nothing else marked layout dirty.
        self.tree.layout_dirty = true;
    }
}
