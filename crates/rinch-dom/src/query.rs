//! `querySelector` for the desktop DOM (#1796): Stylo's own selector parser
//! and matcher, the ones the cascade uses, run over the node tree.
//!
//! A selector is parsed as an author stylesheet's would be
//! ([`SelectorParser::parse_author_origin_no_namespace`]) and matched with
//! [`matches_selector_list`] over [`RinchNode`], so whatever the cascade
//! matches a query matches: combinators, compounds, selector lists, `:not()`,
//! `:is()`, the attribute operators and their `i` flag, and the pseudo-classes
//! `stylo_impl` answers. Matching is read-only: no selector flags are written
//! (`NeedsSelectorFlags::No`), no `:hover`/`:active`/`:focus` sensitivity flag
//! either ([`querying`]), nothing is restyled or laid out, and no bloom filter
//! is consulted.
//!
//! The candidates are the elements under a scope in document order, as
//! `Element.querySelectorAll`: the scope itself is never one, the selector is
//! matched against the whole tree (an ancestor above the scope can satisfy a
//! combinator), and `:scope` is the scope element. Generated pseudo-element
//! nodes (`::before`, `::after`, list markers), which a browser keeps out of
//! its DOM, are not candidates and are not walked.
//!
//! An invalid selector — which a browser throws a `SyntaxError` for — answers
//! nothing and logs one `tracing::warn!` per distinct string, for the first
//! [`rinch_core::dom::MAX_WARNED_SELECTORS`] strings. `:has()` is one: Stylo's servo build does not parse
//! it (#680), though a browser does.

use std::cell::Cell;

use selectors::Element;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::matches_selector_list;
use selectors::parser::SelectorList;
use style::selector_parser::{SelectorImpl, SelectorParser};

use crate::node::{NodeKind, NodeTree, RawNodeId};
use crate::stylo_impl::RinchNode;

thread_local! {
    /// Set while [`select`] matches. `stylo_impl`'s `:hover`/`:active`/
    /// `:focus` arms then leave `hover_sensitive` & co. alone: those flags are
    /// invalidation state — they make a state change snapshot the node — and a
    /// query is not a rule the cascade has to keep current.
    static QUERYING: Cell<bool> = const { Cell::new(false) };
}

/// Whether a query is matching on this thread ([`select`]).
pub(crate) fn querying() -> bool {
    QUERYING.with(Cell::get)
}

/// Restores [`QUERYING`] on drop (a panicking match included).
struct QueryingGuard(bool);

impl Drop for QueryingGuard {
    fn drop(&mut self) {
        QUERYING.with(|q| q.set(self.0));
    }
}

/// A parsed selector list.
pub type Selectors = SelectorList<SelectorImpl>;

/// Parse `selector` as a CSS selector list, or `None` when it is not one.
/// Does not warn.
pub fn parse(selector: &str) -> Option<Selectors> {
    SelectorParser::parse_author_origin_no_namespace(selector, &crate::layout::BLANK_URL_DATA).ok()
}

/// [`parse`], warning once per distinct invalid string.
pub fn parse_or_warn(selector: &str) -> Option<Selectors> {
    let parsed = parse(selector);
    if parsed.is_none() {
        rinch_core::dom::warn_invalid_selector(selector);
    }
    parsed
}

/// The elements under `scope` (and `scope` itself when `include_scope`)
/// matching `selectors`, in document order; at most one when `first_only`.
pub fn select(
    tree: &NodeTree,
    scope: RawNodeId,
    selectors: &Selectors,
    include_scope: bool,
    first_only: bool,
) -> Vec<RawNodeId> {
    let mut out = Vec::new();
    let Some(scope_node) = tree.nodes.get(scope) else {
        return out;
    };
    let _guard = QueryingGuard(QUERYING.with(|q| q.replace(true)));
    let mut caches = SelectorCaches::default();
    let mut ctx = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    if matches!(scope_node.kind, NodeKind::Element(_)) {
        ctx.scope_element = Some(RinchNode::new(scope, tree).opaque());
    }

    // Pre-order, children pushed in reverse so they pop in order.
    let mut stack = if include_scope {
        vec![scope]
    } else {
        scope_node.children.iter().rev().copied().collect()
    };
    while let Some(id) = stack.pop() {
        let Some(node) = tree.nodes.get(id) else {
            continue;
        };
        if node.is_pseudo_element {
            continue;
        }
        if matches!(node.kind, NodeKind::Element(_))
            && matches_selector_list(selectors, &RinchNode::new(id, tree), &mut ctx)
        {
            out.push(id);
            if first_only {
                return out;
            }
        }
        stack.extend(node.children.iter().rev().copied());
    }
    out
}

/// `scope.querySelectorAll(selector)`: [`select`] after [`parse_or_warn`].
pub fn query_all(tree: &NodeTree, scope: RawNodeId, selector: &str) -> Vec<RawNodeId> {
    match parse_or_warn(selector) {
        Some(list) => select(tree, scope, &list, false, false),
        None => Vec::new(),
    }
}

/// `scope.querySelector(selector)`.
pub fn query_first(tree: &NodeTree, scope: RawNodeId, selector: &str) -> Option<RawNodeId> {
    let list = parse_or_warn(selector)?;
    select(tree, scope, &list, false, true).into_iter().next()
}
