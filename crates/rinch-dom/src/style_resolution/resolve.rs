//! Style resolution: Stylo CSS cascade for computing element styles.

use servo_arc::Arc as ServoArc;

use style::properties::ComputedValues;

use style::computed_value_flags::ComputedValueFlags;
use style::invalidation::element::restyle_hints::RestyleHint;

use crate::RinchDocument;
use crate::node::DirtyFlags;

/// What an element's new style requires of its children's styles.
///
/// Ordered: each variant asks for at least what the one before it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChildCascade {
    /// Nothing a child inherits changed.
    Skip,
    /// Only reset properties changed; a child that explicitly `inherit`s one
    /// (`ComputedValueFlags::INHERITS_RESET_STYLE`) is re-cascaded.
    IfInheritReset,
    /// Something inherited changed: every child is re-cascaded (and each
    /// decides for its own children).
    Cascade,
    /// Every descendant is re-matched and re-cascaded — a restyle hint for the
    /// whole subtree (`RESTYLE_DESCENDANTS`).
    Subtree,
}

/// Compare an element's style before and after a cascade, and answer what its
/// children need — Stylo's `accumulate_damage_for` / `ChildRestyleRequirement`,
/// which lives in Stylo's own traversal and so never runs for rinch's.
///
/// Children must follow when anything they can read from the parent changed:
/// an inherited style struct (font, inherited text / box / table / UI, list),
/// a custom property, the inherited computed-value flags (text-decoration
/// propagation among them), the effective zoom, the writing mode, or the
/// `display` (blockification of children and `display: contents` read it).
/// Otherwise only a child that explicitly inherits a reset property can
/// differ. Servo's own `compute_style_difference` never says "reset only"
/// (a FIXME there), so it would re-cascade every child on any change — this
/// is what lets a `:hover { background }` cascade one element instead of its
/// subtree.
pub(crate) fn child_cascade(old: Option<&ComputedValues>, new: &ComputedValues) -> ChildCascade {
    let Some(old) = old else {
        return ChildCascade::Cascade;
    };
    if std::ptr::eq(old, new) {
        return ChildCascade::Skip;
    }
    fn same<T: PartialEq>(a: &T, b: &T) -> bool {
        std::ptr::eq(a, b) || a == b
    }
    let inherited_same = old.flags.maybe_inherited() == new.flags.maybe_inherited()
        && old.effective_zoom == new.effective_zoom
        && old.writing_mode == new.writing_mode
        && same(old.get_font(), new.get_font())
        && same(old.get_inherited_text(), new.get_inherited_text())
        && same(old.get_inherited_box(), new.get_inherited_box())
        && same(old.get_inherited_table(), new.get_inherited_table())
        && same(old.get_inherited_ui(), new.get_inherited_ui())
        && same(old.get_list(), new.get_list())
        && old.custom_properties_equal(new)
        && old.clone_display() == new.clone_display();
    if !inherited_same {
        return ChildCascade::Cascade;
    }
    let reset_same = same(old.get_background(), new.get_background())
        && same(old.get_border(), new.get_border())
        && same(old.get_box(), new.get_box())
        && same(old.get_column(), new.get_column())
        && same(old.get_counters(), new.get_counters())
        && same(old.get_effects(), new.get_effects())
        && same(old.get_margin(), new.get_margin())
        && same(old.get_outline(), new.get_outline())
        && same(old.get_padding(), new.get_padding())
        && same(old.get_position(), new.get_position())
        && same(old.get_svg(), new.get_svg())
        && same(old.get_table(), new.get_table())
        && same(old.get_text(), new.get_text())
        && same(old.get_ui(), new.get_ui());
    if reset_same {
        ChildCascade::Skip
    } else {
        ChildCascade::IfInheritReset
    }
}

impl RinchDocument {
    /// Resolve styles for elements that need it.
    ///
    /// When `style_roots` is populated (the common case for incremental
    /// updates), only those subtrees are visited — O(changed_subtree)
    /// instead of O(tree).  Falls back to a full tree walk when no
    /// roots are tracked (initial render, stylesheet reload, viewport
    /// resize).
    pub fn resolve_styles(&mut self) {
        self.tree.hit_cache.invalidate();
        let t = web_time::Instant::now();
        self.tree.perf.bump(crate::perf::Counter::StyleResolves);
        self.resolve_styles_inner();
        self.tree
            .perf
            .add_elapsed(crate::perf::Counter::TimeStyleNs, t);
    }

    fn resolve_styles_inner(&mut self) {
        use crate::stylo_impl::RinchNode;
        use style::shared_lock::StylesheetGuards;

        // Flush any pending stylesheet changes
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            self.stylist.flush::<RinchNode>(&guards, None, None);
        }
        self.refresh_pseudo_rule_presence();
        // Attribute and state changes since the last resolve: Stylo's
        // invalidator turns their snapshots into restyle hints and style
        // roots (`invalidation.rs`).
        self.process_snapshots();

        let roots = std::mem::take(&mut self.tree.style_roots);

        // Full tree walk when:
        // - a whole-document restyle asked for one (`full_style_walk`: a
        //   stylesheet, theme, device pixel ratio or media-query change, and
        //   the very first resolve);
        // - the first layout hasn't completed yet (DOM still being constructed,
        //   parent classes may not be resolved when children are appended).
        //
        // An empty `style_roots` alone is **not** a reason to walk: it means
        // nothing was invalidated. It used to be one, and a synchronous
        // insertion restyle — which consumes its own root — left the list
        // empty with `styles_dirty` still set, so the next frame's resolve
        // visited every node in the document and cascaded none of them.
        if self.tree.full_style_walk || !self.tree.transitions_enabled {
            self.tree.full_style_walk = false;
            self.tree.perf.bump(crate::perf::Counter::FullStyleWalks);
            let html_id = self.tree.html_id;
            // The one place the filter is zeroed outright: a rare
            // whole-document walk, which also discards any counts a walk that
            // unwound early (a panic) may have left behind.
            self.style_bloom.clear();
            self.style_bloom_filled.clear();
            self.resolve_styles_recursive(html_id, None, ChildCascade::Skip, true);
            return;
        }
        if roots.is_empty() {
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
        // **The `style_roots` list is still not filtered on removal**, and that
        // is unchanged by #699. `remove_node` could drop the subtree's entries
        // eagerly, but it is one of four detach routes (`remove_child`,
        // `replace_node`'s implicit detach of `old`, and `set_text_content`'s
        // orphaning of an element's children are the others), so an eager drop
        // there would be a partial cure that reads as a complete one — and it
        // can remove no work this skip does not already remove.
        //
        // #699 does hook all four of those routes
        // (`RinchDocument::detach_subtree_styles`), and it is a different
        // question with a different answer: it resets `has_been_styled` and
        // cancels running transitions and animations, because a subtree that
        // left the document has no *before-change style*. It touches neither
        // this list nor the node's `computed_style`. A reparenting
        // `append_child` / `insert_before` / `insert_child` is deliberately not
        // hooked — those are moves, and the node is connected again before the
        // call returns. **Unless the destination is detached**, in which case it
        // is not: #702 hooks those three and `replace_node` through
        // `detach_subtree_styles_if_moved_out`, which asks exactly the
        // connectivity question this skip asks, through
        // `depth_if_connected` — the same walk, so the two cannot disagree
        // about what "connected" means.
        //
        // Sort by depth (shallowest first): a root's ancestors are resolved
        // before it, so the parent style it cascades against is current, and
        // a root an earlier walk already reached — every node a walk cascades
        // has its hint and dirty-descendants bit cleared — is skipped as
        // having nothing left to do.
        let mut sorted: Vec<(usize, usize)> = roots
            .into_iter()
            .filter_map(|id| Some((id, self.depth_if_connected(id)?)))
            .collect();
        sorted.sort_unstable_by_key(|&(_, depth)| depth);
        sorted.dedup_by_key(|entry| entry.0);

        for (root_id, _depth) in sorted {
            // A non-element root (the document node) is walked for whatever
            // below it needs a visit.
            if self.tree.nodes[root_id].is_element() && !self.node_needs_style_visit(root_id) {
                continue;
            }
            let parent_style = self.find_parent_computed_style(root_id);
            self.fill_style_bloom_for(root_id);
            self.resolve_styles_recursive(root_id, parent_style, ChildCascade::Skip, false);
        }
    }

    /// Mark a node the style walk re-cascaded as paint-dirty. Only the paint
    /// list, not `dirty_nodes`: the walk runs inside `resolve_layout`, after
    /// the shell has taken `dirty_nodes`, and an entry left there would read as
    /// "something is pending" and ask the next idle frame for a redraw.
    pub(crate) fn mark_restyled_for_paint(&mut self, node_id: usize) {
        if let Some(node) = self.tree.nodes.get_mut(node_id) {
            node.dirty.insert(DirtyFlags::STYLE | DirtyFlags::PAINT);
            self.tree.paint_dirty_nodes.push(node_id);
        }
    }

    /// Cascade just the freshly inserted (and unstyled) subtree at `node_id`,
    /// leaving every other pending style change for the next
    /// `resolve_styles`. Answers `false`, doing nothing, when that would be
    /// wrong and the caller should run a full `resolve_styles` instead:
    ///
    /// - no layout has completed yet, or a whole-document walk is pending;
    /// - the node is not connected (a detached node is not styled at all —
    ///   `resolve_styles` drops such a root, #651);
    /// - some ancestor is itself waiting for a restyle (no style, a restyle
    ///   hint, a pending snapshot, or a marked descendant path): the subtree
    ///   would cascade against an ancestor style this frame is about to
    ///   replace, and its next cascade would then read as a *change* — a
    ///   transition on a node that was only just inserted.
    pub(crate) fn resolve_inserted_subtree(&mut self, node_id: usize) -> bool {
        use crate::stylo_impl::RinchNode;
        use style::shared_lock::StylesheetGuards;

        if self.tree.full_style_walk || !self.tree.transitions_enabled {
            return false;
        }
        if self.depth_if_connected(node_id).is_none() {
            return false;
        }
        let mut current = self.tree.nodes[node_id].parent;
        while let Some(id) = current {
            let n = &self.tree.nodes[id];
            if n.is_element() {
                if n.has_snapshot || n.style_dirty_descendants.get() {
                    return false;
                }
                let data = n.stylo_element_data.borrow();
                match data.as_ref() {
                    Some(d) if d.styles.primary.is_some() && d.hint.is_empty() => {}
                    _ => return false,
                }
            }
            current = n.parent;
        }

        self.tree.hit_cache.invalidate();
        let t = web_time::Instant::now();
        self.tree.perf.bump(crate::perf::Counter::StyleResolves);
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            self.stylist.flush::<RinchNode>(&guards, None, None);
        }
        self.refresh_pseudo_rule_presence();
        let parent_style = self.find_parent_computed_style(node_id);
        self.fill_style_bloom_for(node_id);
        self.resolve_styles_recursive(node_id, parent_style, ChildCascade::Skip, false);
        self.tree
            .perf
            .add_elapsed(crate::perf::Counter::TimeStyleNs, t);
        true
    }

    /// Whether the style walk has anything to do at `node_id`: it has no
    /// style yet, carries a restyle hint, or some descendant does.
    fn node_needs_style_visit(&self, node_id: usize) -> bool {
        let Some(node) = self.tree.nodes.get(node_id) else {
            return false;
        };
        if !node.is_element() {
            return false;
        }
        if node.style_dirty_descendants.get() {
            return true;
        }
        let data = node.stylo_element_data.borrow();
        match data.as_ref() {
            None => true,
            Some(d) => d.styles.primary.is_none() || !d.hint.is_empty(),
        }
    }

    /// Reset the style bloom filter to hold exactly `node_id`'s ancestor
    /// elements, for a walk starting at `node_id`.
    ///
    /// A walk leaves the filter as it found it (`resolve_style_children`
    /// pops every element it pushed), so what is in it on entry is exactly
    /// the previous fill's ancestor chain, whose hashes `style_bloom_filled`
    /// recorded. Removing those is a handful of counter updates where
    /// `clear()` zeroed all 4096 of them — on every hover and every
    /// insertion. The recorded hashes are removed, not recomputed, so an
    /// ancestor whose attributes changed since cannot unbalance a counter.
    fn fill_style_bloom_for(&mut self, node_id: usize) {
        use selectors::bloom::BLOOM_HASH_MASK;
        for hash in self.style_bloom_filled.drain(..) {
            self.style_bloom.remove_hash(hash);
        }
        let mut current = self.tree.nodes.get(node_id).and_then(|n| n.parent);
        while let Some(id) = current {
            if self.tree.nodes[id].is_element() {
                let bloom = &mut self.style_bloom;
                let filled = &mut self.style_bloom_filled;
                style::bloom::each_relevant_element_hash(
                    crate::stylo_impl::RinchNode::new(id, &self.tree),
                    |hash| {
                        let hash = hash & BLOOM_HASH_MASK;
                        bloom.insert_hash(hash);
                        filled.push(hash);
                    },
                );
            }
            current = self.tree.nodes[id].parent;
        }
    }

    /// Add `node_id`'s tag, id, classes and attribute names to the style bloom
    /// filter, which descendant-combinator matching consults to reject a
    /// selector whose ancestor compounds no ancestor can satisfy.
    fn push_style_bloom(&mut self, node_id: usize) {
        use selectors::bloom::BLOOM_HASH_MASK;
        let bloom = &mut self.style_bloom;
        style::bloom::each_relevant_element_hash(
            crate::stylo_impl::RinchNode::new(node_id, &self.tree),
            |hash| bloom.insert_hash(hash & BLOOM_HASH_MASK),
        );
    }

    /// Undo [`Self::push_style_bloom`] for `node_id` (its hashes are recomputed
    /// from the same, unchanged attributes).
    fn pop_style_bloom(&mut self, node_id: usize) {
        use selectors::bloom::BLOOM_HASH_MASK;
        let bloom = &mut self.style_bloom;
        style::bloom::each_relevant_element_hash(
            crate::stylo_impl::RinchNode::new(node_id, &self.tree),
            |hash| bloom.remove_hash(hash & BLOOM_HASH_MASK),
        );
    }

    /// Walk `node_id`'s children after `node_id` was visited with `style`:
    /// every child when `cascade` forces some or `visit_all` is set, otherwise
    /// only those [`Self::node_needs_style_visit`] names.
    fn resolve_style_children(
        &mut self,
        node_id: usize,
        style: &ServoArc<ComputedValues>,
        cascade: ChildCascade,
        visit_all: bool,
    ) {
        let children: Vec<usize> = self.tree.nodes[node_id].children.clone();
        let mut pushed = false;
        for child_id in children {
            let visit = visit_all
                || (cascade != ChildCascade::Skip && self.tree.nodes[child_id].is_element())
                || self.node_needs_style_visit(child_id);
            if !visit {
                continue;
            }
            if !pushed {
                self.push_style_bloom(node_id);
                pushed = true;
            }
            self.resolve_styles_recursive(child_id, Some(style.clone()), cascade, visit_all);
        }
        if pushed {
            self.pop_style_bloom(node_id);
        }
    }

    /// Recompute [`Self::has_before_rules`] / [`Self::has_after_rules`] from
    /// the flushed cascade data: whether any origin has a `::before` /
    /// `::after` rule at all. A handful of hash lookups per resolve.
    fn refresh_pseudo_rule_presence(&mut self) {
        use style::selector_parser::PseudoElement;
        let has = |pseudo: PseudoElement| {
            self.stylist.iter_origins().any(|(data, _)| {
                data.normal_rules(std::slice::from_ref(&pseudo))
                    .is_some_and(|map| !map.is_empty())
            })
        };
        self.has_before_rules = has(PseudoElement::Before);
        self.has_after_rules = has(PseudoElement::After);
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

    /// The **layout parent** style of `node_id`, given the style of its DOM
    /// parent: that style, unless it is `display: contents`, in which case
    /// the style of the nearest styled ancestor that is not (#998).
    ///
    /// Stylo's `StyleAdjuster` blockifies an element whose *layout* parent is
    /// a flex or grid container (`blockify_if_necessary`). A
    /// `display: contents` element generates no box, so its children are
    /// items of *its* parent's container (css-display-3 §2.5), and Stylo's own
    /// traversal hands the adjuster the nearest non-contents ancestor for
    /// exactly that reason. rinch hand-rolls the cascade and used to pass the
    /// DOM parent in both slots, so an inline-level child of an `rsx!` wrapper
    /// inside a `Stack` or `Group` kept its inline display.
    ///
    /// Inheritance still comes from the DOM parent — only the second slot
    /// changes. The ancestors are read from their stored styles, which the
    /// walk writes before it descends, so they are this pass's.
    pub(crate) fn layout_parent_style(
        &self,
        node_id: usize,
        parent_style: Option<&ServoArc<ComputedValues>>,
    ) -> Option<ServoArc<ComputedValues>> {
        let parent = parent_style?;
        if !parent.clone_display().is_contents() {
            return Some(parent.clone());
        }
        let start = self.tree.nodes.get(node_id).and_then(|n| n.parent);
        self.nearest_non_contents_style(start)
            .or_else(|| Some(parent.clone()))
    }

    /// The stored style of `start` or its nearest ancestor that has one and is
    /// not `display: contents`.
    pub(crate) fn nearest_non_contents_style(
        &self,
        start: Option<usize>,
    ) -> Option<ServoArc<ComputedValues>> {
        let mut current = start;
        while let Some(id) = current {
            let node = self.tree.nodes.get(id)?;
            let style = node
                .stylo_element_data
                .borrow()
                .as_ref()
                .and_then(|d| d.styles.primary.clone());
            if let Some(style) = style
                && !style.clone_display().is_contents()
            {
                return Some(style);
            }
            current = node.parent;
        }
        None
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
    pub(crate) fn depth_if_connected(&self, node_id: usize) -> Option<usize> {
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
    ///
    /// `inherited` is what the parent's cascade requires of this node (its
    /// [`child_cascade`] answer). A node is cascaded when it has no style,
    /// carries a restyle hint, or `inherited` asks; its children are then
    /// walked as its own `child_cascade` requires, and otherwise only where
    /// something below is marked (`style_dirty_descendants`).
    pub(crate) fn resolve_styles_recursive(
        &mut self,
        node_id: usize,
        parent_style: Option<ServoArc<ComputedValues>>,
        inherited: ChildCascade,
        visit_all: bool,
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

        let Some(node) = self.tree.nodes.get(node_id) else {
            return;
        };
        self.tree.perf.bump(crate::perf::Counter::StyleNodesVisited);
        if !node.is_element() {
            // Text and comment nodes carry no style and have no children; the
            // document node has children and passes the walk on to them.
            if !node.children.is_empty() {
                let children = node.children.clone();
                for child_id in children {
                    if visit_all || self.node_needs_style_visit(child_id) {
                        self.resolve_styles_recursive(
                            child_id,
                            parent_style.clone(),
                            inherited,
                            visit_all,
                        );
                    }
                }
            }
            return;
        }
        let dirty_descendants = node.style_dirty_descendants.replace(false);
        let (old_style, hint) = {
            let mut data = node.stylo_element_data.borrow_mut();
            match data.as_mut() {
                Some(d) => (
                    d.styles.primary.clone(),
                    std::mem::replace(&mut d.hint, RestyleHint::empty()),
                ),
                None => (None, RestyleHint::empty()),
            }
        };
        let needs_cascade = match &old_style {
            None => true,
            Some(old) => {
                !hint.is_empty()
                    || inherited >= ChildCascade::Cascade
                    || (inherited == ChildCascade::IfInheritReset
                        && old.flags.contains(ComputedValueFlags::INHERITS_RESET_STYLE))
            }
        };
        let subtree = inherited == ChildCascade::Subtree
            || hint
                .intersects(RestyleHint::RESTYLE_DESCENDANTS | RestyleHint::RECASCADE_DESCENDANTS);

        // Nothing to redo here: the style stays, and the walk goes on only
        // where something below is marked.
        if !needs_cascade {
            let computed = old_style.expect("a node with no style is always cascaded");
            if dirty_descendants || visit_all {
                self.resolve_style_children(node_id, &computed, ChildCascade::Skip, visit_all);
            }
            return;
        }

        // Remove existing pseudo-element children before re-resolving styles
        // to avoid duplicates when styles are recomputed.
        let had_pseudo;
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
            had_pseudo = !children_to_remove.is_empty();
            for cid in children_to_remove {
                // The IFC the generated box was laid out in names it; see the
                // invalidation after the pseudo-elements are re-resolved.
                if let Some(root) = self.tree.nodes[cid].ifc_root {
                    self.invalidate_ifc_root(root);
                }
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

        // The `*_sensitive` flags are **not** cleared here. They are set on
        // whatever element a `:hover` / `:active` / `:focus` compound is
        // evaluated against — which for `.card:hover .title` is the *card*,
        // while the title is being matched. A card re-cascaded on its own (its
        // class changed, nothing below it did) would lose the flag its
        // descendants' matching set, and its hover would then invalidate
        // nothing. They only ever over-state a dependency, which costs a
        // snapshot that Stylo's invalidator answers with no restyle.

        // The style Stylo's adjuster blockifies against: the parent's, unless
        // the parent is `display: contents` (#998). See
        // [`Self::layout_parent_style`].
        let layout_parent_style = self.layout_parent_style(node_id, parent_style.as_ref());

        // Compute styles in a block so borrows are dropped before recursion
        let computed = {
            // Create the RinchNode wrapper for Stylo
            let rinch_node = RinchNode::new(node_id, &self.tree);

            // Set up matching context
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);

            let mut selector_caches = SelectorCaches::default();
            // The bloom filter holds this node's ancestors (the walk pushes
            // each element before descending into it), so a descendant
            // combinator whose ancestor compounds no ancestor carries is
            // rejected without walking the chain. Selector flags are
            // recorded: they are how an insertion or removal knows which
            // siblings a structural selector ties to it (`invalidation.rs`).
            let mut matching_context = MatchingContext::new_for_visited(
                MatchingMode::Normal,
                Some(&*self.style_bloom),
                &mut selector_caches,
                VisitedHandlingMode::AllLinksUnvisited,
                IncludeStartingStyle::No,
                self.stylist.quirks_mode(),
                NeedsSelectorFlags::Yes,
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
            let layout_parent_style_ref = layout_parent_style.as_deref();
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
                parent_style_ref,        // parent_style
                layout_parent_style_ref, // layout_parent_style
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
            self.tree.perf.bump(crate::perf::Counter::ElementsCascaded);

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

        // Viewport-unit usage, for `restyle_for_viewport_change`. The pseudo
        // resolutions below OR their own usage in.
        self.tree.nodes[node_id].uses_viewport_units.set(
            computed
                .flags
                .intersects(style::computed_value_flags::ComputedValueFlags::USES_VIEWPORT_UNITS),
        );

        // Re-derived by the pseudo passes below.
        self.tree.nodes[node_id].content_reads_attrs.set(false);

        // Check for ::before and ::after pseudo-elements — only when some
        // stylesheet has a rule for that pseudo at all. The UA sheet has none,
        // so an app that declares none pays nothing here.
        use style::selector_parser::PseudoElement;
        if self.has_before_rules {
            self.tree
                .perf
                .bump(crate::perf::Counter::PseudoElementPasses);
            self.resolve_pseudo_element(node_id, &computed, PseudoElement::Before);
        }
        if self.has_after_rules {
            self.tree
                .perf
                .bump(crate::perf::Counter::PseudoElementPasses);
            self.resolve_pseudo_element(node_id, &computed, PseudoElement::After);
        }

        // Generate list markers for <li> elements (if no CSS ::before exists)
        self.resolve_list_marker(node_id);

        // The pseudo-element children were just freed and minted again, so the
        // inline layout that holds them — this node's own IFC, or the one it is
        // an inline member of — names nodes that no longer exist, or that the
        // slab has since handed to someone else. The cascade's per-node text
        // comparison cannot see that: nothing about *this* node's style had to
        // change for it to happen. It used to be covered by accident, by an
        // eager drop of every text layout under any restyled node. Only nodes
        // that actually carry generated content pay for it.
        let has_pseudo = self.tree.nodes[node_id]
            .children
            .iter()
            .any(|&c| self.tree.nodes.get(c).is_some_and(|n| n.is_pseudo_element));
        if had_pseudo || has_pseudo {
            // The generated children are new nodes (or gone): a structural
            // change to this node's child list, which the structural pass has
            // to be told about like any other — and a layout owed, which is
            // the half nothing else supplies when the content went away. A
            // regenerated node's first cascade seeds and dirties layout on its
            // own (its Taffy style is new), so for a *replaced* pseudo-element
            // this is redundant; for one **removed** with nothing in its place
            // it is the only notice, and without it no layout ran at all: the
            // freed node stayed a member of its anonymous box's run
            // (`scoped_ifc_scenario_tests::probe_pseudo_content_removed_from_a_mixed_container`,
            // which fails on both passes without these two lines).
            self.tree
                .seed_ifc(node_id, crate::ifc_scope::IfcSeed::Children);
            self.tree.layout_dirty = true;
            if let Some(root) = self.tree.nodes[node_id].ifc_root {
                self.invalidate_ifc_root(root);
            }
            self.invalidate_ifc_root(node_id);
        }
        // Generated content that went away and did not come back (an `attr()`
        // whose attribute was removed, a class that took the `content` rule
        // away) is covered by the block above: `had_pseudo && !has_pseudo` is
        // one of its cases, and it seeds this node's region and owes a layout.
        // #894 fixed the same case with a bare `ifc_dirty = true`, which is a
        // whole-document structural pass per removal; the seed is exact.

        // A re-cascaded style is a paint change for this node's box: the
        // software renderer's dirty region must cover it.
        if old_style.is_some() {
            self.mark_restyled_for_paint(node_id);
        }

        // What the new style asks of the children (`child_cascade`), then the
        // walk. Pseudo-element resolution above may have added children.
        // A `display: contents` element's children are blockified against
        // *its* layout parent (`layout_parent_style`, #998), so a flex
        // container appearing or going above it has to reach them even though
        // the wrapper's own display did not move. It does, through the
        // `maybe_inherited` flags: Stylo marks a contents element in an item
        // container `DIPLAY_CONTENTS_IN_ITEM_CONTAINER` for exactly this.
        let cascade = if subtree {
            ChildCascade::Subtree
        } else {
            child_cascade(old_style.as_deref(), &computed)
        };
        self.resolve_style_children(node_id, &computed, cascade, visit_all);
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
        // …and remembered, so a Device rebuilt by a resize carries it too.
        self.device_params.root_style = Some(root_style.clone());

        let size = root_style
            .effective_zoom
            .unzoom(root_style.get_font().clone_font_size().computed_size().px());
        if size == self.device_params.root_font_size {
            return;
        }
        self.device_params.root_font_size = size;
        device.set_root_font_size(size);
        self.tree
            .note_full_restyle(crate::perf::FullRestyleReason::RootFontSize);

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
