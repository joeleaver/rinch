//! CSS stylesheet for class-based styling.
//!
//! Parses a subset of CSS (class selectors, `:root` variables) and resolves
//! class attributes to property maps. Supports `var(--name)` resolution and
//! `rem` to `px` conversion.
//!
//! Uses Mozilla's `cssparser` for tokenization and rule parsing, and the
//! `selectors` crate for proper selector parsing and matching.

use std::collections::HashMap;
use std::fmt;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss,
};

use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::{ElementSelectorFlags, matches_selector_list};
use selectors::parser::{
    self as sel_parser, NonTSPseudoClass, ParseRelative, PseudoElement, SelectorImpl, SelectorList,
};
use selectors::{Element, OpaqueElement};

// ---------------------------------------------------------------------------
// selectors crate integration types
// ---------------------------------------------------------------------------

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
struct RinchSelectorParser;

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

/// An element adapter for matching selectors against our ElementState.
/// Uses an index into a shared chain: index 0 = target, 1 = parent, 2 = grandparent, etc.
#[derive(Clone, Debug)]
pub struct RinchElement<'a> {
    index: usize,
    chain: &'a [ElementState],
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

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed CSS rule: selectors + property map.
#[derive(Debug, Clone)]
pub struct CssRule {
    /// The raw selector text (kept for :root detection and debugging).
    pub selector_text: String,
    /// Parsed selector list from the selectors crate.
    pub selectors: Option<SelectorList<RinchSelectorImpl>>,
    /// Property name -> (value, is_important).
    pub properties: HashMap<String, (String, bool)>,
}

/// A stylesheet that stores CSS rules and variables.
#[derive(Debug, Clone)]
pub struct Stylesheet {
    /// CSS rules in source order.
    pub rules: Vec<CssRule>,
    /// CSS custom properties from `:root` blocks.
    pub variables: HashMap<String, String>,
}

impl Default for Stylesheet {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylesheet {
    /// Create an empty stylesheet.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            variables: HashMap::new(),
        }
    }

    /// Parse a CSS string and return a new Stylesheet.
    pub fn parse(css: &str) -> Self {
        let mut stylesheet = Self::new();
        stylesheet.add_css(css);
        stylesheet
    }

    /// Parse CSS and merge into this stylesheet.
    pub fn add_css(&mut self, css: &str) {
        let css = css.trim();
        if css.is_empty() {
            return;
        }

        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = RinchRuleParser;

        for result in StyleSheetParser::new(&mut parser, &mut rule_parser) {
            let rule = match result {
                Ok(r) => r,
                Err(_) => continue,
            };

            if rule.properties.is_empty() {
                continue;
            }

            // Handle :root selector — extract variables
            if rule.selector_text.trim() == ":root" {
                for (k, (v, _)) in &rule.properties {
                    if k.starts_with("--") {
                        self.variables.insert(k.clone(), v.clone());
                    }
                }
                continue;
            }

            // Try to parse selector with the selectors crate
            let selector_text = rule.selector_text.clone();
            let parsed = {
                let mut sel_input = ParserInput::new(&selector_text);
                let mut sel_parser = Parser::new(&mut sel_input);
                SelectorList::parse(&RinchSelectorParser, &mut sel_parser, ParseRelative::No).ok()
            };

            if parsed.is_none() {
                tracing::warn!("Failed to parse selector: {:?}", selector_text);
                // Still store the rule without parsed selectors for debugging
                continue;
            }

            self.rules.push(CssRule {
                selector_text,
                selectors: parsed,
                properties: rule.properties,
            });
        }
    }

    /// Recursively resolve `var(--name)` references in a value.
    /// Returns the resolved value. Handles cycles via max depth.
    pub fn resolve_var(&self, value: &str) -> String {
        self.resolve_var_depth(value, 0)
    }

    fn resolve_var_depth(&self, value: &str, depth: usize) -> String {
        if depth > 10 || !value.contains("var(") {
            return value.to_string();
        }

        let mut result = String::with_capacity(value.len());
        let mut remaining = value;

        while let Some(start) = remaining.find("var(") {
            result.push_str(&remaining[..start]);
            let after_var = &remaining[start + 4..];

            // Find matching closing paren, handling nested parens
            let mut depth_paren = 1;
            let mut end = 0;
            for (i, ch) in after_var.char_indices() {
                match ch {
                    '(' => depth_paren += 1,
                    ')' => {
                        depth_paren -= 1;
                        if depth_paren == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if depth_paren != 0 {
                // Malformed — just append the rest
                result.push_str(&remaining[start..]);
                return result;
            }

            let inner = &after_var[..end].trim();
            // inner might be `--name` or `--name, fallback`
            let (var_name, fallback) = if let Some(comma_pos) = inner.find(',') {
                (
                    inner[..comma_pos].trim(),
                    Some(inner[comma_pos + 1..].trim()),
                )
            } else {
                (*inner, None)
            };

            if let Some(resolved) = self.variables.get(var_name) {
                let resolved = self.resolve_var_depth(resolved, depth + 1);
                result.push_str(&resolved);
            } else if let Some(fb) = fallback {
                let resolved = self.resolve_var_depth(fb, depth + 1);
                result.push_str(&resolved);
            } else {
                // Unresolved, keep original
                result.push_str(&remaining[start..start + 4 + end + 1]);
            }

            remaining = &after_var[end + 1..];
        }

        result.push_str(remaining);
        result
    }

    /// Convert `rem` units to `px` (1rem = 16px) in potentially compound values.
    pub fn resolve_unit(value: &str) -> String {
        let value = value.trim();
        // Quick check — if no 'rem' anywhere, return as-is
        if !value.contains("rem") {
            return value.to_string();
        }
        // Split on whitespace, convert each part
        let parts: Vec<String> = value
            .split_whitespace()
            .map(|part| {
                if let Some(rem_str) = part.strip_suffix("rem")
                    && let Ok(num) = rem_str.parse::<f64>()
                {
                    return format!("{}px", num * 16.0);
                }
                part.to_string()
            })
            .collect();
        parts.join(" ")
    }

    /// Re-parse CSS and update only `:root` variables (no rule duplication).
    /// Use this when theme CSS changes at runtime to update CSS custom properties
    /// without re-adding all the non-`:root` rules.
    pub fn update_variables_from_css(&mut self, css: &str) {
        let css = css.trim();
        if css.is_empty() {
            return;
        }

        self.variables.clear();

        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = RinchRuleParser;

        for result in StyleSheetParser::new(&mut parser, &mut rule_parser) {
            let rule = match result {
                Ok(r) => r,
                Err(_) => continue,
            };
            if rule.selector_text.trim() == ":root" {
                for (k, (v, _)) in &rule.properties {
                    if k.starts_with("--") {
                        self.variables.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    /// Resolve all units in a value: var() first, then rem.
    /// Viewport units (vh/vw) are left as-is for the layout engine to resolve.
    pub fn resolve_value(&self, value: &str) -> String {
        let resolved = self.resolve_var(value);
        Self::resolve_unit(&resolved)
    }

    /// Match a space-separated class string against all rules.
    /// Returns merged properties (later rules override earlier ones).
    /// Compound selectors `.a.b` only match if all classes are present.
    /// Respects !important: an !important value won't be overridden by a non-!important value.
    pub fn match_classes(&self, class_attr: &str) -> HashMap<String, (String, bool)> {
        self.match_classes_with_tag(class_attr, None)
    }

    /// Match with both classes and an optional tag name.
    pub fn match_classes_with_tag(
        &self,
        class_attr: &str,
        tag: Option<&str>,
    ) -> HashMap<String, (String, bool)> {
        let classes: Vec<&str> = class_attr.split_whitespace().collect();
        let state = ElementState {
            classes: classes.iter().map(|s| s.to_string()).collect(),
            tag: tag.map(|t| t.to_string()),
            ..Default::default()
        };
        self.match_element(&state, &[])
    }

    /// Match rules against an element with full state (pseudo-classes, ancestors, attributes).
    pub fn match_element(
        &self,
        element: &ElementState,
        ancestors: &[ElementState],
    ) -> HashMap<String, (String, bool)> {
        // Note: we don't early-return on empty classes/tag because universal selectors (*)
        // and attribute selectors can still match.

        // Build the chain: [element, parent, grandparent, ...]
        let mut chain = Vec::with_capacity(1 + ancestors.len());
        chain.push(element.clone());
        chain.extend_from_slice(ancestors);

        let elem = RinchElement {
            index: 0,
            chain: &chain,
        };

        // Track (value, important, specificity, source_order) per property
        // to implement correct CSS cascade: !important > specificity > source order
        let mut result: HashMap<String, (String, bool, u32, usize)> = HashMap::new();
        let mut caches = SelectorCaches::default();

        for (rule_index, rule) in self.rules.iter().enumerate() {
            if let Some(ref selector_list) = rule.selectors {
                let mut context = MatchingContext::new(
                    MatchingMode::Normal,
                    None,
                    &mut caches,
                    QuirksMode::NoQuirks,
                    NeedsSelectorFlags::No,
                    MatchingForInvalidation::No,
                );

                if matches_selector_list(selector_list, &elem, &mut context) {
                    // Compute max specificity of matching selectors in this rule
                    let specificity = selector_list
                        .slice()
                        .iter()
                        .map(|s| s.specificity())
                        .max()
                        .unwrap_or(0);

                    for (k, (v, imp)) in &rule.properties {
                        let should_insert =
                            if let Some((_, existing_imp, existing_spec, existing_order)) =
                                result.get(k)
                            {
                                if *imp && !existing_imp {
                                    // New is !important, existing is not -> new wins
                                    true
                                } else if !imp && *existing_imp {
                                    // Existing is !important, new is not -> existing wins
                                    false
                                } else {
                                    // Same importance level: higher specificity wins,
                                    // equal specificity: later source order wins
                                    specificity > *existing_spec
                                        || (specificity == *existing_spec
                                            && rule_index >= *existing_order)
                                }
                            } else {
                                true
                            };

                        if should_insert {
                            result.insert(k.clone(), (v.clone(), *imp, specificity, rule_index));
                        }
                    }
                }
            }
        }

        // Strip specificity/order tracking from result
        result
            .into_iter()
            .map(|(k, (v, imp, _, _))| (k, (v, imp)))
            .collect()
    }

    /// Resolve class-based styles for a node, with var() and rem resolution.
    pub fn resolve_class_styles(&self, class_attr: &str) -> HashMap<String, String> {
        let props = self.match_classes(class_attr);
        let mut result = HashMap::new();
        for (k, (v, _)) in props {
            let resolved = self.resolve_value(&v);
            result.insert(k, resolved);
        }
        result
    }
}

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

// ---------------------------------------------------------------------------
// cssparser trait implementations
// ---------------------------------------------------------------------------

/// A single parsed declaration (property: value with optional !important).
struct ParsedDeclaration {
    name: String,
    value: String,
    important: bool,
}

/// Parser for declarations inside a rule block.
struct RinchDeclarationParser;

impl<'i> DeclarationParser<'i> for RinchDeclarationParser {
    type Declaration = ParsedDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let start_pos = input.position();
        while input.next_including_whitespace().is_ok() {}
        let raw = input.slice_from(start_pos).trim();

        let (value, important) = if let Some(pos) = raw.rfind("!important") {
            let before = raw[..pos].trim_end();
            if before.is_empty() {
                return Err(input.new_custom_error(()));
            }
            (before.to_string(), true)
        } else {
            (raw.to_string(), false)
        };

        if value.is_empty() {
            return Err(input.new_custom_error(()));
        }

        Ok(ParsedDeclaration {
            name: name.to_string(),
            value,
            important,
        })
    }
}

impl<'i> AtRuleParser<'i> for RinchDeclarationParser {
    type Prelude = ();
    type AtRule = ParsedDeclaration;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for RinchDeclarationParser {
    type Prelude = ();
    type QualifiedRule = ParsedDeclaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, ParsedDeclaration, ()> for RinchDeclarationParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

/// Intermediate rule from top-level parsing (before selector parsing).
struct RawCssRule {
    selector_text: String,
    properties: HashMap<String, (String, bool)>,
}

/// Top-level rule parser for stylesheets.
struct RinchRuleParser;

impl<'i> AtRuleParser<'i> for RinchRuleParser {
    type Prelude = ();
    type AtRule = RawCssRule;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for RinchRuleParser {
    type Prelude = String;
    type QualifiedRule = RawCssRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        // Collect the entire prelude as a string (selector text)
        let start = input.position();
        while input.next_including_whitespace().is_ok() {}
        let selector = input.slice_from(start).trim().to_string();
        if selector.is_empty() {
            return Err(input.new_custom_error(()));
        }
        Ok(selector)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        // Parse declarations inside the block
        let mut properties = HashMap::new();
        let mut decl_parser = RinchDeclarationParser;
        for decl in RuleBodyParser::new(input, &mut decl_parser).flatten() {
            properties.insert(decl.name, (decl.value, decl.important));
        }

        Ok(RawCssRule {
            selector_text: prelude,
            properties,
        })
    }
}

/// Parse CSS property declarations from a string (used for inline styles).
fn parse_properties(body: &str) -> HashMap<String, (String, bool)> {
    let mut input = ParserInput::new(body);
    let mut parser = Parser::new(&mut input);
    let mut decl_parser = RinchDeclarationParser;
    let mut result = HashMap::new();
    for decl in RuleBodyParser::new(&mut parser, &mut decl_parser).flatten() {
        result.insert(decl.name, (decl.value, decl.important));
    }
    result
}

/// Resolve var() references checking local custom properties first, then global stylesheet variables.
fn resolve_var_with_locals(
    value: &str,
    stylesheet: &Stylesheet,
    local_vars: &HashMap<String, String>,
) -> String {
    if !value.contains("var(") {
        return value.to_string();
    }

    let mut result = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(start) = remaining.find("var(") {
        result.push_str(&remaining[..start]);
        let after_var = &remaining[start + 4..];

        // Find matching closing paren (handling nested parens)
        let mut depth = 1;
        let mut end = 0;
        for (i, ch) in after_var.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            // Unbalanced parens, keep as-is
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }

        let inner = &after_var[..end];
        remaining = &after_var[end + 1..];

        // Split on first comma for fallback
        let (var_name, fallback) = if let Some(comma_pos) = inner.find(',') {
            (
                inner[..comma_pos].trim(),
                Some(inner[comma_pos + 1..].trim()),
            )
        } else {
            (inner.trim(), None)
        };

        // Check local vars first, then global
        if let Some(resolved) = local_vars.get(var_name) {
            result.push_str(resolved);
        } else if let Some(resolved) = stylesheet.variables.get(var_name) {
            // Recursively resolve the global var value
            let resolved = resolve_var_with_locals(resolved, stylesheet, local_vars);
            result.push_str(&resolved);
        } else if let Some(fb) = fallback {
            let resolved = resolve_var_with_locals(fb, stylesheet, local_vars);
            result.push_str(&resolved);
        } else {
            // Unresolved, keep original
            result.push_str("var(");
            result.push_str(inner);
            result.push(')');
        }
    }

    result.push_str(remaining);
    result
}

/// Compute merged style properties: class-based styles + inline overrides.
/// Resolves var() and rem in the final result.
/// Respects !important: inline styles won't override class-based !important values unless the inline style is also !important.
pub fn compute_merged_styles(
    stylesheet: &Stylesheet,
    class_attr: Option<&str>,
    inline_style: Option<&str>,
    tag: Option<&str>,
) -> HashMap<String, String> {
    compute_merged_styles_with_state(stylesheet, class_attr, inline_style, None, &[], tag, None)
}

/// Compute merged styles with full element state for pseudo-class and descendant matching.
/// The `inherited_custom_props` parameter allows CSS custom properties (--xxx) to be
/// inherited from parent elements, enabling proper CSS variable inheritance.
pub fn compute_merged_styles_with_state(
    stylesheet: &Stylesheet,
    class_attr: Option<&str>,
    inline_style: Option<&str>,
    element_state: Option<&ElementState>,
    ancestors: &[ElementState],
    tag: Option<&str>,
    inherited_custom_props: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    // Get class-based styles with importance tracking
    let class_props = match (element_state, class_attr) {
        (Some(state), _) => stylesheet.match_element(state, ancestors),
        (None, Some(cls)) => stylesheet.match_classes_with_tag(cls, tag),
        (None, None) if tag.is_some() => stylesheet.match_classes_with_tag("", tag),
        _ => HashMap::new(),
    };

    // Parse inline styles
    let inline_props = if let Some(style_str) = inline_style {
        parse_properties(style_str)
    } else {
        HashMap::new()
    };

    // Merge: inline wins UNLESS class value is !important and inline is not
    let mut merged: HashMap<String, String> = HashMap::new();

    // Start with class-based
    for (k, (v, _)) in &class_props {
        merged.insert(k.clone(), v.clone());
    }

    // Build importance map from class props
    let important_keys: std::collections::HashSet<String> = class_props
        .iter()
        .filter(|(_, (_, imp))| *imp)
        .map(|(k, _)| k.clone())
        .collect();

    // Overlay inline styles, but don't override !important class values
    for (k, (v, imp)) in &inline_props {
        if important_keys.contains(k) && !imp {
            // Class has !important, inline doesn't — class wins
            continue;
        }
        merged.insert(k.clone(), v.clone());
    }

    // Resolve var() references, considering inherited, global :root, and local custom properties
    // Step 1: Start with inherited custom properties from parent (CSS custom property inheritance)
    let mut local_vars: HashMap<String, String> = HashMap::new();
    if let Some(inherited) = inherited_custom_props {
        for (key, value) in inherited {
            if key.starts_with("--") {
                local_vars.insert(key.clone(), value.clone());
            }
        }
    }

    // Step 2: Override/add local custom properties (--xxx) from this element
    for (key, value) in &merged {
        if key.starts_with("--") {
            // Resolve the value using inherited vars + global vars
            local_vars.insert(
                key.clone(),
                resolve_var_with_locals(&stylesheet.resolve_value(value), stylesheet, &local_vars),
            );
        }
    }

    // Step 3: Re-resolve local vars in case they reference each other
    for _ in 0..5 {
        let mut changed = false;
        let vars_snapshot = local_vars.clone();
        for (key, value) in local_vars.iter_mut() {
            if key.starts_with("--") && value.contains("var(") {
                let resolved = resolve_var_with_locals(value, stylesheet, &vars_snapshot);
                if &resolved != value {
                    *value = resolved;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Step 4: Add resolved custom properties back to merged (for inheritance to children)
    for (key, value) in &local_vars {
        merged.insert(key.clone(), value.clone());
    }

    // Step 5: Resolve all properties using a combined lookup (local vars first, then global)
    for value in merged.values_mut() {
        // First resolve using local custom properties
        let mut resolved = value.clone();
        // Keep resolving until no more var() references change
        for _ in 0..10 {
            let prev = resolved.clone();
            resolved = resolve_var_with_locals(&resolved, stylesheet, &local_vars);
            if resolved == prev {
                break;
            }
        }
        // Then resolve rem units
        resolved = Stylesheet::resolve_unit(&resolved);
        *value = resolved;
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_rules() {
        let css = ".foo { color: red; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
        assert_eq!(
            props.get("font-size").map(|(v, _)| v.as_str()),
            Some("14px")
        );
    }

    #[test]
    fn test_compound_selectors() {
        let css = ".a.b { color: blue; }";
        let ss = Stylesheet::parse(css);
        // Both classes present
        let props = ss.match_classes("a b");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
        // Only one class — no match
        let props = ss.match_classes("a");
        assert!(!props.contains_key("color"));
        let props = ss.match_classes("b");
        assert!(!props.contains_key("color"));
    }

    #[test]
    fn test_var_resolution() {
        let css = r#"
            :root { --rinch-primary-color: #339af0; --rinch-alias: var(--rinch-primary-color); }
            .btn { color: var(--rinch-primary-color); background: var(--rinch-alias); }
        "#;
        let ss = Stylesheet::parse(css);
        let props = ss.resolve_class_styles("btn");
        assert_eq!(props.get("color").unwrap(), "#339af0");
        assert_eq!(props.get("background").unwrap(), "#339af0");
    }

    #[test]
    fn test_rem_conversion() {
        assert_eq!(Stylesheet::resolve_unit("1.5rem"), "24px");
        assert_eq!(Stylesheet::resolve_unit("1rem"), "16px");
        assert_eq!(Stylesheet::resolve_unit("0.25rem"), "4px");
        assert_eq!(Stylesheet::resolve_unit("10px"), "10px");
    }

    #[test]
    fn test_inline_overrides_class() {
        let css = ".foo { color: red; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(&ss, Some("foo"), Some("color: blue"), None);
        assert_eq!(merged.get("color").unwrap(), "blue");
        assert_eq!(merged.get("font-size").unwrap(), "14px");
    }

    #[test]
    fn test_pseudo_selectors_stored() {
        let css = ".foo:hover { color: red; } .bar { color: blue; }";
        let ss = Stylesheet::parse(css);
        // Both rules stored
        assert_eq!(ss.rules.len(), 2);
        // match_classes (no state) doesn't match :hover
        assert!(!ss.match_classes("foo").contains_key("color"));
        assert_eq!(
            ss.match_classes("bar")
                .get("color")
                .map(|(v, _)| v.as_str()),
            Some("blue")
        );
        // match_element with hover state matches
        let state = ElementState {
            classes: vec!["foo".into()],
            is_hovered: true,
            ..Default::default()
        };
        let props = ss.match_element(&state, &[]);
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
    }

    #[test]
    fn test_skip_at_rules() {
        let css = "@keyframes spin { from { transform: rotate(0); } to { transform: rotate(360deg); } } .foo { color: red; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        assert_eq!(
            ss.match_classes("foo")
                .get("color")
                .map(|(v, _)| v.as_str()),
            Some("red")
        );
    }

    #[test]
    fn test_multiple_css_loads() {
        let mut ss = Stylesheet::new();
        ss.add_css(":root { --color: red; }");
        ss.add_css(".btn { color: var(--color); }");
        let props = ss.resolve_class_styles("btn");
        assert_eq!(props.get("color").unwrap(), "red");
    }

    #[test]
    fn test_later_rules_override() {
        let css = ".foo { color: red; } .foo { color: blue; }";
        let ss = Stylesheet::parse(css);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
    }

    #[test]
    fn test_comma_separated_selectors() {
        let css = ".a, .b { color: red; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(
            ss.match_classes("a").get("color").map(|(v, _)| v.as_str()),
            Some("red")
        );
        assert_eq!(
            ss.match_classes("b").get("color").map(|(v, _)| v.as_str()),
            Some("red")
        );
    }

    #[test]
    fn test_var_with_fallback() {
        let css = ".foo { color: var(--undefined, green); }";
        let ss = Stylesheet::parse(css);
        let props = ss.resolve_class_styles("foo");
        assert_eq!(props.get("color").unwrap(), "green");
    }

    #[test]
    fn test_css_comments_handled() {
        let css = "/* Comment at start */ .foo { color: red; /* inline comment */ font-size: 14px; } /* end */";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1);
        let props = ss.match_classes("foo");
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
        assert_eq!(
            props.get("font-size").map(|(v, _)| v.as_str()),
            Some("14px")
        );
    }

    #[test]
    fn test_descendant_selectors() {
        let css = ".a .b { color: red; } .c { color: blue; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 2);
        // match_classes without ancestors doesn't match descendant selector
        assert!(!ss.match_classes("b").contains_key("color"));
        assert_eq!(
            ss.match_classes("c").get("color").map(|(v, _)| v.as_str()),
            Some("blue")
        );
        // match_element with proper ancestor chain matches
        let element = ElementState::from_classes("b");
        let parent = ElementState::from_classes("a");
        let props = ss.match_element(&element, &[parent]);
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("red"));
    }

    #[test]
    fn test_strip_important_from_values() {
        let css = ".btn { color: red !important; background: blue!important; font-size: 14px; }";
        let ss = Stylesheet::parse(css);
        let props = ss.match_classes("btn");
        // !important should be stripped from all values, but tracked
        assert_eq!(
            props.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("red", true))
        );
        assert_eq!(
            props.get("background").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("blue", true))
        );
        assert_eq!(
            props.get("font-size").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("14px", false))
        );
    }

    #[test]
    fn test_strip_important_from_inline() {
        let css = ".foo { color: red; }";
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(
            &ss,
            Some("foo"),
            Some("color: green !important; padding: 10px!important"),
            None,
        );
        // Inline styles with !important should also have it stripped and override class styles
        assert_eq!(merged.get("color").unwrap(), "green");
        assert_eq!(merged.get("padding").unwrap(), "10px");
    }

    #[test]
    fn test_important_overrides() {
        let css = r#"
            .base { display: flex; color: red; }
            .hidden { display: none !important; }
        "#;
        let ss = Stylesheet::parse(css);

        // Both classes: !important display wins
        let merged = compute_merged_styles(&ss, Some("base hidden"), None, None);
        assert_eq!(merged.get("display").unwrap(), "none");

        // Inline can't override !important without !important
        let merged = compute_merged_styles(&ss, Some("hidden"), Some("display: flex"), None);
        assert_eq!(merged.get("display").unwrap(), "none");

        // Inline !important CAN override class !important
        let merged =
            compute_merged_styles(&ss, Some("hidden"), Some("display: flex !important"), None);
        assert_eq!(merged.get("display").unwrap(), "flex");
    }

    #[test]
    fn test_important_prevents_later_override() {
        let css = r#"
            .first { color: red !important; }
            .second { color: blue; }
        "#;
        let ss = Stylesheet::parse(css);

        // !important in first rule prevents second rule from overriding
        let props = ss.match_classes("first second");
        assert_eq!(
            props.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("red", true))
        );

        // But if second also has !important, it wins (later rule)
        let css2 = r#"
            .first { color: red !important; }
            .second { color: blue !important; }
        "#;
        let ss2 = Stylesheet::parse(css2);
        let props2 = ss2.match_classes("first second");
        assert_eq!(
            props2.get("color").map(|(v, imp)| (v.as_str(), *imp)),
            Some(("blue", true))
        );
    }

    #[test]
    fn test_universal_selector() {
        let css = "* { box-sizing: border-box; margin: 0; padding: 0; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 1, "Universal selector should parse");
        // Check that all 3 properties are stored
        let rule_props = &ss.rules[0].properties;
        eprintln!("Universal rule props: {:?}", rule_props);
        assert!(
            rule_props.contains_key("box-sizing"),
            "should have box-sizing"
        );
        assert!(rule_props.contains_key("margin"), "should have margin");
        assert!(rule_props.contains_key("padding"), "should have padding");
        // Universal selector matches any element with a tag
        let props = ss.match_classes_with_tag("", Some("div"));
        assert_eq!(
            props.get("box-sizing").map(|(v, _)| v.as_str()),
            Some("border-box")
        );
        assert_eq!(props.get("margin").map(|(v, _)| v.as_str()), Some("0"));
        assert_eq!(props.get("padding").map(|(v, _)| v.as_str()), Some("0"));
    }

    #[test]
    fn test_universal_plus_class_merge() {
        let css = r#"
            * { box-sizing: border-box; margin: 0; padding: 0; }
            .main-content { padding: 20px; }
        "#;
        let ss = Stylesheet::parse(css);
        let merged = compute_merged_styles(&ss, Some("main-content"), None, Some("div"));
        eprintln!("Merged main-content: {:?}", merged);
        assert_eq!(
            merged.get("box-sizing").map(|v| v.as_str()),
            Some("border-box")
        );
        assert_eq!(merged.get("margin").map(|v| v.as_str()), Some("0"));
        assert_eq!(merged.get("padding").map(|v| v.as_str()), Some("20px"));
    }

    #[test]
    fn test_element_type_selectors() {
        let css = "h1 { font-size: 2em; } a { color: blue; } code { font-family: monospace; }";
        let ss = Stylesheet::parse(css);
        assert_eq!(ss.rules.len(), 3);
        let props = ss.match_classes_with_tag("", Some("h1"));
        assert_eq!(props.get("font-size").map(|(v, _)| v.as_str()), Some("2em"));
        let props = ss.match_classes_with_tag("some-class", Some("a"));
        assert_eq!(props.get("color").map(|(v, _)| v.as_str()), Some("blue"));
    }

    #[test]
    fn test_borderless_window_matching() {
        let css = r#"
            .rinch-borderlesswindow { display: flex; flex-direction: column; height: 100vh; }
            .rinch-borderlesswindow--radius-md { border-radius: 8px; }
            .rinch-borderlesswindow__content { flex: 1; min-height: 0; }
            .rinch-borderlesswindow__titlebar { display: flex; height: 40px; }
        "#;
        let ss = Stylesheet::parse(css);
        assert_eq!(
            ss.rules.len(),
            4,
            "Expected 4 rules, got {}",
            ss.rules.len()
        );

        // Element with two classes
        let props = ss.match_classes("rinch-borderlesswindow rinch-borderlesswindow--radius-md");
        assert_eq!(
            props.get("display").map(|(v, _)| v.as_str()),
            Some("flex"),
            "display missing"
        );
        assert_eq!(
            props.get("height").map(|(v, _)| v.as_str()),
            Some("100vh"),
            "height missing"
        );
        assert_eq!(
            props.get("border-radius").map(|(v, _)| v.as_str()),
            Some("8px"),
            "border-radius missing"
        );

        // Content child
        let props2 = ss.match_classes("rinch-borderlesswindow__content");
        assert_eq!(
            props2.get("flex").map(|(v, _)| v.as_str()),
            Some("1"),
            "flex missing"
        );
    }

    #[test]
    fn test_inline_custom_property_resolution() {
        // Test that inline custom properties can reference global variables
        let css = r#"
            :root { --rinch-color-cyan-6: #0c8599; }
        "#;
        let ss = Stylesheet::parse(css);

        // Inline style defines a local custom property that references a global one
        let inline = "--rinch-action-icon-color: var(--rinch-color-cyan-6); background-color: var(--rinch-action-icon-color, transparent)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        // The local custom property should be resolved
        assert_eq!(merged.get("--rinch-action-icon-color").unwrap(), "#0c8599");
        // And it should be used to resolve other properties
        assert_eq!(merged.get("background-color").unwrap(), "#0c8599");
    }

    #[test]
    fn test_inline_custom_property_chain() {
        // Test chained local custom properties
        let css = r#"
            :root { --base-color: red; }
        "#;
        let ss = Stylesheet::parse(css);

        let inline =
            "--local-a: var(--base-color); --local-b: var(--local-a); color: var(--local-b)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        assert_eq!(merged.get("--local-a").unwrap(), "red");
        assert_eq!(merged.get("--local-b").unwrap(), "red");
        assert_eq!(merged.get("color").unwrap(), "red");
    }

    #[test]
    fn test_inline_custom_property_with_fallback() {
        // Test that fallbacks work with local custom properties
        let css = r#"
            :root { --rinch-color-blue: blue; }
        "#;
        let ss = Stylesheet::parse(css);

        let inline =
            "--local-color: var(--undefined, var(--rinch-color-blue)); color: var(--local-color)";
        let merged = compute_merged_styles(&ss, None, Some(inline), None);

        assert_eq!(merged.get("--local-color").unwrap(), "blue");
        assert_eq!(merged.get("color").unwrap(), "blue");
    }
}
