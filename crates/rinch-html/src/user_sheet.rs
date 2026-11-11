//! A user style sheet: the app's own CSS for a document it did not write, as
//! a browser's user sheet (a mail client hiding quoted text behind a button,
//! greying a signature).
//!
//! Its selectors are matched against the document **as written**, parsed as
//! a browser parses it (html5ever, into [`depth`](crate::depth)'s arena),
//! before the allow-list strips the classes, ids and attributes (`type`,
//! `name`, `data-*`) they name. The matcher is the `selectors` crate, the one
//! Stylo and css-inline use. Each matched element is marked with
//! [`MARK`]`="i j"`, the indices of the rules it matched; the allow-list keeps
//! that attribute (and drops any the document wrote itself), and the view
//! adds each rule's declarations to the marked elements' `style`
//! ([`HtmlView`](crate::HtmlView)).
//!
//! Matching costs no pass over a parent's children per element: `A ~ B`
//! with `B` one compound (`#marker ~ *`, `.x ~ p.y`) is matched by one walk
//! along each parent's children, and the other combinators by the
//! `selectors` crate (`+` looks at one sibling, ` ` and `>` at ancestors).
//! Any other use of `~` falls to the crate, which walks back over earlier
//! siblings.

use std::borrow::Cow;
use std::fmt;

use cssparser::{Delimiter, ToCss, Token};
use html5ever::tendril::TendrilSink;
use html5ever::{Attribute, LocalName as Atom, Namespace, QualName, local_name, ns};
use precomputed_hash::PrecomputedHash;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::{ElementSelectorFlags, matches_selector_list};
use selectors::parser::{ParseRelative, SelectorParseErrorKind};
use selectors::{OpaqueElement, SelectorList};

use crate::depth::{Arena, Id, Kind, Node};

/// The attribute a matched element is marked with: the indices of the rules
/// it matched, space-separated, so `[data-rinch-user~="2"]` selects rule 2's.
pub const MARK: &str = "data-rinch-user";

/// One rule of a user sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    /// The selector list, as written (trimmed).
    pub selector: String,
    /// Its declarations, each made `!important`, joined by `; `.
    pub declarations: String,
}

/// The style rules of `css`, in order. At-rules (`@media`, `@import`…) are
/// left out, and so is a rule with no selector; a declaration that does not
/// parse is left out of its rule.
pub(crate) fn rules(css: &str) -> Vec<Rule> {
    let mut parser = cssparser::Parser::new(css);
    let mut out = Vec::new();
    loop {
        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }
        let start = parser.position();
        let state = parser.state();
        let at_rule = matches!(parser.next(), Ok(Token::AtKeyword(_)));
        parser.reset(&state);
        let prelude: Result<(), cssparser::ParseError<()>> =
            parser.parse_until_before(Delimiter::CurlyBracketBlock | Delimiter::Semicolon, |p| {
                while p.next().is_ok() {}
                Ok(())
            });
        let _ = prelude;
        let selector = parser.slice_from(start).trim().to_owned();
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {}
            // `@import …;` or a stray `;`.
            Ok(_) => continue,
            Err(_) => break,
        }
        let block: Result<String, cssparser::ParseError<()>> =
            parser.parse_nested_block(|p| Ok(declarations(p)));
        let declarations = block.unwrap_or_default();
        if !at_rule && !selector.is_empty() {
            out.push(Rule {
                selector,
                declarations,
            });
        }
    }
    out
}

/// A block's declarations, each `!important`, joined by `; `.
fn declarations(p: &mut cssparser::Parser<'_>) -> String {
    let mut out: Vec<String> = Vec::new();
    while !p.is_exhausted() {
        let start = p.position();
        let one: Result<(), cssparser::ParseError<()>> =
            p.parse_until_after(Delimiter::Semicolon, |p| {
                while p.next().is_ok() {}
                Ok(())
            });
        let _ = one;
        let text = p.slice_from(start).trim().trim_end_matches(';').trim();
        let Some((name, value)) = text.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let mut value = value.trim();
        if let Some(i) = value.to_ascii_lowercase().rfind("!important")
            && value[i + "!important".len()..].trim().is_empty()
        {
            value = value[..i].trim_end();
        }
        let ident = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if ident && !value.is_empty() {
            out.push(format!("{name}: {value} !important"));
        }
    }
    out.join("; ")
}

/// `document` with each element that `selectors` match marked ([`MARK`]),
/// and how many elements each matched. Borrowed when there are no
/// selectors; otherwise written back as html5ever writes a parsed document
/// (`<html><head>…<body>…`), with every [`MARK`] the document wrote itself
/// removed. A selector that does not parse matches nothing. The document's
/// `<html>`, `<head>` and `<body>` are never marked.
pub(crate) fn mark<'a>(document: &'a str, selectors: &[String]) -> (Cow<'a, str>, Vec<usize>) {
    if selectors.is_empty() {
        return (Cow::Borrowed(document), Vec::new());
    }
    let compiled: Vec<Vec<Part>> = selectors.iter().map(|s| compile(s)).collect();
    let arena = html5ever::parse_document(Arena::default(), Default::default()).one(document);
    let matched = {
        let nodes = arena.nodes.borrow();
        matching(&nodes, &compiled)
    };
    let mut counts = vec![0; selectors.len()];
    let mark = QualName::new(None, ns!(), Atom::from(MARK));
    let mut nodes = arena.nodes.borrow_mut();
    for (id, node) in nodes.iter_mut().enumerate() {
        if let Kind::Element { attrs, .. } = &mut node.kind {
            attrs.retain(|a| a.name != mark);
            if !matched[id].is_empty() {
                for &i in &matched[id] {
                    counts[i] += 1;
                }
                let value: Vec<String> = matched[id].iter().map(usize::to_string).collect();
                attrs.push(Attribute {
                    name: mark.clone(),
                    value: value.join(" ").into(),
                });
            }
        }
    }
    drop(nodes);
    (
        Cow::Owned(crate::depth::write(&arena, document.len())),
        counts,
    )
}

/// One selector of a rule's list, ready to match.
enum Part {
    /// Matched by the `selectors` crate.
    Plain(SelectorList<Impl>),
    /// `left ~ right`, `right` one compound: matched along each parent's
    /// children in one walk.
    Later {
        left: SelectorList<Impl>,
        right: SelectorList<Impl>,
    },
}

/// A rule's selector list, split at its commas. Nothing if the list does
/// not parse (as a browser drops the whole rule).
fn compile(selector: &str) -> Vec<Part> {
    if parse(selector).is_err() {
        return Vec::new();
    }
    top_level_split(selector, |t| matches!(t, Token::Comma))
        .into_iter()
        .filter_map(|one| {
            let one = one.trim();
            if let Some((left, right)) = later_split(one)
                && let (Ok(left), Ok(right)) = (parse(left), parse(right))
            {
                return Some(Part::Later { left, right });
            }
            parse(one).ok().map(Part::Plain)
        })
        .collect()
}

/// `text` cut at each top-level token `at` says to cut at (nested blocks,
/// strings and attribute brackets are not top level).
fn top_level_split(text: &str, at: impl Fn(&Token<'_>) -> bool) -> Vec<&str> {
    let mut parser = cssparser::Parser::new(text);
    let mut out = Vec::new();
    let mut from = 0;
    loop {
        let before = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        if at(token) {
            out.push(&text[from..before]);
            from = parser.position().byte_index();
        }
    }
    out.push(&text[from..]);
    out
}

/// `(left, right)` when `selector` is `left ~ right` with `right` a single
/// compound (no combinator in it).
fn later_split(selector: &str) -> Option<(&str, &str)> {
    let pieces = top_level_split(selector, |t| matches!(t, Token::Delim('~')));
    if pieces.len() < 2 {
        return None;
    }
    let right = pieces.last()?.trim();
    let left_len = selector.len() - pieces.last()?.len() - 1;
    let left = selector[..left_len].trim();
    let combinator = top_level_split(right, |t| {
        matches!(t, Token::WhiteSpace(_) | Token::Delim('>' | '+' | '~'))
    });
    (combinator.len() == 1 && !left.is_empty() && !right.is_empty()).then_some((left, right))
}

/// For each node, the indices of the rules it matched, in order.
fn matching(nodes: &[Node], rules: &[Vec<Part>]) -> Vec<Vec<usize>> {
    // Each node's place among its parent's children, for the sibling
    // combinators.
    let mut index = vec![0; nodes.len()];
    for node in nodes {
        for (i, &c) in node.children.iter().enumerate() {
            index[c] = i;
        }
    }
    let mut out = vec![Vec::new(); nodes.len()];
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    let candidate = |id: Id| match &nodes[id].kind {
        Kind::Element { name, .. } => {
            !(name.ns == ns!(html)
                && matches!(
                    name.local,
                    local_name!("html") | local_name!("head") | local_name!("body")
                ))
        }
        _ => false,
    };
    // Every node is some parent's child once, so this visits each element
    // once, and each `~` part walks each child list once.
    for parent in nodes {
        // Per rule and part: whether an earlier sibling matched its `left`.
        let mut seen: Vec<Vec<bool>> = rules.iter().map(|r| vec![false; r.len()]).collect();
        for &id in &parent.children {
            if !matches!(nodes[id].kind, Kind::Element { .. }) {
                continue;
            }
            let element = El {
                nodes,
                index: &index,
                id,
            };
            let is_candidate = candidate(id);
            for (r, parts) in rules.iter().enumerate() {
                let mut hit = false;
                for (p, part) in parts.iter().enumerate() {
                    match part {
                        Part::Plain(list) => {
                            hit = hit
                                || (is_candidate
                                    && matches_selector_list(list, &element, &mut context));
                        }
                        Part::Later { left, right } => {
                            hit = hit
                                || (is_candidate
                                    && seen[r][p]
                                    && matches_selector_list(right, &element, &mut context));
                            if !seen[r][p] && matches_selector_list(left, &element, &mut context) {
                                seen[r][p] = true;
                            }
                        }
                    }
                }
                if hit {
                    out[id].push(r);
                }
            }
        }
    }
    out
}

fn parse(selector: &str) -> Result<SelectorList<Impl>, String> {
    let mut parser = cssparser::Parser::new(selector);
    SelectorList::parse(&SelectorParser, &mut parser, ParseRelative::No)
        .map_err(|e| format!("{:?}", e.kind))
}

// ── The `selectors` crate's view of the arena ──────────────────────────────

/// A name (tag, attribute, class, id) as the `selectors` crate needs it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Name(Atom);

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Self(Atom::from(s))
    }
}

impl ToCss for Name {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(&self.0)
    }
}

impl PrecomputedHash for Name {
    fn precomputed_hash(&self) -> u32 {
        self.0.precomputed_hash()
    }
}

/// An attribute value in a selector.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Value(String);

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl AsRef<str> for Value {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for Value {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_string(&self.0, dest)
    }
}

/// No pseudo-class beyond the structural ones the crate knows itself.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PseudoClass {}

impl ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl selectors::parser::NonTSPseudoClass for PseudoClass {
    fn is_active_or_hover(&self) -> bool {
        match *self {}
    }
    fn is_user_action_state(&self) -> bool {
        match *self {}
    }
}

/// No pseudo-element: a document's elements are what is left out.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PseudoElement {}

impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl selectors::parser::PseudoElement for PseudoElement {}

#[derive(Debug, Clone)]
struct Impl;

impl selectors::SelectorImpl for Impl {
    type ExtraMatchingData<'a> = std::marker::PhantomData<&'a ()>;
    type AttrValue = Value;
    type Identifier = Name;
    type LocalName = Name;
    type NamespaceUrl = Namespace;
    type NamespacePrefix = Name;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = Name;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElement;
}

struct SelectorParser;

impl<'i> selectors::Parser<'i> for SelectorParser {
    type Impl = Impl;
    type Error = SelectorParseErrorKind;
}

/// An element of the arena.
#[derive(Clone, Copy)]
struct El<'a> {
    nodes: &'a [Node],
    index: &'a [usize],
    id: Id,
}

impl fmt::Debug for El<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "El({})", self.id)
    }
}

impl<'a> El<'a> {
    fn at(&self, id: Id) -> Option<Self> {
        matches!(self.nodes[id].kind, Kind::Element { .. }).then_some(Self { id, ..*self })
    }

    fn name(&self) -> &'a html5ever::QualName {
        match &self.nodes[self.id].kind {
            Kind::Element { name, .. } => name,
            _ => unreachable!("an El is an element"),
        }
    }

    fn attr(&self, local: &Atom) -> Option<&'a str> {
        match &self.nodes[self.id].kind {
            Kind::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name.ns == ns!() && a.name.local == *local)
                .map(|a| a.value.as_ref()),
            _ => None,
        }
    }

    fn siblings(&self) -> &'a [Id] {
        match self.nodes[self.id].parent {
            Some(p) => &self.nodes[p].children,
            None => &[],
        }
    }
}

impl selectors::Element for El<'_> {
    type Impl = Impl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(&self.nodes[self.id])
    }

    fn parent_element(&self) -> Option<Self> {
        self.at(self.nodes[self.id].parent?)
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
        #[cfg(test)]
        tests::SIBLING_STEPS.with(|s| s.set(s.get() + 1));
        let at = self.index[self.id];
        self.siblings()[..at].iter().rev().find_map(|&s| self.at(s))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let at = self.index[self.id];
        self.siblings()
            .get(at + 1..)?
            .iter()
            .find_map(|&s| self.at(s))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.nodes[self.id]
            .children
            .iter()
            .find_map(|&c| self.at(c))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.name().ns == ns!(html)
    }

    fn has_local_name(&self, name: &Name) -> bool {
        self.name().local == name.0
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.name().ns == *ns
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.name() == other.name()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&Namespace>,
        local: &Name,
        operation: &AttrSelectorOperation<&Value>,
    ) -> bool {
        let Kind::Element { attrs, .. } = &self.nodes[self.id].kind else {
            return false;
        };
        attrs.iter().any(|a| {
            a.name.local == local.0
                && match ns {
                    NamespaceConstraint::Any => true,
                    NamespaceConstraint::Specific(ns) => a.name.ns == **ns,
                }
                && operation.eval_str(&a.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo: &PseudoClass,
        _context: &mut MatchingContext<'_, Impl>,
    ) -> bool {
        match *pseudo {}
    }

    fn match_pseudo_element(
        &self,
        pseudo: &PseudoElement,
        _context: &mut MatchingContext<'_, Impl>,
    ) -> bool {
        match *pseudo {}
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        self.name().ns == ns!(html)
            && matches!(
                self.name().local,
                local_name!("a") | local_name!("area") | local_name!("link")
            )
            && self.attr(&local_name!("href")).is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &Name, case: CaseSensitivity) -> bool {
        self.attr(&local_name!("id"))
            .is_some_and(|have| case.eq(have.as_bytes(), id.0.as_bytes()))
    }

    fn has_class(&self, name: &Name, case: CaseSensitivity) -> bool {
        self.attr(&local_name!("class")).is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|c| case.eq(c.as_bytes(), name.0.as_bytes()))
        })
    }

    fn has_custom_state(&self, _name: &Name) -> bool {
        false
    }

    fn imported_part(&self, _name: &Name) -> Option<Name> {
        None
    }

    fn is_part(&self, _name: &Name) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.nodes[self.id]
            .children
            .iter()
            .all(|&c| match &self.nodes[c].kind {
                Kind::Element { .. } => false,
                Kind::Text(t) => t.is_empty(),
                _ => true,
            })
    }

    fn is_root(&self) -> bool {
        self.nodes[self.id]
            .parent
            .is_some_and(|p| matches!(self.nodes[p].kind, Kind::Document))
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{MARK, mark, rules};

    thread_local! {
        /// Calls of `prev_sibling_element`: the walk back over siblings.
        pub(super) static SIBLING_STEPS: Cell<usize> = const { Cell::new(0) };
    }

    fn steps(doc: &str, selector: &str) -> (usize, usize) {
        SIBLING_STEPS.with(|s| s.set(0));
        let (out, counts) = mark(doc, &[selector.to_owned()]);
        assert!(out.contains(MARK));
        (counts[0], SIBLING_STEPS.with(Cell::get))
    }

    /// `#m ~ *` over n siblings walks no sibling list per element: an
    /// operation count, not a timing (the crate's own `~` walks back to the
    /// marker from each, n²/2 steps).
    #[test]
    fn a_later_sibling_selector_costs_no_walk_back_per_element() {
        let n = 2_000;
        let doc = format!("<p>keep</p><i id=m></i>{}", "<p>x</p>text".repeat(n));
        let (matched, walked) = steps(&doc, "#m ~ *");
        assert_eq!(matched, n);
        assert!(walked < n, "{walked} sibling steps for {n} siblings");
        let (matched, walked) = steps(&doc, "#m ~ p, i ~ p.none");
        assert_eq!(matched, n);
        assert!(walked < n, "{walked}");
        // `+` looks at one sibling each.
        let (matched, walked) = steps(&doc, "#m + p");
        assert_eq!(matched, 1);
        assert!(walked <= 2 * n + 2, "{walked}");
    }

    #[test]
    fn rules_are_read_as_a_style_sheet() {
        let r = rules(
            "/* c */ a.b , c { color : red ; margin:0!IMPORTANT; bad; :x; }\n\
             @media x { d { e: f } } @import 'y'; ; g{}",
        );
        let got: Vec<(&str, &str)> = r
            .iter()
            .map(|r| (r.selector.as_str(), r.declarations.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("a.b , c", "color: red !important; margin: 0 !important"),
                ("g", "")
            ]
        );
    }
}
