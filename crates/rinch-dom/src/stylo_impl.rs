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
use selectors::attr::{AttrSelectorOperation, AttrSelectorOperator, NamespaceConstraint};
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

    fn prev_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.backward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }
        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(node) = self.forward(n) {
            if node.is_element() {
                return Some(node);
            }
            n += 1;
        }
        None
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node()
            .children
            .iter()
            .find(|&&id| self.tree.nodes[id].is_element())
            .map(|&id| self.with(id))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
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

        match operation {
            AttrSelectorOperation::Exists => true,
            AttrSelectorOperation::WithValue {
                operator,
                case_sensitivity: _,
                value,
            } => {
                let value = value.as_ref();
                match operator {
                    AttrSelectorOperator::Equal => attr_value == value,
                    AttrSelectorOperator::Includes => attr_value
                        .split_ascii_whitespace()
                        .any(|word| word == value),
                    AttrSelectorOperator::DashMatch => {
                        attr_value.starts_with(value)
                            && (attr_value.len() == value.len()
                                || attr_value.chars().nth(value.len()) == Some('-'))
                    }
                    AttrSelectorOperator::Prefix => attr_value.starts_with(value),
                    AttrSelectorOperator::Substring => attr_value.contains(value),
                    AttrSelectorOperator::Suffix => attr_value.ends_with(value),
                }
            }
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pseudo_class {
            NonTSPseudoClass::Hover => self.node().is_hovered,
            NonTSPseudoClass::Active => false, // TODO: track active state
            NonTSPseudoClass::Focus => false,  // TODO: track focus state
            NonTSPseudoClass::Enabled => true,
            NonTSPseudoClass::Disabled => false,
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
            _ => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
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

    fn is_empty(&self) -> bool {
        self.node().children.is_empty()
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
        let mut state = ElementState::empty();
        if self.node().is_hovered {
            state |= ElementState::HOVER;
        }
        // TODO: Add other states (FOCUS, ACTIVE, etc.)
        state
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    fn id(&self) -> Option<&Atom> {
        // TODO: Store parsed Atom for ID
        None
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
        // Check if any child has STYLE dirty flag
        self.node().children.iter().any(|&id| {
            self.tree.nodes[id]
                .dirty
                .contains(crate::node::DirtyFlags::STYLE)
        })
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

    unsafe fn set_dirty_descendants(&self) {
        // Mark this node as needing style recalc
        // Note: We can't mutate through a shared reference, so this would need
        // interior mutability in production. For now, we track dirty state separately.
    }

    unsafe fn unset_dirty_descendants(&self) {
        // Clear dirty descendants flag
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

    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        None
    }

    fn match_element_lang(
        &self,
        _override_lang: Option<Option<style::selector_parser::AttrValue>>,
        _value: &style::selector_parser::Lang,
    ) -> bool {
        false
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
            _ => EMPTY.get_or_init(|| web_atoms::local_name!("")),
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
