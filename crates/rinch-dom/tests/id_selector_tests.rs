//! #675 — an id selector (`#foo { … }`) must match on desktop.
//!
//! Stylo's `SelectorMap` files a rule whose rightmost compound carries an `#id`
//! into the **id bucket only** (`selector_map.rs`'s `Bucket::ID` arm writes
//! `self.id_hash`, `Bucket::Universal` writes `self.other`, and `find_bucket`
//! keeps exactly one). `get_all_matching_rules` then consults that bucket
//! behind `if let Some(id) = rule_hash_target.id() { … }` — with no `else` and
//! no fallback. So an element whose `TElement::id()` answers `None` is never
//! offered a single id-keyed rule, however correct its `has_id` is: `has_id`
//! is the *predicate*, reached only after the bucket hands the rule over.
//!
//! rinch-dom's `id()` returned a hard `None` behind a TODO until this fix, so
//! `#foo { … }` was silently dropped on desktop while working on rinch-web
//! (the browser matches ids itself).
//!
//! Every fixture here declares a tag rule as its **positive control**, so a
//! failure that is really "the stylesheet did not parse" or "the cascade did
//! not run" is distinguishable from "the id rule did not match".

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn margin_top(doc: &RinchDocument, n: NodeId) -> f32 {
    doc.tree.get(n.0).unwrap().computed_style.margin_top.to_px()
}

fn margin_bottom(doc: &RinchDocument, n: NodeId) -> f32 {
    doc.tree
        .get(n.0)
        .unwrap()
        .computed_style
        .margin_bottom
        .to_px()
}

fn hex(doc: &RinchDocument, n: NodeId) -> String {
    let c = doc.tree.get(n.0).unwrap().computed_style.color;
    let c = c.expect("color should resolve");
    let rgba = c.to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

/// A `<div>` under `<body>` carrying the given attributes.
fn div(doc: &mut RinchDocument, parent: NodeId, attrs: &[(&str, &str)]) -> NodeId {
    let n = doc.create_element("div");
    for (k, v) in attrs {
        doc.set_attribute(n, k, v);
    }
    doc.append_child(parent, n);
    n
}

/// The issue's own table: an id rule, a class rule, and a tag rule that is the
/// positive control.
///
/// Kills the mutant `fn id(&self) -> Option<&Atom> { None }` — the state this
/// file was written against.
#[test]
fn an_id_selector_matches_where_a_class_selector_does() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "#byid { margin-top: 11px; } .bycls { margin-top: 13px; } div { margin-bottom: 17px; }",
    );
    let body = doc.body();

    let byid = div(&mut doc, body, &[("id", "byid")]);
    let bycls = div(&mut doc, body, &[("class", "bycls")]);
    let bare = div(&mut doc, body, &[]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (
            margin_bottom(&doc, byid),
            margin_bottom(&doc, bycls),
            margin_bottom(&doc, bare)
        ),
        (17.0, 17.0, 17.0),
        "positive control: the tag rule must land on all three, or the \
         stylesheet never parsed and nothing below means anything"
    );
    assert_eq!(
        margin_top(&doc, bycls),
        13.0,
        "positive control: the class rule lands (it always did — `each_class` \
         is implemented where `id()` was not)"
    );
    assert_eq!(
        margin_top(&doc, bare),
        0.0,
        "negative control: nothing gives the bare div a top margin"
    );
    assert_eq!(
        margin_top(&doc, byid),
        11.0,
        "#675: the id rule must reach the element with that id"
    );
}

/// `#a.b` — a compound where the id is the most specific component, so
/// `find_bucket` files it under `Bucket::ID` even though a class is present.
/// The class half must still be required.
#[test]
fn an_id_compounded_with_a_class_needs_both() {
    let mut doc = RinchDocument::new();
    doc.load_css("#a.b { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();

    let both = div(&mut doc, body, &[("id", "a"), ("class", "b")]);
    let id_only = div(&mut doc, body, &[("id", "a")]);
    let class_only = div(&mut doc, body, &[("class", "b")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, both),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(margin_top(&doc, both), 11.0, "#a.b matches id + class");
    assert_eq!(
        (margin_top(&doc, id_only), margin_top(&doc, class_only)),
        (0.0, 0.0),
        "#a.b must not match half of itself — a bucket lookup that skipped \
         `has_id`/`has_class` entirely would light both of these up"
    );
}

/// Combinators keyed by an id on either side.
///
/// **Only the second half discriminates, and that is measured rather than
/// assumed.** `#a > p` puts the id on the *ancestor* side, so the rightmost
/// compound is `p`, the rule is filed in the local-name bucket, and it is
/// offered to every `<p>` whatever `id()` answers — matching then walks up
/// through `has_id`, which was correct all along. Run against the
/// `id() -> None` mutant this assertion **passes**: it is a fixed point, kept
/// as coverage of the ancestor side rather than as a pin on #675. (The bloom
/// filter would have rejected it, since an absent id hash is a fast-reject —
/// but `style_resolution/resolve.rs` passes `None` for the bloom, so that
/// consumer is dormant in rinch.)
///
/// `section #b` is the discriminating half: the id is the *subject*, the rule
/// is id-bucketed, and it is the assertion that goes red against that mutant.
#[test]
fn id_keyed_child_and_descendant_combinators_match() {
    let mut doc = RinchDocument::new();
    doc.load_css("#a > p { margin-top: 11px; } section #b { margin-top: 23px; } div { margin-bottom: 17px; }");
    let body = doc.body();

    let a = div(&mut doc, body, &[("id", "a")]);
    let child_p = doc.create_element("p");
    doc.append_child(a, child_p);

    // Same tag, same id-less parent: the control for `#a > p`.
    let other = div(&mut doc, body, &[]);
    let other_p = doc.create_element("p");
    doc.append_child(other, other_p);

    let section = doc.create_element("section");
    doc.append_child(body, section);
    let b = div(&mut doc, section, &[("id", "b")]);
    // Same id, no `section` ancestor: the control for `section #b`.
    let b_loose = div(&mut doc, body, &[("id", "b")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, a),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        margin_top(&doc, child_p),
        11.0,
        "#a > p: an id on the ancestor side of a child combinator (a fixed \
         point — see this test's doc)"
    );
    assert_eq!(
        margin_top(&doc, other_p),
        16.0,
        "…and only under that ancestor — 16px is the UA sheet's own \
         `p` block-margin rule (#674, `margin-block: 1em`) at the default 16px root, i.e. the \
         rule did not match, not that the element has no margin"
    );
    assert_eq!(
        margin_top(&doc, b),
        23.0,
        "section #b: an id as the subject of a descendant combinator"
    );
    assert_eq!(
        margin_top(&doc, b_loose),
        0.0,
        "…and only under that ancestor"
    );
}

/// `#a:hover` — the id bucket and the `:hover` state must compose. rinch
/// tracks hover in `Node::is_hovered` and reports it through
/// `TElement::state()`, so this is a real state selector, not a parse-only
/// check.
#[test]
fn an_id_selector_composes_with_hover() {
    let mut doc = RinchDocument::new();
    doc.load_css("#a:hover { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();
    let a = div(&mut doc, body, &[("id", "a")]);

    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_bottom(&doc, a),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        margin_top(&doc, a),
        0.0,
        "not hovered yet, so the rule must not apply"
    );

    let mut changed = false;
    doc.update_hover(Some(a.0), &mut changed);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, a),
        11.0,
        "#a:hover must apply once the node is hovered"
    );

    doc.update_hover(None, &mut changed);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, a),
        0.0,
        "…and stop applying when it is not"
    );
}

/// An id written, rewritten and removed at runtime. Each step restyles, so the
/// stored atom has to track every write.
///
/// Kills three mutants, though not all three through the margin. Each figure
/// below is measured against that mutant, not predicted:
///
/// - **atom never stored** — step 1 reads 0.
/// - **atom not updated on rewrite** — step 2 reads **0**, not the old 11. A
///   stale atom does not keep the old rule: it sends the bucket lookup to
///   `#a`, where `has_id` reads the *attribute* and refuses on the new value,
///   while `#b` is never offered at all. The node ends up with neither rule.
/// - **atom not cleared on remove** — step 3's margin is a *fixed point* and
///   passes. The store assertion that follows it is what kills that one; the
///   comment there explains why.
#[test]
fn an_id_set_changed_and_removed_at_runtime_tracks_the_rules() {
    let mut doc = RinchDocument::new();
    doc.load_css("#a { margin-top: 11px; } #b { margin-top: 23px; } div { margin-bottom: 17px; }");
    let body = doc.body();
    let n = div(&mut doc, body, &[]);

    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_bottom(&doc, n),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(margin_top(&doc, n), 0.0, "no id yet");

    doc.set_attribute(n, "id", "a");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, n),
        11.0,
        "an id set after mount must start matching #a"
    );

    doc.set_attribute(n, "id", "b");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, n),
        23.0,
        "an id rewritten at runtime must move to #b — a stored atom written \
         once and never updated leaves this at 0, not 11 (measured): the stale \
         atom sends the bucket lookup to `#a`, which `has_id` then refuses on \
         the new attribute value, while `#b` is never offered at all"
    );

    doc.remove_attribute(n, "id");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, n),
        0.0,
        "an id removed at runtime must stop matching"
    );
    // Measured, not assumed: a stale atom does NOT change the margin above.
    // The bucket is only a pre-filter — `has_id` reads the `id` *attribute*, so
    // an element left in the `#b` bucket after the attribute is gone is offered
    // the rule and then correctly refused by the predicate. The margin
    // assertion therefore sits on a fixed point for the "not cleared on remove"
    // mutant, and this is the assertion that kills it: the store itself.
    assert!(
        doc.tree.get(n.0).unwrap().id_atom().is_none(),
        "the interned id must be cleared on remove — leaving it set keeps the \
         node in a bucket it no longer belongs to and puts a stale hash in the \
         ancestor bloom filter"
    );
}

/// Specificity: an id beats a class beats a tag, in source order that would
/// give the opposite answer if specificity were ignored.
///
/// Sampled **off** the fixed point: the id rule is written *first*, so a
/// cascade that ordered by document position alone would answer `#0000ff`.
#[test]
fn an_id_selector_outranks_a_class_and_a_tag() {
    let mut doc = RinchDocument::new();
    doc.load_css("#a { color: #ff0000; } .c { color: #0000ff; } div { color: #00ff00; margin-bottom: 17px; }");
    let body = doc.body();
    let all = div(&mut doc, body, &[("id", "a"), ("class", "c")]);
    let cls = div(&mut doc, body, &[("class", "c")]);
    let bare = div(&mut doc, body, &[]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, all),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        (hex(&doc, all), hex(&doc, cls), hex(&doc, bare)),
        (
            "#ff0000".to_string(),
            "#0000ff".to_string(),
            "#00ff00".to_string()
        ),
        "id > class > tag, even though the id rule is written first"
    );
}

/// Two elements carrying the same id. Invalid HTML, entirely constructible,
/// and the bucket is a hash of the id rather than a unique index — so both
/// must match, as they do in a browser.
#[test]
fn two_elements_sharing_an_id_both_match() {
    let mut doc = RinchDocument::new();
    doc.load_css("#dup { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();
    let first = div(&mut doc, body, &[("id", "dup")]);
    let second = div(&mut doc, body, &[("id", "dup")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, first),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        (margin_top(&doc, first), margin_top(&doc, second)),
        (11.0, 11.0),
        "an id is a hash key, not a unique index"
    );
}

/// The id comes out of `Element::Html` parsing too. That path funnels through
/// `DomDocument::set_attribute` like the `rsx!` `id:` attribute does, and this
/// fixture is what says so — if HTML parsing ever grew a direct write into
/// `Node::attributes`, the stored atom would miss it and this would go red.
#[test]
fn an_id_from_parsed_html_matches() {
    let mut doc = RinchDocument::new();
    doc.load_css("#fromhtml { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();
    doc.set_inner_html(body, r#"<div id="fromhtml"></div><div></div>"#);

    doc.resolve_layout(VW, VH);

    let kids: Vec<_> = doc.tree.get(body.0).unwrap().children.clone();
    let with_id = NodeId(kids[0]);
    let without = NodeId(kids[1]);

    assert_eq!(
        margin_bottom(&doc, with_id),
        17.0,
        "positive control: the tag rule landed on the parsed element"
    );
    assert_eq!(
        (margin_top(&doc, with_id), margin_top(&doc, without)),
        (11.0, 0.0),
        "an id written by the HTML parser reaches the same store"
    );
}

/// Case sensitivity: rinch runs Stylo in `QuirksMode::NoQuirks`
/// (`stylo_impl.rs`'s `quirks_mode`), where ids are case-**sensitive**, and
/// the id bucket is a plain hash map there. `#A` must not match `id="a"`.
///
/// Sampled off the fixed point in the other direction too: a lowercasing store
/// would make both of these match.
#[test]
fn an_id_is_case_sensitive_outside_quirks_mode() {
    let mut doc = RinchDocument::new();
    doc.load_css("#CamelId { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();
    let exact = div(&mut doc, body, &[("id", "CamelId")]);
    let lower = div(&mut doc, body, &[("id", "camelid")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, exact),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        (margin_top(&doc, exact), margin_top(&doc, lower)),
        (11.0, 0.0),
        "NoQuirks: an id matches case-sensitively"
    );
}
