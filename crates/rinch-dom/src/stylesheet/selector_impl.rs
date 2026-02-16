//! Selector implementation types for the `selectors` crate integration.

use std::fmt;

use cssparser::{CowRcStr, ParseError, ToCss};

use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::MatchingContext;
use selectors::matching::ElementSelectorFlags;
use selectors::parser::{
    self as sel_parser, NonTSPseudoClass, PseudoElement, SelectorImpl,
};
use selectors::{Element, OpaqueElement};

use std::collections::HashMap;

/// Our SelectorImpl for the selectors crate.
#[derive(Clone, Debug)]
pub struct RinchSelectorImpl;

/// A simple string wrapper for use as selector atoms.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RinchAtom(pub String);

impl From<&str> for RinchAtom {
    fn from(s: &str) -> Self {
        RinchAtom(s.to_string())
    }
}

impl AsRef<str> for RinchAtom {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for RinchAtom {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RinchAtom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl ToCss for RinchAtom {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl precomputed_hash::PrecomputedHash for RinchAtom {
    fn precomputed_hash(&self) -> u32 {
        let mut h: u32 = 5381;
        for b in self.0.bytes() {
            h = h.wrapping_mul(33).wrapping_add(b as u32);
        }
        h
    }
}

/// Pseudo-class enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RinchPseudoClass {
    Hover,
    Focus,
    FocusVisible,
    Active,
    Disabled,
    FirstChild,
    LastChild,
    Checked,
    Empty,
    Root,
}

impl ToCss for RinchPseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        match self {
            Self::Hover => dest.write_str(":hover"),
            Self::Focus => dest.write_str(":focus"),
            Self::FocusVisible => dest.write_str(":focus-visible"),
            Self::Active => dest.write_str(":active"),
            Self::Disabled => dest.write_str(":disabled"),
            Self::FirstChild => dest.write_str(":first-child"),
            Self::LastChild => dest.write_str(":last-child"),
            Self::Checked => dest.write_str(":checked"),
            Self::Empty => dest.write_str(":empty"),
            Self::Root => dest.write_str(":root"),
        }
    }
}

impl NonTSPseudoClass for RinchPseudoClass {
    type Impl = RinchSelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        matches!(self, Self::Hover | Self::Active)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(
            self,
            Self::Hover | Self::Active | Self::Focus | Self::FocusVisible
        )
    }
}

/// Pseudo-element enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RinchPseudoElement {
    Before,
    After,
    Placeholder,
}

impl ToCss for RinchPseudoElement {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        match self {
            Self::Before => dest.write_str("::before"),
            Self::After => dest.write_str("::after"),
            Self::Placeholder => dest.write_str("::placeholder"),
        }
    }
}

impl PseudoElement for RinchPseudoElement {
    type Impl = RinchSelectorImpl;
}

impl SelectorImpl for RinchSelectorImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = RinchAtom;
    type Identifier = RinchAtom;
    type LocalName = RinchAtom;
    type NamespaceUrl = RinchAtom;
    type NamespacePrefix = RinchAtom;
    type BorrowedLocalName = str;
    type BorrowedNamespaceUrl = str;
    type NonTSPseudoClass = RinchPseudoClass;
    type PseudoElement = RinchPseudoElement;
}

/// Selector parser.
pub(super) struct RinchSelectorParser;

impl<'i> sel_parser::Parser<'i> for RinchSelectorParser {
    type Impl = RinchSelectorImpl;
    type Error = sel_parser::SelectorParseErrorKind<'i>;

    fn parse_non_ts_pseudo_class(
        &self,
        _location: cssparser::SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<RinchPseudoClass, ParseError<'i, Self::Error>> {
        match &*name.to_ascii_lowercase() {
            "hover" => Ok(RinchPseudoClass::Hover),
            "focus" => Ok(RinchPseudoClass::Focus),
            "focus-visible" => Ok(RinchPseudoClass::FocusVisible),
            "active" => Ok(RinchPseudoClass::Active),
            "disabled" => Ok(RinchPseudoClass::Disabled),
            "first-child" => Ok(RinchPseudoClass::FirstChild),
            "last-child" => Ok(RinchPseudoClass::LastChild),
            "checked" => Ok(RinchPseudoClass::Checked),
            "empty" => Ok(RinchPseudoClass::Empty),
            "root" => Ok(RinchPseudoClass::Root),
            _ => Err(cssparser::ParseError {
                kind: cssparser::ParseErrorKind::Custom(
                    sel_parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ),
                location: _location,
            }),
        }
    }

    fn parse_pseudo_element(
        &self,
        location: cssparser::SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<RinchPseudoElement, ParseError<'i, Self::Error>> {
        match &*name.to_ascii_lowercase() {
            "before" => Ok(RinchPseudoElement::Before),
            "after" => Ok(RinchPseudoElement::After),
            "placeholder" => Ok(RinchPseudoElement::Placeholder),
            _ => Err(cssparser::ParseError {
                kind: cssparser::ParseErrorKind::Custom(
                    sel_parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ),
                location,
            }),
        }
    }

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_has(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// RinchElement adapter for selectors::tree::Element
// ---------------------------------------------------------------------------

/// Runtime state for an element, used for pseudo-class matching.
#[derive(Debug, Clone, Default)]
pub struct ElementState {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub is_hovered: bool,
    pub is_focused: bool,
    pub is_focus_visible: bool,
    pub is_disabled: bool,
    pub is_active: bool,
    pub is_first_child: bool,
    pub is_last_child: bool,
    pub is_checked: bool,
    pub is_empty: bool,
    pub is_root: bool,
    /// Attribute name -> value pairs (e.g. `data-active` -> `"true"`)
    pub attributes: HashMap<String, String>,
}

impl ElementState {
    /// Create an `ElementState` from a space-separated class string (for backward compat).
    pub fn from_classes(class_attr: &str) -> Self {
        Self {
            classes: class_attr.split_whitespace().map(String::from).collect(),
            ..Default::default()
        }
    }
}

/// An element adapter for matching selectors against our ElementState.
/// Uses an index into a shared chain: index 0 = target, 1 = parent, 2 = grandparent, etc.
#[derive(Clone, Debug)]
pub struct RinchElement<'a> {
    pub(super) index: usize,
    pub(super) chain: &'a [ElementState],
}

impl<'a> RinchElement<'a> {
    fn state(&self) -> &ElementState {
        &self.chain[self.index]
    }
}

impl<'a> Element for RinchElement<'a> {
    type Impl = RinchSelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        // Use a stable pointer derived from the chain element address
        OpaqueElement::new(&self.chain[self.index])
    }

    fn parent_element(&self) -> Option<Self> {
        if self.index + 1 < self.chain.len() {
            Some(RinchElement {
                index: self.index + 1,
                chain: self.chain,
            })
        } else {
            None
        }
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
        None
    }
    fn next_sibling_element(&self) -> Option<Self> {
        None
    }
    fn first_element_child(&self) -> Option<Self> {
        None
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.state().tag.as_deref() == Some(local_name)
    }

    fn has_namespace(&self, ns: &str) -> bool {
        ns.is_empty()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.state().tag == other.state().tag
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&RinchAtom>,
        local_name: &RinchAtom,
        operation: &AttrSelectorOperation<&RinchAtom>,
    ) -> bool {
        // Only support no-namespace attributes
        match ns {
            NamespaceConstraint::Specific(ns) if !ns.0.is_empty() => return false,
            _ => {}
        }
        let state = self.state();
        match operation {
            AttrSelectorOperation::Exists => state.attributes.contains_key(&local_name.0),
            AttrSelectorOperation::WithValue {
                operator,
                case_sensitivity,
                value,
            } => {
                if let Some(actual) = state.attributes.get(&local_name.0) {
                    use selectors::attr::AttrSelectorOperator;
                    let sensitive = matches!(case_sensitivity, CaseSensitivity::CaseSensitive);
                    let eq = |a: &str, b: &str| {
                        if sensitive {
                            a == b
                        } else {
                            a.eq_ignore_ascii_case(b)
                        }
                    };
                    match operator {
                        AttrSelectorOperator::Equal => eq(actual, &value.0),
                        AttrSelectorOperator::Includes => {
                            actual.split_whitespace().any(|w| eq(w, &value.0))
                        }
                        AttrSelectorOperator::DashMatch => {
                            eq(actual, &value.0)
                                || (actual.starts_with(&*value.0)
                                    && actual.as_bytes().get(value.0.len()) == Some(&b'-'))
                        }
                        AttrSelectorOperator::Prefix => {
                            if sensitive {
                                actual.starts_with(&*value.0)
                            } else {
                                actual
                                    .to_ascii_lowercase()
                                    .starts_with(&value.0.to_ascii_lowercase())
                            }
                        }
                        AttrSelectorOperator::Suffix => {
                            if sensitive {
                                actual.ends_with(&*value.0)
                            } else {
                                actual
                                    .to_ascii_lowercase()
                                    .ends_with(&value.0.to_ascii_lowercase())
                            }
                        }
                        AttrSelectorOperator::Substring => {
                            if sensitive {
                                actual.contains(&*value.0)
                            } else {
                                actual
                                    .to_ascii_lowercase()
                                    .contains(&value.0.to_ascii_lowercase())
                            }
                        }
                    }
                } else {
                    false
                }
            }
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &RinchPseudoClass,
        _context: &mut MatchingContext<RinchSelectorImpl>,
    ) -> bool {
        let s = self.state();
        match pc {
            RinchPseudoClass::Hover => s.is_hovered,
            RinchPseudoClass::Focus => s.is_focused,
            RinchPseudoClass::FocusVisible => s.is_focus_visible,
            RinchPseudoClass::Active => s.is_active,
            RinchPseudoClass::Disabled => s.is_disabled,
            RinchPseudoClass::FirstChild => s.is_first_child,
            RinchPseudoClass::LastChild => s.is_last_child,
            RinchPseudoClass::Checked => s.is_checked,
            RinchPseudoClass::Empty => s.is_empty,
            RinchPseudoClass::Root => s.is_root,
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &RinchPseudoElement,
        _context: &mut MatchingContext<RinchSelectorImpl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {
        // No-op for our use case
    }

    fn is_link(&self) -> bool {
        false
    }
    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &RinchAtom, case_sensitivity: CaseSensitivity) -> bool {
        if let Some(elem_id) = self.state().attributes.get("id") {
            match case_sensitivity {
                CaseSensitivity::CaseSensitive => elem_id == &id.0,
                CaseSensitivity::AsciiCaseInsensitive => elem_id.eq_ignore_ascii_case(&id.0),
            }
        } else {
            false
        }
    }

    fn has_class(&self, name: &RinchAtom, case_sensitivity: CaseSensitivity) -> bool {
        self.state().classes.iter().any(|c| match case_sensitivity {
            CaseSensitivity::CaseSensitive => c == &name.0,
            CaseSensitivity::AsciiCaseInsensitive => c.eq_ignore_ascii_case(&name.0),
        })
    }

    fn has_custom_state(&self, _name: &RinchAtom) -> bool {
        false
    }

    fn imported_part(&self, _name: &RinchAtom) -> Option<RinchAtom> {
        None
    }

    fn is_part(&self, _name: &RinchAtom) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.state().is_empty
    }

    fn is_root(&self) -> bool {
        self.state().is_root
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }

    fn ignores_nth_child_selectors(&self) -> bool {
        false
    }
}
