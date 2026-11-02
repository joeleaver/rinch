//! #1796: desktop `query_selector` matches the CSS selectors the web does.
//!
//! The table (`support/query_selector_table.rs`) is shared with the browser
//! twin `crates/rinch-web/tests/query_selector_1796.rs`, which checks every
//! row against Chrome's own `Element.querySelectorAll` first: the browser is
//! the oracle for the expectations. Before #1796 desktop matched one simple
//! selector (`#id`, `.class`, `[attr]`, `[attr=value]`, a tag), searched the
//! whole document whatever the node, and answered nothing — silently — for
//! anything else.
use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
use rinch_dom::RinchDocument;

#[path = "support/query_selector_table.rs"]
mod table;

fn doc_with_markup() -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "id", "host");
    doc.append_child(body, host);
    doc.set_inner_html(host, table::MARKUP);
    doc
}

fn by_id(doc: &RinchDocument, id: &str) -> NodeId {
    let found = rinch_dom::testing::query_selector(&doc.tree, &format!("[id={id}]"));
    assert_eq!(found.len(), 1, "one #{id}");
    NodeId(found[0])
}

fn ids(doc: &RinchDocument, nodes: &[NodeId]) -> Vec<String> {
    nodes
        .iter()
        .map(|&n| doc.get_attribute(n, "id").unwrap_or_else(|| "?".into()))
        .collect()
}

#[test]
fn every_row_of_the_table_answers_as_chrome() {
    let doc = doc_with_markup();
    let mut wrong = Vec::new();
    for &(scope, selector, want) in table::CASES {
        let scope = by_id(&doc, scope);
        let all = ids(&doc, &doc.query_selector_all_in(scope, selector));
        let first = doc
            .query_selector_in(scope, selector)
            .map(|n| ids(&doc, &[n]).remove(0));
        if all != want || first.as_deref() != want.first().copied() {
            wrong.push(format!(
                "{selector:?} in #{scope:?}: all {all:?} first {first:?}, want {want:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} rows wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn an_invalid_selector_answers_nothing() {
    let doc = doc_with_markup();
    let host = by_id(&doc, "host");
    for &sel in table::INVALID {
        assert!(rinch_dom::query::parse(sel).is_none(), "{sel:?} parses");
        assert_eq!(doc.query_selector_in(host, sel), None, "{sel:?}");
        assert!(doc.query_selector_all_in(host, sel).is_empty(), "{sel:?}");
        assert_eq!(doc.query_selector(sel), None, "{sel:?}");
        assert!(doc.query_selector_all(sel).is_empty(), "{sel:?}");
    }
    // Positive control: a valid selector on the same document answers.
    assert!(doc.query_selector_in(host, "span").is_some());
}

/// The document-wide forms keep searching the whole document, now with any
/// selector.
#[test]
fn the_document_wide_query_takes_any_selector() {
    let doc = doc_with_markup();
    assert_eq!(
        ids(&doc, &doc.query_selector_all(".panel--right .panel__body")),
        ["body"]
    );
    assert_eq!(
        ids(&doc, &doc.query_selector_all("body > div > div")),
        ["root", "other"]
    );
    assert_eq!(
        doc.query_selector("section > p > span")
            .map(|n| ids(&doc, &[n]).remove(0)),
        Some("s1".to_string())
    );
}

/// `NodeHandle::query_selector{,_all}` are scoped to the handle's subtree.
#[test]
fn a_node_handle_queries_its_own_subtree() {
    let doc = doc_with_markup();
    let section = by_id(&doc, "body");
    let other = by_id(&doc, "other");
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(doc));
    let weak = Rc::downgrade(&doc);
    let h = NodeHandle::new(section, weak.clone());
    let spans = h.query_selector_all(".a");
    assert_eq!(spans.len(), 2, "s1 and s2, not s3 outside the section");
    let first = h.query_selector(".a.b").expect("s1");
    assert_eq!(first.get_attribute("id").as_deref(), Some("s1"));
    assert!(
        h.query_selector("section").is_none(),
        "the handle itself is no candidate"
    );
    let o = NodeHandle::new(other, weak);
    assert_eq!(
        o.query_selector(".a")
            .and_then(|n| n.get_attribute("id"))
            .as_deref(),
        Some("s3")
    );
}

/// A generated `::before` box is a node in rinch-dom's tree, never a match.
#[test]
fn generated_boxes_are_not_candidates() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style = doc.create_element("style");
    let css = doc.create_text("p::before { content: 'x' } p::after { content: 'y' }");
    doc.append_child(style, css);
    let head = doc.query_selector("head").unwrap_or(body);
    doc.append_child(head, style);
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.set_inner_html(host, "<p><span>a</span></p>");
    doc.resolve_layout(400.0, 300.0);
    let p = doc.query_selector("p").expect("p");
    assert!(
        doc.get_children(p).len() > 1,
        "positive control: the ::before / ::after boxes exist as nodes"
    );
    assert_eq!(
        doc.query_selector_all_in(host, "*").len(),
        2,
        "p and span only"
    );
    assert_eq!(doc.query_selector_all_in(p, "*").len(), 1, "span only");
}

/// The MCP tool's matcher keeps its documented syntax, unquoted numbers
/// included (invalid CSS, read by the old simple matcher), and takes any CSS.
#[test]
fn the_testing_matcher_keeps_its_old_syntax_and_takes_css() {
    let mut doc = doc_with_markup();
    let b1 = by_id(&doc, "b1");
    doc.set_attribute(b1, "data-rid", "5");
    let q = |s: &str| {
        ids(
            &doc,
            &rinch_dom::testing::query_selector(&doc.tree, s)
                .into_iter()
                .map(NodeId)
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(q("[data-rid=5]"), ["b1"]);
    assert_eq!(q("[data-rid]"), ["b1"]);
    assert_eq!(q("button"), ["b1", "b2"]);
    assert_eq!(q(".secondary"), ["b2"]);
    assert_eq!(q(".panel--right .panel__body"), ["body"]);
    assert_eq!(q("body")[0], "?", "it still answers <body> itself");
}

/// A query is no stylesheet rule: matching `:hover`, `:active` or `:focus`
/// leaves the sensitivity flags alone, which would otherwise make every hover
/// change on the elements it tested snapshot them for invalidation.
#[test]
fn a_state_query_marks_nothing_sensitive() {
    let mut doc = doc_with_markup();
    doc.resolve_layout(400.0, 300.0);
    let host = by_id(&doc, "host");
    let sensitive = |doc: &RinchDocument| {
        doc.tree
            .nodes
            .iter()
            .filter(|(_, n)| {
                n.hover_sensitive.get() || n.active_sensitive.get() || n.focus_sensitive.get()
            })
            .count()
    };
    let before = sensitive(&doc);
    assert!(
        doc.query_selector_all_in(host, "span:hover, p:active, button:focus, :focus-visible")
            .is_empty()
    );
    assert_eq!(sensitive(&doc), before, "the query set sensitivity flags");
    // Positive control: the same selectors in a stylesheet do mark them.
    let style = doc.create_element("style");
    let css = doc.create_text("span:hover { color: red }");
    doc.append_child(style, css);
    let body = doc.body();
    doc.append_child(body, style);
    doc.resolve_layout(401.0, 300.0);
    assert!(
        sensitive(&doc) > before,
        "a stylesheet :hover marks its subjects"
    );
}

/// Nor does a query record selector flags: they are what makes an insertion
/// or removal restyle siblings (`HAS_SLOW_SELECTOR*`, `HAS_EDGE_CHILD_SELECTOR`,
/// `HAS_EMPTY_SELECTOR`), and a query is no stylesheet rule.
#[test]
fn a_structural_query_records_no_selector_flags() {
    let mut doc = doc_with_markup();
    doc.resolve_layout(400.0, 300.0);
    let host = by_id(&doc, "host");
    let flags = |doc: &RinchDocument| -> Vec<(u64, usize)> {
        doc.tree
            .nodes
            .iter()
            .map(|(id, n)| (id, n.selector_flags.borrow().bits()))
            .collect()
    };
    let before = flags(&doc);
    let found = doc.query_selector_all_in(
        host,
        "li:nth-child(2), li:last-child, li:empty, header + section, #b1 ~ p",
    );
    assert_eq!(found.len(), 5, "positive control: the query matched");
    assert_eq!(flags(&doc), before, "the query recorded selector flags");
}
