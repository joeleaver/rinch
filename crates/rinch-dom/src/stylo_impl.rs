//! Stylo CSS engine integration for rinch-dom.
//!
//! Implements Stylo's TNode and TElement traits to enable CSS cascade,
//! specificity, and custom property inheritance.

use std::sync::atomic::Ordering;

use atomic_refcell::{AtomicRef, AtomicRefMut};
// Use selectors re-exported from style to ensure version compatibility
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::context::{QuirksMode, SharedStyleContext};
use style::dom::{
    AttributeProvider, LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode,
    TShadowRoot,
};
use style::properties::{ComputedValues, PropertyDeclarationBlock};
use style::selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl};
use style::servo_arc::{Arc, ArcBorrow};
use style::shared_lock::{Locked, SharedRwLock};
use style::stylesheets::scope_rule::ImplicitScopeRoot;
use style::values::{AtomIdent, AtomString, GenericAtomIdent};
use style::{Atom, LocalName, Namespace};
use stylo_dom::ElementState;

// Re-import selectors types from style to ensure version compatibility
use selectors::attr::{AttrSelectorOperation, NamespaceConstraint};
use selectors::matching::{ElementSelectorFlags, MatchingContext};
use selectors::sink::Push;
use selectors::{Element, OpaqueElement};

// The selectors Element trait expects raw Atom types, not GenericAtomIdent wrappers
// Use web_atoms directly for these borrowed types
type BorrowedLocalName = web_atoms::LocalName;
type BorrowedNamespaceUrl = web_atoms::Namespace;

use crate::node::{Node, NodeKind, NodeTree, RawNodeId};

/// A handle to a node that implements Stylo's traits.
///
/// This is a thin wrapper around a node reference and tree reference
/// that provides navigation and style data access.
#[derive(Clone, Copy)]
pub struct RinchNode<'a> {
    /// The node ID.
    pub id: RawNodeId,
    /// Reference to the node tree.
    pub tree: &'a NodeTree,
}

impl<'a> RinchNode<'a> {
    /// Create a new RinchNode wrapper.
    pub fn new(id: RawNodeId, tree: &'a NodeTree) -> Self {
        Self { id, tree }
    }

    /// Get the underlying Node reference.
    pub fn node(&self) -> &'a Node {
        &self.tree.nodes[self.id]
    }

    /// Create a RinchNode for a different node in the same tree.
    pub fn with(&self, id: RawNodeId) -> Self {
        Self {
            id,
            tree: self.tree,
        }
    }

    /// Navigate forward by n siblings.
    fn forward(&self, n: usize) -> Option<Self> {
        let parent_id = self.node().parent?;
        let parent = &self.tree.nodes[parent_id];
        let pos = parent.children.iter().position(|&c| c == self.id)?;
        let sibling_id = parent.children.get(pos + n)?;
        Some(self.with(*sibling_id))
    }

    /// Navigate backward by n siblings.
    fn backward(&self, n: usize) -> Option<Self> {
        let parent_id = self.node().parent?;
        let parent = &self.tree.nodes[parent_id];
        let pos = parent.children.iter().position(|&c| c == self.id)?;
        if pos < n {
            return None;
        }
        let sibling_id = parent.children.get(pos - n)?;
        Some(self.with(*sibling_id))
    }
}

impl std::fmt::Debug for RinchNode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RinchNode").field("id", &self.id).finish()
    }
}

impl PartialEq for RinchNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.tree, other.tree)
    }
}

impl std::hash::Hash for RinchNode<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_usize(self.id);
    }
}

impl Eq for RinchNode<'_> {}

// === NodeInfo trait ===

impl NodeInfo for RinchNode<'_> {
    fn is_element(&self) -> bool {
        matches!(self.node().kind, NodeKind::Element(_))
    }

    fn is_text_node(&self) -> bool {
        matches!(self.node().kind, NodeKind::Text(_))
    }
}

// === AttributeProvider trait ===

impl AttributeProvider for RinchNode<'_> {
    fn get_attr(&self, attr: &LocalName) -> Option<String> {
        self.node().attributes.get(attr.as_ref()).cloned()
    }
}

// === TDocument trait ===

impl<'a> TDocument for RinchNode<'a> {
    type ConcreteNode = RinchNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn is_html_document(&self) -> bool {
        true
    }

    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        &self.node().guard
    }
}

// === TShadowRoot trait ===

impl<'a> TShadowRoot for RinchNode<'a> {
    type ConcreteNode = RinchNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        // Shadow DOM not implemented
        unimplemented!("Shadow DOM not supported")
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        // Shadow DOM not implemented
        None
    }
}

// === TNode trait ===

impl<'a> TNode for RinchNode<'a> {
    type ConcreteElement = RinchNode<'a>;
    type ConcreteDocument = RinchNode<'a>;
    type ConcreteShadowRoot = RinchNode<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.node().parent.map(|id| self.with(id))
    }

    fn first_child(&self) -> Option<Self> {
        self.node().children.first().map(|&id| self.with(id))
    }

    fn last_child(&self) -> Option<Self> {
        self.node().children.last().map(|&id| self.with(id))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.backward(1)
    }

    fn next_sibling(&self) -> Option<Self> {
        self.forward(1)
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        // Document is always node 0
        self.with(self.tree.root_id)
    }

    fn is_in_document(&self) -> bool {
        true
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().and_then(|node| node.as_element())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.id)
    }

    fn debug_id(self) -> usize {
        self.id
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        match self.node().kind {
            NodeKind::Element(_) => Some(*self),
            _ => None,
        }
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        match self.node().kind {
            NodeKind::Document => Some(*self),
            _ => None,
        }
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        // Shadow DOM not implemented
        None
    }
}

// === selectors::Element trait ===

impl<'a> Element for RinchNode<'a> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        use std::ptr::NonNull;
        // Use node ID + 1 to avoid null pointer
        let non_null = NonNull::new((self.id + 1) as *mut ()).unwrap();
        OpaqueElement::from_non_null_ptr(non_null)
    }

    fn parent_element(&self) -> Option<Self> {
        TElement::traversal_parent(self)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    /// The previous sibling that is an element — and not a generated
    /// `::before` / `::after` / list-marker box. rinch keeps those in the
    /// child list, but they are not children for selector matching:
    /// `:nth-child`, `:first-child` and `+` count element children only
    /// (Selectors 4 §14). Counting them made an element's index depend on
    /// whether its parent's generated content existed yet — and change,
    /// unrestyled, when it appeared.
    fn prev_sibling_element(&self) -> Option<Self> {
        let parent = &self.tree.nodes[self.node().parent?];
        let pos = parent.children.iter().position(|&c| c == self.id)?;
        parent.children[..pos]
            .iter()
            .rev()
            .find(|&&c| is_matchable_element(&self.tree.nodes[c]))
            .map(|&c| self.with(c))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let parent = &self.tree.nodes[self.node().parent?];
        let pos = parent.children.iter().position(|&c| c == self.id)?;
        parent.children[pos + 1..]
            .iter()
            .find(|&&c| is_matchable_element(&self.tree.nodes[c]))
            .map(|&c| self.with(c))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node()
            .children
            .iter()
            .find(|&&id| is_matchable_element(&self.tree.nodes[id]))
            .map(|&id| self.with(id))
    }

    /// Used by the selectors crate to decide whether a type selector matches
    /// its local name ASCII case-insensitively (HTML) or exactly (#683).
    ///
    /// rinch carries no element namespace — every node hands Stylo the XHTML
    /// namespace (see [`Self::namespace`]) — so this answers from the tag
    /// name instead, the same move `attr_name::is_svg_content_tag` makes for
    /// the sibling question of attribute-name folding (#688): an element
    /// whose tag is one of SVG's own (`svg`, `linearGradient`, `clipPath`,
    /// …) is not an HTML element in an HTML document, whatever its ancestry,
    /// so a type selector against it is compared with the author's exact
    /// spelling. The four tags that collide with HTML (`a`, `script`,
    /// `style`, `title`) stay HTML — same four, same reasoning, see that
    /// function's doc.
    fn is_html_element_in_html_document(&self) -> bool {
        !self
            .node()
            .tag()
            .is_some_and(crate::attr_name::is_svg_content_tag)
    }

    fn has_local_name(&self, local_name: &BorrowedLocalName) -> bool {
        self.node()
            .tag()
            .map(|tag| tag == local_name.as_ref())
            .unwrap_or(false)
    }

    fn has_namespace(&self, _ns: &BorrowedNamespaceUrl) -> bool {
        // We only support HTML namespace
        true
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.node().tag() == other.node().tag()
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&Namespace>,
        local_name: &LocalName,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        let Some(attr_value) = self.node().attributes.get(local_name.as_ref()) else {
            return false;
        };
        // The selectors crate's own evaluator: the same one Stylo's
        // invalidation snapshots use (`ServoElementSnapshot::attr_matches` →
        // `AttrValue::eval_selector`), so an element and its snapshot can
        // never disagree about what an attribute selector means — and it
        // honours the `[attr=v i]` / `[attr=v s]` case flags, which a
        // hand-rolled comparison used to drop.
        operation.eval_str(attr_value)
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pseudo_class {
            NonTSPseudoClass::Hover => {
                self.node().hover_sensitive.set(true);
                self.node().is_hovered
            }
            NonTSPseudoClass::Active => {
                self.node().active_sensitive.set(true);
                self.node().is_active
            }
            NonTSPseudoClass::Focus => {
                self.node().focus_sensitive.set(true);
                self.node().is_focused
            }
            NonTSPseudoClass::FocusVisible => {
                // Shares `focus_sensitive`: the flag only ever changes on the
                // (gaining/losing) focused node, so focus-change invalidation
                // covers :focus-visible rules too.
                self.node().focus_sensitive.set(true);
                self.node().is_focus_visible
            }
            // `:enabled` / `:disabled` (issue #429). These answered a
            // hardcoded `true`/`false`, so `button:disabled { ... }` matched
            // nothing at all however the element was marked, while `:enabled`
            // matched everything — including a disabled control and a `<div>`.
            // Components papered over it with a `--disabled` modifier class
            // written beside the attribute, which is why it went unnoticed.
            //
            // The state is read from the attribute rather than from
            // `ElementState::DISABLED`, because Stylo does not gate these on
            // the bitflag: neither is in `RARE_PSEUDO_CLASS_STATES`, so the
            // selector is always reached and this predicate is the whole
            // answer. Invalidation needs no `*_sensitive` flag either (unlike
            // hover/focus, which change without an attribute write) —
            // `set_attribute` already re-resolves the node and its subtree,
            // which is exactly what a `<fieldset disabled>` toggle needs.
            //
            // Narrower than the focus machinery's rule on purpose: HTML
            // defines both pseudo-classes over form controls only, so a
            // `<div data-disabled>` — which rinch *does* remove from the Tab
            // order — must not style as disabled, or desktop would paint what
            // a real browser (and therefore `rinch-web`) leaves alone.
            NonTSPseudoClass::Enabled => {
                crate::node::tag_is_disableable(self.node().tag())
                    && !crate::node::node_is_disabled_in_tree(self.tree, self.id)
            }
            NonTSPseudoClass::Disabled => {
                crate::node::tag_is_disableable(self.node().tag())
                    && crate::node::node_is_disabled_in_tree(self.tree, self.id)
            }
            NonTSPseudoClass::Checked => {
                // Check for checked attribute on input/checkbox
                self.node()
                    .attributes
                    .get("checked")
                    .map(|_| true)
                    .unwrap_or(false)
            }
            NonTSPseudoClass::AnyLink | NonTSPseudoClass::Link => {
                // Is this an <a> or <area> with href?
                self.node()
                    .tag()
                    .map(|tag| {
                        (tag == "a" || tag == "area") && self.node().attributes.contains_key("href")
                    })
                    .unwrap_or(false)
            }
            NonTSPseudoClass::Visited => false, // We don't track visited links

            // #681. `:required`/`:optional`, `:read-only`/`:read-write` and
            // `:placeholder-shown` are plain attribute-derived predicates,
            // same shape as `:enabled`/`:disabled` above (#429) — narrower
            // than the full CSS definition on purpose; see
            // `node::tag_supports_required` / `node::tag_is_readonly_capable`
            // for the exact scope this leaves out. They are ALSO fed into
            // `element_state` below (as `ElementState::REQUIRED` etc.), so
            // Stylo's own invalidation maps see the dependency and restyle
            // precisely — self, descendants or later siblings — whenever a
            // `required`/`readonly`/`placeholder`/`value` write flips one,
            // exactly the mechanism `:checked` already rides.
            NonTSPseudoClass::Required => crate::node::node_is_required(self.node()),
            NonTSPseudoClass::Optional => crate::node::node_is_optional(self.node()),
            NonTSPseudoClass::ReadOnly => crate::node::node_is_read_only(self.node()),
            NonTSPseudoClass::ReadWrite => crate::node::node_is_read_write(self.node()),
            NonTSPseudoClass::PlaceholderShown => {
                crate::node::node_is_placeholder_shown(self.node())
            }

            // `:defined` (#681): rinch has no custom-element registry, so
            // every element rinch can build is defined — there is no
            // "unresolved" state to answer `false` for. Constant, so no
            // invalidation is needed (nothing can ever flip it).
            NonTSPseudoClass::Defined => true,

            // `:focus-within` is NOT implemented (#681, left open). A first
            // attempt propagated an eager `is_focus_within` bit to the
            // focused node's ancestors on every `update_focus`, the same
            // chain-walk `update_active` does for `:active` — but
            // `style_invalidation_twin_tests.rs`'s random-mutation oracle
            // (`.a:focus-within { --r32: 1; }`, already in its battery)
            // caught it diverging from a fresh rebuild whenever a `Move`
            // relocated a subtree while focus lived elsewhere: nothing
            // re-derives the eagerly-set bit when the TREE changes, only
            // when FOCUS changes, so a moved ancestor can carry a stale
            // answer. Reverted rather than shipped broken; see the issue for
            // the fix sketch (either re-derive on every structural verb while
            // some node is focused, or answer dynamically from
            // `self.tree.focused_node` and ensure invalidation reaches
            // whatever moved).

            // `:lang()` (#681). `lang_attr`/`match_element_lang` below exist
            // for Stylo's invalidation wrapper; ordinary matching (this
            // function) never goes through them and resolves inline instead.
            NonTSPseudoClass::Lang(ref lang) => resolve_lang(self.tree, self.id)
                .is_some_and(|resolved| lang_tag_matches(&resolved, lang)),

            _ => false,
        }
    }

    fn match_pseudo_element(
        &self,
        pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        matches!(pe, PseudoElement::Before | PseudoElement::After)
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        let self_flags = flags.for_self();
        if !self_flags.is_empty() {
            *self.node().selector_flags.borrow_mut() |= self_flags;
        }

        let parent_flags = flags.for_parent();
        if !parent_flags.is_empty()
            && let Some(parent) = self.parent_node()
        {
            *parent.node().selector_flags.borrow_mut() |= parent_flags;
        }
    }

    fn is_link(&self) -> bool {
        self.node().tag().map(|tag| tag == "a").unwrap_or(false)
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &AtomIdent, case_sensitivity: selectors::attr::CaseSensitivity) -> bool {
        self.node()
            .attributes
            .get("id")
            .map(|id_attr| {
                let id_atom = Atom::from(id_attr.as_str());
                style::CaseSensitivityExt::eq_atom(case_sensitivity, &id_atom, &id.0)
            })
            .unwrap_or(false)
    }

    fn has_class(
        &self,
        name: &AtomIdent,
        case_sensitivity: selectors::attr::CaseSensitivity,
    ) -> bool {
        let Some(class_attr) = self.node().attributes.get("class") else {
            return false;
        };

        for class in class_attr.split_ascii_whitespace() {
            let class_atom = Atom::from(class);
            if style::CaseSensitivityExt::eq_atom(case_sensitivity, &class_atom, &name.0) {
                return true;
            }
        }

        false
    }

    fn imported_part(&self, _name: &AtomIdent) -> Option<AtomIdent> {
        None
    }

    fn is_part(&self, _name: &AtomIdent) -> bool {
        false
    }

    /// `:empty` (Selectors 4 §14.2): no element children and no text of
    /// non-zero length — comments and generated `::before` / `::after` boxes
    /// do not count.
    fn is_empty(&self) -> bool {
        node_is_empty(self.tree, self.id)
    }

    fn is_root(&self) -> bool {
        // Root element is the html element (child of document)
        self.node()
            .parent
            .map(|p| matches!(self.tree.nodes[p].kind, NodeKind::Document))
            .unwrap_or(false)
    }

    fn has_custom_state(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn add_element_unique_hashes(&self, filter: &mut selectors::bloom::BloomFilter) -> bool {
        use selectors::bloom::BLOOM_HASH_MASK;
        use style::bloom::each_relevant_element_hash;
        each_relevant_element_hash(*self, |hash| filter.insert_hash(hash & BLOOM_HASH_MASK));
        true
    }
}

// === TElement trait ===

/// Iterator over element children for Stylo traversal.
pub struct ChildrenIterator<'a> {
    parent: RinchNode<'a>,
    index: usize,
}

impl<'a> Iterator for ChildrenIterator<'a> {
    type Item = RinchNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let child_id = self.parent.node().children.get(self.index)?;
        self.index += 1;
        Some(self.parent.with(*child_id))
    }
}

impl<'a> TElement for RinchNode<'a> {
    type ConcreteNode = RinchNode<'a>;
    type TraversalChildrenIterator = ChildrenIterator<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn implicit_scope_for_sheet_in_shadow_root(
        _opaque_host: OpaqueElement,
        _sheet_index: usize,
    ) -> Option<ImplicitScopeRoot> {
        // Shadow DOM not implemented
        None
    }

    fn traversal_children(&self) -> LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(ChildrenIterator {
            parent: *self,
            index: 0,
        })
    }

    fn is_html_element(&self) -> bool {
        self.is_element()
    }

    fn is_mathml_element(&self) -> bool {
        false
    }

    fn is_svg_element(&self) -> bool {
        self.node().tag().map(|t| t == "svg").unwrap_or(false)
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.node()
            .style_attribute_cache
            .as_ref()
            .map(|arc| arc.borrow_arc())
    }

    fn state(&self) -> ElementState {
        element_state(self.node())
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    /// The interned `id`, or `None` — Stylo's key into the id bucket (#675).
    ///
    /// This is **not** the matcher. `SelectorMap::get_all_matching_rules`
    /// consults `self.id_hash` behind `if let Some(id) = rule_hash_target.id()`
    /// with no `else` branch, so a `None` here means an id-**bucketed** rule is
    /// never even offered to this element and [`Element::has_id`] — the real
    /// predicate, correct all along — never runs for one. It does still run for
    /// an id on the ancestor side (`#a > p` is bucketed by `p`, offered anyway,
    /// and matched correctly even before #675), which is the asymmetry that hid
    /// the bug. `bloom.rs`'s `each_relevant_element_hash` and stylo's id
    /// invalidation read this too, and both are dormant in rinch.
    ///
    /// The return type is a reference, which is why `Node` stores the atom
    /// rather than interning per call the way [`Self::each_class`] does.
    fn id(&self) -> Option<&Atom> {
        self.node().id_atom()
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        if let Some(class_attr) = self.node().attributes.get("class") {
            for class in class_attr.split_ascii_whitespace() {
                let atom = Atom::from(class);
                callback(AtomIdent::cast(&atom));
            }
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&LocalName),
    {
        for attr_name in self.node().attributes.keys() {
            // Create a LocalName using the web_atoms LocalName type which uses LocalNameStaticSet
            let local_atom: web_atoms::LocalName = web_atoms::LocalName::from(attr_name.as_str());
            let local_name: LocalName = GenericAtomIdent(local_atom);
            callback(&local_name);
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        self.node().style_dirty_descendants.get()
    }

    fn has_snapshot(&self) -> bool {
        self.node().has_snapshot
    }

    fn handled_snapshot(&self) -> bool {
        self.node().snapshot_handled.load(Ordering::SeqCst)
    }

    unsafe fn set_handled_snapshot(&self) {
        self.node().snapshot_handled.store(true, Ordering::SeqCst);
    }

    /// Stylo's invalidator marks the path from an invalidated element down to
    /// every invalidated descendant; `resolve_styles` follows it.
    unsafe fn set_dirty_descendants(&self) {
        self.node().style_dirty_descendants.set(true);
    }

    unsafe fn unset_dirty_descendants(&self) {
        self.node().style_dirty_descendants.set(false);
    }

    fn store_children_to_process(&self, _n: isize) {
        // Not needed for our single-threaded traversal
    }

    fn did_process_child(&self) -> isize {
        // Not needed for our single-threaded traversal
        0
    }

    unsafe fn ensure_data(&self) -> AtomicRefMut<'_, style::data::ElementData> {
        let mut stylo_data = self.node().stylo_element_data.borrow_mut();
        if stylo_data.is_none() {
            *stylo_data = Some(style::data::ElementData::default());
        }
        AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap())
    }

    unsafe fn clear_data(&self) {
        *self.node().stylo_element_data.borrow_mut() = None;
    }

    fn has_data(&self) -> bool {
        self.node().stylo_element_data.borrow().is_some()
    }

    fn borrow_data(&self) -> Option<AtomicRef<'_, style::data::ElementData>> {
        let stylo_data = self.node().stylo_element_data.borrow();
        if stylo_data.is_some() {
            Some(AtomicRef::map(stylo_data, |sd| sd.as_ref().unwrap()))
        } else {
            None
        }
    }

    fn mutate_data(&self) -> Option<AtomicRefMut<'_, style::data::ElementData>> {
        let stylo_data = self.node().stylo_element_data.borrow_mut();
        if stylo_data.is_some() {
            Some(AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap()))
        } else {
            None
        }
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false // TODO: implement animations
    }

    fn has_animations(&self, _context: &SharedStyleContext) -> bool {
        false
    }

    fn has_css_animations(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<PseudoElement>,
    ) -> bool {
        false
    }

    fn has_css_transitions(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<PseudoElement>,
    ) -> bool {
        false
    }

    fn animation_rule(
        &self,
        _context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn transition_rule(
        &self,
        _context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    /// The resolved `lang` (#681): this element's own `lang` attribute, or
    /// the nearest ancestor's — see [`resolve_lang`]. Used by Stylo's
    /// invalidation wrapper (`element_wrapper.rs`'s `get_lang`), not by
    /// ordinary `:lang()` matching, which resolves inline in
    /// [`Self::match_non_ts_pseudo_class`].
    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        resolve_lang(self.tree, self.id).map(|lang| AtomString::from(lang.as_str()))
    }

    fn match_element_lang(
        &self,
        override_lang: Option<Option<style::selector_parser::AttrValue>>,
        value: &style::selector_parser::Lang,
    ) -> bool {
        let resolved = match override_lang {
            // `Some(None)`: the snapshot says this element and its chain
            // carried no `lang` at the snapshotted moment — nothing to match.
            Some(explicit) => explicit.map(|v| v.as_ref().to_owned()),
            None => resolve_lang(self.tree, self.id),
        };
        resolved.is_some_and(|lang| lang_tag_matches(&lang, value))
    }

    fn is_html_document_body_element(&self) -> bool {
        self.node().tag() == Some("body")
            && self
                .node()
                .parent
                .map(|p| self.tree.nodes[p].tag() == Some("html"))
                .unwrap_or(false)
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: selectors::matching::VisitedHandlingMode,
        _hints: &mut V,
    ) where
        V: Push<ApplicableDeclarationBlock>,
    {
        // TODO: Handle legacy attributes like align, bgcolor, etc.
    }

    fn local_name(&self) -> &BorrowedLocalName {
        // We need to return a reference to a static atom that lives long enough.
        // Cache common HTML tag names to avoid allocation.
        // For unknown tags, fall back to empty (will match via has_local_name).
        use std::sync::OnceLock;

        static EMPTY: OnceLock<BorrowedLocalName> = OnceLock::new();
        static DIV: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SPAN: OnceLock<BorrowedLocalName> = OnceLock::new();
        static P: OnceLock<BorrowedLocalName> = OnceLock::new();
        static A: OnceLock<BorrowedLocalName> = OnceLock::new();
        static BODY: OnceLock<BorrowedLocalName> = OnceLock::new();
        static HTML: OnceLock<BorrowedLocalName> = OnceLock::new();
        static HEAD: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H1: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H2: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H3: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H4: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H5: OnceLock<BorrowedLocalName> = OnceLock::new();
        static H6: OnceLock<BorrowedLocalName> = OnceLock::new();
        static UL: OnceLock<BorrowedLocalName> = OnceLock::new();
        static OL: OnceLock<BorrowedLocalName> = OnceLock::new();
        static LI: OnceLock<BorrowedLocalName> = OnceLock::new();
        static BUTTON: OnceLock<BorrowedLocalName> = OnceLock::new();
        static INPUT: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TEXTAREA: OnceLock<BorrowedLocalName> = OnceLock::new();
        static IMG: OnceLock<BorrowedLocalName> = OnceLock::new();
        static FORM: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TABLE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SECTION: OnceLock<BorrowedLocalName> = OnceLock::new();
        static ARTICLE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static HEADER: OnceLock<BorrowedLocalName> = OnceLock::new();
        static FOOTER: OnceLock<BorrowedLocalName> = OnceLock::new();
        static MAIN: OnceLock<BorrowedLocalName> = OnceLock::new();
        static NAV: OnceLock<BorrowedLocalName> = OnceLock::new();
        static ASIDE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static PRE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static CODE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static EM: OnceLock<BorrowedLocalName> = OnceLock::new();
        static STRONG: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SELECT: OnceLock<BorrowedLocalName> = OnceLock::new();
        static LABEL: OnceLock<BorrowedLocalName> = OnceLock::new();
        static STYLE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SCRIPT: OnceLock<BorrowedLocalName> = OnceLock::new();
        static LINK: OnceLock<BorrowedLocalName> = OnceLock::new();
        static META: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TITLE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SVG: OnceLock<BorrowedLocalName> = OnceLock::new();
        static PATH: OnceLock<BorrowedLocalName> = OnceLock::new();
        static BLOCKQUOTE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static HR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static BR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static MARK: OnceLock<BorrowedLocalName> = OnceLock::new();
        static B: OnceLock<BorrowedLocalName> = OnceLock::new();
        static I: OnceLock<BorrowedLocalName> = OnceLock::new();
        static U: OnceLock<BorrowedLocalName> = OnceLock::new();
        static S: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SUB: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SUP: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SMALL: OnceLock<BorrowedLocalName> = OnceLock::new();
        static ABBR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static CITE: OnceLock<BorrowedLocalName> = OnceLock::new();
        static KBD: OnceLock<BorrowedLocalName> = OnceLock::new();
        static SAMP: OnceLock<BorrowedLocalName> = OnceLock::new();
        static VAR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static Q: OnceLock<BorrowedLocalName> = OnceLock::new();
        static DFN: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TIME: OnceLock<BorrowedLocalName> = OnceLock::new();
        static WBR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static DEL: OnceLock<BorrowedLocalName> = OnceLock::new();
        static INS: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TR: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TD: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TH: OnceLock<BorrowedLocalName> = OnceLock::new();
        static THEAD: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TBODY: OnceLock<BorrowedLocalName> = OnceLock::new();
        static TFOOT: OnceLock<BorrowedLocalName> = OnceLock::new();
        static CAPTION: OnceLock<BorrowedLocalName> = OnceLock::new();

        match self.node().tag() {
            Some("div") => DIV.get_or_init(|| web_atoms::local_name!("div")),
            Some("span") => SPAN.get_or_init(|| web_atoms::local_name!("span")),
            Some("p") => P.get_or_init(|| web_atoms::local_name!("p")),
            Some("a") => A.get_or_init(|| web_atoms::local_name!("a")),
            Some("body") => BODY.get_or_init(|| web_atoms::local_name!("body")),
            Some("html") => HTML.get_or_init(|| web_atoms::local_name!("html")),
            Some("head") => HEAD.get_or_init(|| web_atoms::local_name!("head")),
            Some("h1") => H1.get_or_init(|| web_atoms::local_name!("h1")),
            Some("h2") => H2.get_or_init(|| web_atoms::local_name!("h2")),
            Some("h3") => H3.get_or_init(|| web_atoms::local_name!("h3")),
            Some("h4") => H4.get_or_init(|| web_atoms::local_name!("h4")),
            Some("h5") => H5.get_or_init(|| web_atoms::local_name!("h5")),
            Some("h6") => H6.get_or_init(|| web_atoms::local_name!("h6")),
            Some("ul") => UL.get_or_init(|| web_atoms::local_name!("ul")),
            Some("ol") => OL.get_or_init(|| web_atoms::local_name!("ol")),
            Some("li") => LI.get_or_init(|| web_atoms::local_name!("li")),
            Some("button") => BUTTON.get_or_init(|| web_atoms::local_name!("button")),
            Some("input") => INPUT.get_or_init(|| web_atoms::local_name!("input")),
            Some("textarea") => TEXTAREA.get_or_init(|| web_atoms::local_name!("textarea")),
            Some("img") => IMG.get_or_init(|| web_atoms::local_name!("img")),
            Some("form") => FORM.get_or_init(|| web_atoms::local_name!("form")),
            Some("table") => TABLE.get_or_init(|| web_atoms::local_name!("table")),
            Some("tr") => TR.get_or_init(|| web_atoms::local_name!("tr")),
            Some("td") => TD.get_or_init(|| web_atoms::local_name!("td")),
            Some("th") => TH.get_or_init(|| web_atoms::local_name!("th")),
            Some("thead") => THEAD.get_or_init(|| web_atoms::local_name!("thead")),
            Some("tbody") => TBODY.get_or_init(|| web_atoms::local_name!("tbody")),
            Some("tfoot") => TFOOT.get_or_init(|| web_atoms::local_name!("tfoot")),
            Some("caption") => CAPTION.get_or_init(|| web_atoms::local_name!("caption")),
            Some("section") => SECTION.get_or_init(|| web_atoms::local_name!("section")),
            Some("article") => ARTICLE.get_or_init(|| web_atoms::local_name!("article")),
            Some("header") => HEADER.get_or_init(|| web_atoms::local_name!("header")),
            Some("footer") => FOOTER.get_or_init(|| web_atoms::local_name!("footer")),
            Some("main") => MAIN.get_or_init(|| web_atoms::local_name!("main")),
            Some("nav") => NAV.get_or_init(|| web_atoms::local_name!("nav")),
            Some("aside") => ASIDE.get_or_init(|| web_atoms::local_name!("aside")),
            Some("pre") => PRE.get_or_init(|| web_atoms::local_name!("pre")),
            Some("code") => CODE.get_or_init(|| web_atoms::local_name!("code")),
            Some("em") => EM.get_or_init(|| web_atoms::local_name!("em")),
            Some("strong") => STRONG.get_or_init(|| web_atoms::local_name!("strong")),
            Some("select") => SELECT.get_or_init(|| web_atoms::local_name!("select")),
            Some("label") => LABEL.get_or_init(|| web_atoms::local_name!("label")),
            Some("style") => STYLE.get_or_init(|| web_atoms::local_name!("style")),
            Some("script") => SCRIPT.get_or_init(|| web_atoms::local_name!("script")),
            Some("link") => LINK.get_or_init(|| web_atoms::local_name!("link")),
            Some("meta") => META.get_or_init(|| web_atoms::local_name!("meta")),
            Some("title") => TITLE.get_or_init(|| web_atoms::local_name!("title")),
            Some("svg") => SVG.get_or_init(|| web_atoms::local_name!("svg")),
            Some("path") => PATH.get_or_init(|| web_atoms::local_name!("path")),
            Some("blockquote") => BLOCKQUOTE.get_or_init(|| web_atoms::local_name!("blockquote")),
            Some("hr") => HR.get_or_init(|| web_atoms::local_name!("hr")),
            Some("br") => BR.get_or_init(|| web_atoms::local_name!("br")),
            Some("mark") => MARK.get_or_init(|| web_atoms::local_name!("mark")),
            Some("b") => B.get_or_init(|| web_atoms::local_name!("b")),
            Some("i") => I.get_or_init(|| web_atoms::local_name!("i")),
            Some("u") => U.get_or_init(|| web_atoms::local_name!("u")),
            Some("s") => S.get_or_init(|| web_atoms::local_name!("s")),
            Some("sub") => SUB.get_or_init(|| web_atoms::local_name!("sub")),
            Some("sup") => SUP.get_or_init(|| web_atoms::local_name!("sup")),
            Some("small") => SMALL.get_or_init(|| web_atoms::local_name!("small")),
            Some("abbr") => ABBR.get_or_init(|| web_atoms::local_name!("abbr")),
            Some("cite") => CITE.get_or_init(|| web_atoms::local_name!("cite")),
            Some("kbd") => KBD.get_or_init(|| web_atoms::local_name!("kbd")),
            Some("samp") => SAMP.get_or_init(|| web_atoms::local_name!("samp")),
            Some("var") => VAR.get_or_init(|| web_atoms::local_name!("var")),
            Some("q") => Q.get_or_init(|| web_atoms::local_name!("q")),
            Some("dfn") => DFN.get_or_init(|| web_atoms::local_name!("dfn")),
            Some("time") => TIME.get_or_init(|| web_atoms::local_name!("time")),
            Some("wbr") => WBR.get_or_init(|| web_atoms::local_name!("wbr")),
            Some("del") => DEL.get_or_init(|| web_atoms::local_name!("del")),
            Some("ins") => INS.get_or_init(|| web_atoms::local_name!("ins")),
            // Any other tag: intern it dynamically so type selectors still match it.
            // The arms above are a lock-free fast path for common tags; without this
            // fallback, an unlisted tag (a custom element, a less common HTML tag)
            // would return the EMPTY atom and silently fail every tag selector.
            Some(other) => intern_local_name(other),
            None => EMPTY.get_or_init(|| web_atoms::local_name!("")),
        }
    }

    fn namespace(&self) -> &BorrowedNamespaceUrl {
        // HTML namespace
        static HTML_NS: std::sync::OnceLock<BorrowedNamespaceUrl> = std::sync::OnceLock::new();
        HTML_NS.get_or_init(|| web_atoms::namespace_url!("http://www.w3.org/1999/xhtml"))
    }

    fn query_container_size(
        &self,
        _display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        // Container queries not implemented
        Default::default()
    }

    fn each_custom_state<F>(&self, _callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        // Custom states not implemented
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.node().selector_flags.borrow().contains(flags)
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        let flags = self.node().selector_flags.borrow();
        if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR)
        {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR
        } else if flags.contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING) {
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING
        } else {
            ElementSelectorFlags::empty()
        }
    }

    fn compute_layout_damage(
        _old: &ComputedValues,
        _new: &ComputedValues,
    ) -> style::selector_parser::RestyleDamage {
        // Return full damage for now - can optimize later
        style::selector_parser::RestyleDamage::all()
    }
}

/// The element state Stylo sees for `node`: the interaction states rinch
/// tracks, plus `CHECKED` from the `checked` attribute — the same fact
/// `:checked` is matched from, so Stylo's state-dependency map can say which
/// selectors a checkbox toggle reaches. Shared by [`TElement::state`] and the
/// invalidation snapshots, which must agree.
pub(crate) fn element_state(node: &Node) -> ElementState {
    let mut state = ElementState::empty();
    if node.is_hovered {
        state |= ElementState::HOVER;
    }
    if node.is_focused {
        state |= ElementState::FOCUS;
    }
    if node.is_focus_visible {
        state |= ElementState::FOCUSRING;
    }
    if node.is_active {
        state |= ElementState::ACTIVE;
    }
    if node.attributes.contains_key("checked") {
        state |= ElementState::CHECKED;
    }
    // #681: fed from the same facts `match_non_ts_pseudo_class` reads, so
    // Stylo's invalidation maps see a state dependency on each and restyle
    // precisely (self/descendants/siblings) when `note_attribute_change`
    // snapshots an element whose `required`/`readonly`/`placeholder`/`value`
    // is about to change — the generic mechanism `:checked` above already
    // rides, not the manual "restyle self+subtree+later-siblings" forcing
    // `:disabled`/`:enabled`/`href` use (those need the forcing because a
    // `<fieldset disabled>` changes a *descendant's* answer without that
    // descendant's own attributes moving, which no per-element state bit can
    // express; none of these four has that problem).
    if crate::node::tag_supports_required(node.tag()) {
        if node.attributes.contains_key("required") {
            state |= ElementState::REQUIRED;
        } else {
            state |= ElementState::OPTIONAL_;
        }
    }
    if crate::node::tag_is_readonly_capable(node.tag()) {
        if node.attributes.contains_key("readonly") {
            state |= ElementState::READONLY;
        } else {
            state |= ElementState::READWRITE;
        }
        if crate::node::node_is_placeholder_shown(node) {
            state |= ElementState::PLACEHOLDER_SHOWN;
        }
    }
    // `:defined` (#681): constant — rinch has no custom-element registry, so
    // nothing is ever "undefined". Included for `state()` callers that read
    // it as a fact rather than matching through `match_non_ts_pseudo_class`.
    state |= ElementState::DEFINED;
    state
}

/// The effective `lang` for `:lang()` (#681): `node`'s own `lang` attribute,
/// or the nearest ancestor's. An explicit `lang=""` means "no language" per
/// HTML and stops the walk there (`None`), rather than falling through to an
/// ancestor that happens to have one.
pub(crate) fn resolve_lang(tree: &NodeTree, id: RawNodeId) -> Option<String> {
    let mut cur = Some(id);
    while let Some(nid) = cur {
        let node = tree.get(nid)?;
        if let Some(lang) = node.attributes.get("lang") {
            return if lang.is_empty() {
                None
            } else {
                Some(lang.clone())
            };
        }
        cur = node.parent;
    }
    None
}

/// Selectors 4 §16.5's basic filtering (RFC 4647 §3.3.1): `want` matches
/// `resolved` exactly (ASCII case-insensitive), or as a prefix followed by
/// `-` — so `en` matches `en-US` but not `english`.
pub(crate) fn lang_tag_matches(resolved: &str, want: &str) -> bool {
    if resolved.eq_ignore_ascii_case(want) {
        return true;
    }
    resolved.len() > want.len()
        && resolved.as_bytes()[want.len()] == b'-'
        && resolved[..want.len()].eq_ignore_ascii_case(want)
}

/// An element child as selectors count children: an element, and not a
/// generated pseudo-element box.
fn is_matchable_element(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Element(_)) && !node.is_pseudo_element
}

/// `:empty`: no child element other than a generated pseudo-element box, and
/// no text node with any text. Comments do not count.
pub(crate) fn node_is_empty(tree: &NodeTree, id: RawNodeId) -> bool {
    tree.nodes[id]
        .children
        .iter()
        .all(|&c| !child_defeats_empty(&tree.nodes[c]))
}

/// Whether `node`, as a child, makes its parent not `:empty`.
pub(crate) fn child_defeats_empty(child: &Node) -> bool {
    match &child.kind {
        NodeKind::Element(_) => !child.is_pseudo_element,
        NodeKind::Text(t) => !t.content.is_empty(),
        _ => false,
    }
}

/// Whether some child of `id` other than `except` keeps `id` from being
/// `:empty`.
pub(crate) fn other_children_defeat_empty(
    tree: &NodeTree,
    id: RawNodeId,
    except: RawNodeId,
) -> bool {
    tree.nodes[id]
        .children
        .iter()
        .any(|&c| c != except && child_defeats_empty(&tree.nodes[c]))
}

/// Whether one child change (an insertion, a removal, a text edit) can have
/// flipped `id`'s `:empty`: only if, after it, at most one child keeps `id`
/// from being empty. Anything more and `id` was non-empty before the change
/// and is after it. What keeps a list build under an `:empty` rule from
/// restyling the list (and its subtree) on every row.
pub(crate) fn empty_can_have_flipped(tree: &NodeTree, id: RawNodeId) -> bool {
    tree.nodes[id]
        .children
        .iter()
        .filter(|&&c| child_defeats_empty(&tree.nodes[c]))
        .nth(1)
        .is_none()
}

/// Intern an arbitrary tag name into a `'static` [`BorrowedLocalName`] so stylo's
/// type-selector matching recognizes it. The hardcoded fast-path arms in
/// [`local_name`](RinchElement::local_name) cover the common tags lock-free; this
/// handles the long tail (table cells `td`/`tr`/`th`, custom elements, …) by
/// interning on first use and caching the leaked atom. The set of distinct tag
/// names a document uses is small and finite, so the bounded leak is intentional.
fn intern_local_name(tag: &str) -> &'static BorrowedLocalName {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<String, &'static BorrowedLocalName>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&name) = map.get(tag) {
        return name;
    }
    let leaked: &'static BorrowedLocalName = Box::leak(Box::new(BorrowedLocalName::from(tag)));
    map.insert(tag.to_string(), leaked);
    leaked
}
