//! Restyle invalidation: which elements a DOM change can restyle.
//!
//! An attribute, class, id or interaction-state change used to re-cascade the
//! changed element's **whole subtree**, whether or not any selector could see
//! the change, and never its **siblings** — so `.a + .b`, `.a ~ .b` and the
//! structural pseudo-classes (`:nth-child`, `:first-child`, `:last-child`,
//! `:only-child`, `:empty`) went stale on every class toggle, insertion and
//! removal. This module answers the question the way a browser does:
//!
//! - **Attribute and state changes go through Stylo's own invalidator.** Before
//!   a change lands, [`RinchDocument::note_attribute_change`] /
//!   [`RinchDocument::note_state_change`] take a snapshot of the element's
//!   attributes and state (a `ServoElementSnapshot`, once per element per
//!   frame). At the next `resolve_styles`, [`RinchDocument::process_snapshots`]
//!   runs `ElementData::invalidate_style_if_needed` for each snapshotted
//!   element against the whole snapshot map: Stylo looks the changed classes,
//!   id, attribute names and state bits up in the stylesheets' invalidation
//!   maps, and for every dependency whose selector matched before and does not
//!   now (or the reverse) marks exactly the affected element — self,
//!   descendants or later siblings — with a restyle hint. `resolve_styles`
//!   then follows the hints (and the dirty-descendant bits Stylo sets on the
//!   path to them) instead of re-cascading subtrees.
//! - **Child-list changes use the selector flags matching leaves on the
//!   parent** ([`RinchDocument::note_child_list_changed`]). Matching now runs
//!   with `NeedsSelectorFlags::Yes`, so a parent learns that some child was
//!   matched against `:nth-child`/`:nth-last-child` (`HAS_SLOW_SELECTOR*`), a
//!   sibling combinator (`HAS_SLOW_SELECTOR_LATER_SIBLINGS`), `:first-child` /
//!   `:last-child` / `:only-child` (`HAS_EDGE_CHILD_SELECTOR`), or that it was
//!   matched against `:empty` itself (`HAS_EMPTY_SELECTOR`). An insertion or
//!   removal then restyles what those flags say can change — Gecko's
//!   `RestyleForInsertOrChange` / `RestyleForRemove`.
//!
//! Things Stylo's maps cannot know about, and so are forced here:
//!
//! - `:disabled` / `:enabled` and `:link` / `:any-link` are answered from the
//!   `disabled` and `href` attributes (and `disabled` from a `<fieldset>`
//!   ancestor), not from element state: those attributes restyle the element,
//!   its subtree and its later siblings. `:checked` *is* element state
//!   (`stylo_impl::element_state`), so Stylo handles it precisely.
//! - An inline `style` attribute restyles its element.
//! - `rows` on a `<textarea>` feeds its intrinsic height; `start` on an
//!   `<ol>` and `value` on an `<li>` feed the list markers; any attribute on
//!   an element with generated content can feed `content: attr(…)`.
//!
//! `:nth-child(… of S)` needs nothing extra: Stylo's invalidation maps record
//! the selectors inside `of S`, and a change in a child's membership reaches
//! the siblings whose index it moves (`nth_of_and_of_type` pins it).

use rinch_core::dom::NodeId;
use selectors::matching::MatchingContext;
use selectors::matching::{ElementSelectorFlags, SelectorCaches};
use style::attr::{AttrIdentifier, AttrValue};
use style::context::{
    RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext,
    StyleSystemOptions,
};
use style::dom::TElement;
use style::invalidation::element::invalidation_map::Dependency;
use style::invalidation::element::invalidator::{
    DescendantInvalidationLists, InvalidationProcessor, InvalidationVector, SiblingTraversalMap,
    TreeStyleInvalidator,
};
use style::invalidation::element::restyle_hints::RestyleHint;
use style::invalidation::element::state_and_attributes::StateAndAttrInvalidationProcessor;
use style::selector_parser::ServoElementSnapshot;
use style::shared_lock::StylesheetGuards;
use style::traversal_flags::TraversalFlags;
use style::values::GenericAtomIdent;
use style::{Atom, LocalName, Namespace};

use crate::RinchDocument;
use crate::node::DirtyFlags;
use crate::stylo_impl::RinchNode;

/// The parent flags [`RinchDocument::note_child_list_changed_with`] acts on.
/// A parent carrying none of them (and not an `<ol>`) is untouched by a
/// change to its child list, so the note returns before copying the list.
const CHILD_LIST_FLAGS: ElementSelectorFlags = ElementSelectorFlags::HAS_SLOW_SELECTOR
    .union(ElementSelectorFlags::HAS_SLOW_SELECTOR_LATER_SIBLINGS)
    .union(ElementSelectorFlags::HAS_EDGE_CHILD_SELECTOR)
    .union(ElementSelectorFlags::HAS_EMPTY_SELECTOR);

/// Attributes whose value rinch reads to answer a pseudo-class (or pseudo
/// state an ancestor passes down), and that Stylo's attribute maps therefore
/// cannot see a dependency on. Each restyles its element, the element's
/// subtree and its later siblings.
const PSEUDO_CLASS_ATTRIBUTES: &[&str] = &["disabled", "href"];

/// Stylo's attribute-and-state invalidation processor, but walking into the
/// descendants of a `display: none` element as well.
///
/// Stylo skips them (`state_and_attributes::should_process_descendants`)
/// because in a browser an element under `display: none` has no style until
/// it is shown, and showing it styles it from scratch. rinch styles every
/// element, hidden or not — a hidden subtree keeps its computed styles, which
/// is what lets a change made while hidden land outright (#703) — so a
/// subtree the invalidator declined to enter would keep a style its selectors
/// no longer give it, and show it when the ancestor is shown again.
struct EveryDescendant<'a, 'b: 'a, E: TElement + 'b> {
    inner: StateAndAttrInvalidationProcessor<'a, 'b, E>,
    root: E,
}

impl<'a, 'b: 'a, E: TElement + 'b> InvalidationProcessor<'a, 'a, E> for EveryDescendant<'a, 'b, E> {
    fn invalidates_on_pseudo_element(&self) -> bool {
        self.inner.invalidates_on_pseudo_element()
    }

    fn light_tree_only(&self) -> bool {
        self.inner.light_tree_only()
    }

    fn check_outer_dependency(
        &mut self,
        dependency: &Dependency,
        element: E,
        scope: Option<selectors::OpaqueElement>,
    ) -> bool {
        self.inner
            .check_outer_dependency(dependency, element, scope)
    }

    fn matching_context(&mut self) -> &mut MatchingContext<'a, E::Impl> {
        self.inner.matching_context()
    }

    fn sibling_traversal_map(&self) -> &SiblingTraversalMap<E> {
        self.inner.sibling_traversal_map()
    }

    fn collect_invalidations(
        &mut self,
        element: E,
        self_invalidations: &mut InvalidationVector<'a>,
        descendant_invalidations: &mut DescendantInvalidationLists<'a>,
        sibling_invalidations: &mut InvalidationVector<'a>,
    ) -> bool {
        self.inner.collect_invalidations(
            element,
            self_invalidations,
            descendant_invalidations,
            sibling_invalidations,
        )
    }

    fn should_process_descendants(&mut self, element: E) -> bool {
        if element == self.root {
            // Its data is borrowed by `inner`; at worst this re-examines a
            // subtree already marked for a full restyle.
            return true;
        }
        element
            .borrow_data()
            .is_some_and(|d| !d.hint.contains(RestyleHint::RESTYLE_DESCENDANTS))
    }

    fn recursion_limit_exceeded(&mut self, element: E) {
        self.inner.recursion_limit_exceeded(element)
    }

    fn invalidated_self(&mut self, element: E) {
        self.inner.invalidated_self(element)
    }

    fn invalidated_sibling(&mut self, sibling: E, of: E) {
        self.inner.invalidated_sibling(sibling, of)
    }

    fn invalidated_descendants(&mut self, element: E, child: E) {
        self.inner.invalidated_descendants(element, child)
    }
}

/// No paint worklets in rinch.
struct NoPainters;

impl RegisteredSpeculativePainters for NoPainters {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

fn local_name(name: &str) -> LocalName {
    GenericAtomIdent(web_atoms::LocalName::from(name))
}

fn no_namespace() -> Namespace {
    GenericAtomIdent(web_atoms::Namespace::from(""))
}

impl RinchDocument {
    /// Record that `name` is about to change on `node`. Call it **before** the
    /// write: the snapshot holds the attributes as they were.
    pub(crate) fn note_attribute_change(&mut self, node: usize, name: &str) {
        if !self.tree.nodes.get(node).is_some_and(|n| n.is_element()) {
            return;
        }
        self.snapshot_attribute_change(node, name);

        // What Stylo's maps cannot know.
        let tag = self.tree.nodes[node].tag().map(str::to_owned);
        if PSEUDO_CLASS_ATTRIBUTES.contains(&name) {
            self.mark_restyle(node, true);
            self.mark_later_siblings(node, true);
        } else if name == "style" {
            self.mark_restyle(node, false);
        }
        match (tag.as_deref(), name) {
            (Some("textarea"), "rows") => self.mark_restyle(node, false),
            (Some("ol"), "start") => self.mark_element_children(node, false),
            (Some("li"), "value") => {
                self.mark_restyle(node, false);
                self.mark_later_siblings(node, false);
            }
            _ => {}
        }
        let has_generated = self.tree.nodes[node].content_reads_attrs.get()
            || self.tree.nodes[node]
                .children
                .iter()
                .any(|&c| self.tree.nodes[c].is_pseudo_element);
        if has_generated {
            self.mark_restyle(node, false);
        }
    }

    /// The inset fast path's half of [`Self::note_attribute_change`]: when
    /// some rule names the `style` attribute (`[style*=…]`), snapshot it so
    /// Stylo's invalidator sees the write; otherwise do nothing at all, so a
    /// drag's per-frame `set_style` still costs no style resolve.
    pub(crate) fn note_inset_style_write(&mut self, node: usize) {
        let ln = local_name("style");
        let selected = self.tree.nodes.get(node).is_some_and(|n| n.is_element())
            && self
                .stylist
                .any_applicable_rule_data(RinchNode::new(node, &self.tree), |data| {
                    data.might_have_attribute_dependency(&ln)
                });
        if selected {
            self.snapshot_attribute_change(node, "style");
        }
    }

    /// The snapshot half of [`Self::note_attribute_change`] alone: Stylo's
    /// invalidator learns that `name` changed, and nothing is forced.
    pub(crate) fn snapshot_attribute_change(&mut self, node: usize, name: &str) {
        if !self.tree.nodes.get(node).is_some_and(|n| n.is_element()) {
            return;
        }
        self.tree.styles_dirty = true;
        if self.ensure_snapshot(node) {
            let snapshot = self
                .snapshots
                .get_mut(&style::dom::OpaqueNode(node))
                .expect("ensure_snapshot made one");
            if snapshot.attrs.is_none() {
                snapshot.attrs = Some(snapshot_attrs(&self.tree.nodes[node]));
            }
            match name {
                "class" => snapshot.class_changed = true,
                "id" => snapshot.id_changed = true,
                _ => snapshot.other_attributes_changed = true,
            }
            let ln = local_name(name);
            if !snapshot.changed_attrs.contains(&ln) {
                snapshot.changed_attrs.push(ln);
            }
        }
    }

    /// Record that `node`'s interaction state (hover, focus, focus ring,
    /// active) is about to change. Call it **before** the change.
    pub(crate) fn note_state_change(&mut self, node: usize) {
        if !self.tree.nodes.get(node).is_some_and(|n| n.is_element()) {
            return;
        }
        self.tree.styles_dirty = true;
        self.ensure_snapshot(node);
    }

    /// Make sure `node` has a snapshot holding its current state, and answer
    /// whether it has one. An element that has never been styled gets none —
    /// it will be matched from scratch anyway — but anything a change of it
    /// could reach among its later siblings is restyled instead, since no
    /// invalidation will run for it.
    fn ensure_snapshot(&mut self, node: usize) -> bool {
        let opaque = style::dom::OpaqueNode(node);
        if self.snapshots.contains_key(&opaque) {
            if self.tree.nodes[node].has_snapshot {
                return true;
            }
            // An entry for a slab id since freed and handed to this node
            // (`set_inner_html`, `remove_subtree`): it describes the old node.
            self.snapshots.remove(&opaque);
        }
        let styled = self.tree.nodes[node]
            .stylo_element_data
            .borrow()
            .as_ref()
            .is_some_and(|d| d.styles.primary.is_some());
        if !styled {
            // Nothing to snapshot: the element is matched from scratch at the
            // next resolve anyway. Its later siblings need nothing either. An
            // unstyled element among styled siblings got there by an
            // insertion, which already marked every sibling a structural or
            // sibling selector can reach (`note_child_list_changed`, from the
            // flags those siblings' own matching left on the parent). One
            // whose style was dropped *after* its snapshot was taken is
            // handled in `process_snapshots`.
            return false;
        }
        let snapshot = ServoElementSnapshot {
            state: Some(crate::stylo_impl::element_state(&self.tree.nodes[node])),
            ..Default::default()
        };
        self.snapshots.insert(opaque, snapshot);
        self.tree.nodes[node].has_snapshot = true;
        self.snapshot_ids.push(node);
        true
    }

    /// Run Stylo's invalidator for every element snapshotted since the last
    /// resolve, and turn its answers into style roots. Called at the top of
    /// `resolve_styles`, after the stylist is flushed (the invalidation maps
    /// must describe the current sheets).
    pub(crate) fn process_snapshots(&mut self) {
        if self.snapshot_ids.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.snapshot_ids);
        let mut outcomes: Vec<(usize, bool, bool, bool)> = Vec::with_capacity(ids.len());
        // Snapshotted elements whose style was dropped before this resolve —
        // a resize restyling a viewport-unit user, a re-insertion. Stylo
        // cannot compare them; they re-cascade from scratch, and whatever a
        // sibling selector ties to them is restyled wholesale.
        let mut lost: Vec<usize> = Vec::new();
        {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            let painters = NoPainters;
            let shared = SharedStyleContext {
                stylist: &self.stylist,
                visited_styles_enabled: false,
                options: StyleSystemOptions::default(),
                guards,
                current_time_for_animations: 0.0,
                traversal_flags: TraversalFlags::empty(),
                snapshot_map: &self.snapshots,
                animations: Default::default(),
                registered_speculative_painters: &painters,
            };
            let mut caches = SelectorCaches::default();
            for &id in &ids {
                let Some(node) = self.tree.nodes.get(id) else {
                    continue;
                };
                if !node.has_snapshot {
                    continue;
                }
                let element = RinchNode::new(id, &self.tree);
                let mut data = node.stylo_element_data.borrow_mut();
                let Some(data) = data.as_mut().filter(|d| d.styles.primary.is_some()) else {
                    lost.push(id);
                    continue;
                };
                // `ElementData::invalidate_style_if_needed`, with one change:
                // descendants of a `display: none` element are processed too
                // (`EveryDescendant`).
                let mut processor = EveryDescendant {
                    inner: StateAndAttrInvalidationProcessor::new(
                        &shared,
                        element,
                        data,
                        &mut caches,
                    ),
                    root: element,
                };
                let result = TreeStyleInvalidator::new(element, None, &mut processor).invalidate();
                // SAFETY: single-threaded; the flag is plain bookkeeping.
                unsafe { style::dom::TElement::set_handled_snapshot(&element) };
                outcomes.push((
                    id,
                    result.has_invalidated_self(),
                    result.has_invalidated_descendants(),
                    result.has_invalidated_siblings(),
                ));
            }
        }
        self.snapshots.clear();
        for &id in &ids {
            if let Some(node) = self.tree.nodes.get_mut(id) {
                node.has_snapshot = false;
                node.snapshot_handled
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        for id in lost {
            self.mark_later_siblings(id, true);
        }
        for (id, invalidated_self, descendants, siblings) in outcomes {
            self.tree
                .perf
                .bump(crate::perf::Counter::StyleInvalidations);
            if invalidated_self || descendants {
                self.tree.style_roots.push(id);
            }
            let parent = self.tree.nodes[id].parent;
            if siblings && let Some(p) = parent {
                self.tree.nodes[p].style_dirty_descendants.set(true);
                self.tree.style_roots.push(p);
            }
        }
    }

    /// Restyle `node` at the next resolve, and with `subtree` every element
    /// under it too. Keeps its current style as the "before" the cascade
    /// compares against to decide whether its children must follow.
    pub(crate) fn mark_restyle(&mut self, node: usize, subtree: bool) {
        let Some(n) = self.tree.nodes.get(node) else {
            return;
        };
        if !n.is_element() || n.is_pseudo_element {
            return;
        }
        let wanted = if subtree {
            RestyleHint::RESTYLE_SELF | RestyleHint::RESTYLE_DESCENDANTS
        } else {
            RestyleHint::RESTYLE_SELF
        };
        if let Some(data) = n.stylo_element_data.borrow_mut().as_mut() {
            // Already marked this frame: its root and dirty flags are in
            // place. Without this, N insertions into a list under a structural
            // selector pushed O(N²) roots before the frame's one resolve.
            if data.hint.contains(wanted) {
                return;
            }
            data.hint.insert(wanted);
        }
        self.tree.style_roots.push(node);
        self.tree.styles_dirty = true;
        self.push_dirty_flags(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
    }

    /// [`Self::mark_restyle`] for every element sibling after `node`.
    fn mark_later_siblings(&mut self, node: usize, subtree: bool) {
        let Some(parent) = self.tree.nodes.get(node).and_then(|n| n.parent) else {
            return;
        };
        let siblings: Vec<usize> = self.tree.nodes[parent]
            .children
            .iter()
            .copied()
            .skip_while(|&c| c != node)
            .skip(1)
            .collect();
        for s in siblings {
            self.mark_restyle(s, subtree);
        }
    }

    /// [`Self::mark_restyle`] for every element child of `parent`.
    fn mark_element_children(&mut self, parent: usize, subtree: bool) {
        let children = self.tree.nodes[parent].children.clone();
        for c in children {
            self.mark_restyle(c, subtree);
        }
    }

    /// `parent`'s child list changed around position `at`: `at` is the index
    /// the inserted node now occupies, or the index the removed one had.
    ///
    /// Restyles what the selector flags matching left on `parent` say an
    /// insertion or removal can change, CSS Selectors 4's structural
    /// pseudo-classes and sibling combinators:
    ///
    /// - `HAS_SLOW_SELECTOR` (`:nth-last-child`, `:nth-last-of-type`,
    ///   `:only-of-type`, `:last-of-type`): every child;
    /// - `HAS_SLOW_SELECTOR_LATER_SIBLINGS` (`:nth-child`, `:nth-of-type`,
    ///   `:first-of-type`, `+`, `~`): every child after `at`;
    /// - `HAS_EDGE_CHILD_SELECTOR` (`:first-child`, `:last-child`,
    ///   `:only-child`): the element siblings on either side of `at`, which is
    ///   where "first" and "last" can move;
    /// - `HAS_EMPTY_SELECTOR` on `parent` itself: `parent`, with its later
    ///   siblings (`:empty + p`) — only when its `:empty` can have flipped,
    ///   i.e. at most one child now keeps it non-empty. Marking it on every
    ///   insertion forced a full resolve of the list per row (an ancestor of
    ///   the new row carried a hint): 10400 cascades for 100 appends;
    /// - an `<ol>`: every `<li>` after `at`, whose marker number moved.
    ///
    /// Each restyled child takes its subtree along (`:nth-child(2) .x`). A
    /// `parent` never matched against anything — being built, or detached —
    /// carries no flags and costs nothing.
    pub(crate) fn note_child_list_changed(&mut self, parent: usize, at: usize) {
        self.note_child_list_changed_with(parent, at, true);
    }

    /// [`Self::note_child_list_changed`], told by a caller that knows it
    /// (`replace_node` swapping one non-empty child for another) that
    /// `parent`'s `:empty` cannot have flipped.
    pub(crate) fn note_child_list_changed_with(
        &mut self,
        parent: usize,
        at: usize,
        empty_may_flip: bool,
    ) {
        let Some(p) = self.tree.nodes.get(parent) else {
            return;
        };
        if !p.is_element() {
            return;
        }
        let flags = *p.selector_flags.borrow();
        let is_ol = p.tag() == Some("ol");
        if !is_ol && !flags.intersects(CHILD_LIST_FLAGS) {
            return;
        }
        let children = p.children.clone();
        if flags.contains(ElementSelectorFlags::HAS_SLOW_SELECTOR) {
            for &c in &children {
                self.mark_restyle(c, true);
            }
        } else if flags.contains(ElementSelectorFlags::HAS_SLOW_SELECTOR_LATER_SIBLINGS) {
            for &c in children.iter().skip(at) {
                self.mark_restyle(c, true);
            }
        }
        if flags.contains(ElementSelectorFlags::HAS_EDGE_CHILD_SELECTOR) {
            let is_el = |tree: &crate::node::NodeTree, c: usize| {
                tree.nodes[c].is_element() && !tree.nodes[c].is_pseudo_element
            };
            let before = children[..at.min(children.len())]
                .iter()
                .rev()
                .copied()
                .find(|&c| is_el(&self.tree, c));
            let after = children
                .iter()
                .skip(at)
                .copied()
                .find(|&c| is_el(&self.tree, c) && !self.tree.nodes[c].is_pseudo_element);
            // `after` may be the inserted node itself; its successor is the
            // one whose edge-ness can have moved.
            let after_next = children
                .iter()
                .skip(at + 1)
                .copied()
                .find(|&c| is_el(&self.tree, c));
            for c in [before, after, after_next].into_iter().flatten() {
                self.mark_restyle(c, true);
            }
        }
        if flags.contains(ElementSelectorFlags::HAS_EMPTY_SELECTOR)
            && empty_may_flip
            && crate::stylo_impl::empty_can_have_flipped(&self.tree, parent)
        {
            self.note_empty_flipped(parent);
        }
        if is_ol {
            for &c in children.iter().skip(at) {
                if self.tree.nodes[c].tag() == Some("li") {
                    self.mark_restyle(c, false);
                }
            }
        }
    }

    /// Text child `child` of `parent` is about to change between empty and
    /// non-empty (the caller checks that): only `:empty` can see it, and only
    /// if no other child already keeps `parent` non-empty. A text edit that
    /// keeps a node non-empty — every keystroke in an editor paragraph under
    /// `p:empty` — costs nothing.
    pub(crate) fn note_text_emptiness_changed(&mut self, parent: usize, child: usize) {
        let Some(p) = self.tree.nodes.get(parent) else {
            return;
        };
        if p.selector_flags
            .borrow()
            .contains(ElementSelectorFlags::HAS_EMPTY_SELECTOR)
            && !crate::stylo_impl::other_children_defeat_empty(&self.tree, parent, child)
        {
            self.note_empty_flipped(parent);
        }
    }

    /// `parent`'s `:empty` flipped: restyle it, and its later siblings only
    /// if a sibling combinator was ever matched among its siblings
    /// (`:empty + p` flags `parent`'s own parent). The editor's
    /// `p:empty { min-height: 1lh }` names no sibling, so emptying one
    /// paragraph restyles that paragraph, not every one after it.
    fn note_empty_flipped(&mut self, parent: usize) {
        self.mark_restyle(parent, true);
        let siblings_selected = self.tree.nodes[parent].parent.is_some_and(|g| {
            self.tree.nodes[g]
                .selector_flags
                .borrow()
                .contains(ElementSelectorFlags::HAS_SLOW_SELECTOR_LATER_SIBLINGS)
        });
        if siblings_selected {
            self.mark_later_siblings(parent, true);
        }
    }

    /// Take `child` out of `parent`'s child list and answer the index it
    /// held (the list's length if it was not there), for
    /// [`Self::note_child_list_changed`]. One scan, where finding the index
    /// and then `retain`ing the rest was two.
    pub(crate) fn remove_from_children(&mut self, parent: usize, child: usize) -> usize {
        let children = &mut self.tree.nodes[parent].children;
        match children.iter().position(|&c| c == child) {
            Some(i) => {
                children.remove(i);
                i
            }
            None => children.len(),
        }
    }

    /// [`Self::note_child_list_changed_with`] for `child`, just inserted into
    /// `parent`, finding its index only if a selector can care: a parent no
    /// structural selector was ever matched under (and no `<ol>`) costs no
    /// scan of its children.
    pub(crate) fn note_child_inserted(
        &mut self,
        parent: usize,
        child: usize,
        empty_may_flip: bool,
    ) {
        let Some(p) = self.tree.nodes.get(parent) else {
            return;
        };
        if !p.is_element()
            || (p.tag() != Some("ol") && !p.selector_flags.borrow().intersects(CHILD_LIST_FLAGS))
        {
            return;
        }
        let at = self.child_index(parent, child);
        self.note_child_list_changed_with(parent, at, empty_may_flip);
    }

    /// The index `child` occupies in its parent's child list, for
    /// [`Self::note_child_list_changed`].
    pub(crate) fn child_index(&self, parent: usize, child: usize) -> usize {
        self.tree.nodes[parent]
            .children
            .iter()
            .position(|&c| c == child)
            .unwrap_or(self.tree.nodes[parent].children.len())
    }

    /// For tests and tools: whether `node` holds a pending invalidation
    /// snapshot.
    pub fn has_pending_style_snapshot(&self, node: NodeId) -> bool {
        self.tree.nodes.get(node.0).is_some_and(|n| n.has_snapshot)
    }
}

/// The element's attributes as a Stylo snapshot stores them: `class` as a
/// token list and `id` as an atom (what `ElementSnapshot::has_class` /
/// `id_attr` read), everything else as a string.
fn snapshot_attrs(node: &crate::node::Node) -> Vec<(AttrIdentifier, AttrValue)> {
    node.attributes
        .iter()
        .map(|(name, value)| {
            let ln = local_name(name);
            let ident = AttrIdentifier {
                local_name: ln.clone(),
                name: ln,
                namespace: no_namespace(),
                prefix: None,
            };
            let v = match name.as_str() {
                "class" => AttrValue::from_serialized_tokenlist(value.clone()),
                "id" => AttrValue::from_atomic(value.clone()),
                _ => AttrValue::String(value.clone()),
            };
            (ident, v)
        })
        .collect()
}
