//! #681 — a dozen state pseudo-classes silently matched nothing, because
//! `RinchNode::match_non_ts_pseudo_class` ended in a catch-all `_ => false`.
//!
//! This file pins the subset that is CONTAINED with the state/attributes
//! rinch already has: `:required`/`:optional`, `:read-only`/`:read-write`,
//! `:placeholder-shown`, `:defined` and `:lang()`. It is deliberately
//! narrower than the full CSS definition for several of these — see
//! `Node::tag_supports_required` / `Node::tag_is_readonly_capable`'s own
//! docs for the exact scope — so #681 stays open for the rest
//! (`:focus-within` — attempted and reverted, see `stylo_impl.rs`'s comment
//! at the `_ => false` catch-all for why — `:indeterminate`, `:valid`/
//! `:invalid`, `:in-range`/`:out-of-range`, `:target`, `:fullscreen`,
//! `:modal`, `:popover-open`, `:default`, `:autofill`, `:user-valid`,
//! `:user-invalid`), which still fall through the same `_ => false`.
//!
//! `* { margin-bottom: 17px }` is the positive control throughout, matching
//! the issue's own measured table.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn margin_top(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree.get(id).unwrap().computed_style.margin_top.to_px()
}

fn margin_bottom(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree
        .get(id)
        .unwrap()
        .computed_style
        .margin_bottom
        .to_px()
}

/// `:required` / `:optional` on an `<input>`.
#[test]
fn required_and_optional_match_an_input() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "input:required { margin-top: 11px; } input:optional { margin-top: 23px; } \
         * { margin-bottom: 17px; }",
    );
    let body = doc.body();

    let req = doc.create_element("input");
    doc.set_attribute(req, "required", "");
    doc.append_child(body, req);

    let opt = doc.create_element("input");
    doc.append_child(body, opt);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (margin_bottom(&doc, req.0), margin_bottom(&doc, opt.0)),
        (17.0, 17.0),
        "positive control"
    );
    assert_eq!(margin_top(&doc, req.0), 11.0, "#681: :required must match");
    assert_eq!(margin_top(&doc, opt.0), 23.0, "#681: :optional must match");

    // Runtime flip: adding/removing `required` must restyle (`required` is
    // in `PSEUDO_CLASS_ATTRIBUTES`-adjacent machinery — fed through
    // `element_state`'s `ElementState::REQUIRED`/`OPTIONAL_` bits, which
    // Stylo's own invalidation maps see).
    doc.remove_attribute(req, "required");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, req.0),
        23.0,
        "removing `required` at runtime must flip :required -> :optional"
    );
}

/// `:read-only` / `:read-write` on an `<input>`.
#[test]
fn read_only_and_read_write_match_an_input() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "input:read-only { margin-top: 11px; } input:read-write { margin-top: 23px; } \
         * { margin-bottom: 17px; }",
    );
    let body = doc.body();

    let ro = doc.create_element("input");
    doc.set_attribute(ro, "readonly", "");
    doc.append_child(body, ro);

    let rw = doc.create_element("input");
    doc.append_child(body, rw);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (margin_bottom(&doc, ro.0), margin_bottom(&doc, rw.0)),
        (17.0, 17.0),
        "positive control"
    );
    assert_eq!(margin_top(&doc, ro.0), 11.0, "#681: :read-only must match");
    assert_eq!(margin_top(&doc, rw.0), 23.0, "#681: :read-write must match");

    // Runtime flip.
    doc.set_attribute(rw, "readonly", "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, rw.0),
        11.0,
        "adding `readonly` at runtime must flip :read-write -> :read-only"
    );
}

/// `:placeholder-shown`: present only while the field is empty.
#[test]
fn placeholder_shown_tracks_the_live_value() {
    let mut doc = RinchDocument::new();
    doc.load_css("input:placeholder-shown { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let n = doc.create_element("input");
    doc.set_attribute(n, "placeholder", "hi");
    doc.append_child(body, n);

    doc.resolve_layout(VW, VH);
    assert_eq!(margin_bottom(&doc, n.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, n.0),
        11.0,
        "#681: an empty field with a placeholder must match :placeholder-shown"
    );

    doc.set_attribute(n, "value", "typed");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, n.0),
        0.0,
        "a non-empty value must stop :placeholder-shown from matching"
    );

    doc.set_attribute(n, "value", "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, n.0),
        11.0,
        "clearing the value must bring :placeholder-shown back"
    );
}

/// `:defined`: every element rinch can build is defined (no custom-element
/// registry, so nothing is ever "unresolved").
#[test]
fn defined_matches_every_element() {
    let mut doc = RinchDocument::new();
    doc.load_css("div:defined { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();
    let n = doc.create_element("div");
    doc.append_child(body, n);

    doc.resolve_layout(VW, VH);
    assert_eq!(margin_bottom(&doc, n.0), 17.0, "positive control");
    assert_eq!(margin_top(&doc, n.0), 11.0, "#681: :defined must match");
}

/// `:lang()`: resolved from the nearest ancestor's `lang` attribute, with
/// the standard dash-prefix match (`en` matches content tagged `en-US` but
/// not `english`).
#[test]
fn lang_resolves_through_the_nearest_ancestor() {
    let mut doc = RinchDocument::new();
    doc.load_css(":lang(en) { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "lang", "en");
    doc.append_child(body, wrapper);
    let p = doc.create_element("p");
    doc.append_child(wrapper, p);

    let outside = doc.create_element("div");
    doc.append_child(body, outside);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (margin_bottom(&doc, wrapper.0), margin_bottom(&doc, p.0)),
        (17.0, 17.0),
        "positive control"
    );
    assert_eq!(
        margin_top(&doc, wrapper.0),
        11.0,
        "#681: :lang(en) must match the element carrying lang=\"en\""
    );
    assert_eq!(
        margin_top(&doc, p.0),
        11.0,
        "…and an element inheriting it from an ancestor"
    );
    assert_eq!(
        margin_top(&doc, outside.0),
        0.0,
        "an element with no lang anywhere in its chain must not match"
    );
}

/// `:lang()`'s dash-prefix rule, and the runtime invalidation it needs: a
/// `lang` write must restyle the subtree below it (`PSEUDO_CLASS_ATTRIBUTES`).
#[test]
fn lang_dash_prefix_match_and_runtime_invalidation() {
    let mut doc = RinchDocument::new();
    doc.load_css(":lang(en) { margin-top: 11px; } * { margin-bottom: 17px; }");
    let body = doc.body();

    let n = doc.create_element("div");
    doc.set_attribute(n, "lang", "en-US");
    doc.append_child(body, n);

    let other = doc.create_element("div");
    doc.set_attribute(other, "lang", "english");
    doc.append_child(body, other);

    doc.resolve_layout(VW, VH);
    assert_eq!(
        (margin_bottom(&doc, n.0), margin_bottom(&doc, other.0)),
        (17.0, 17.0),
        "positive control"
    );
    assert_eq!(
        margin_top(&doc, n.0),
        11.0,
        ":lang(en) must match lang=\"en-US\" (dash-prefix rule)"
    );
    assert_eq!(
        margin_top(&doc, other.0),
        0.0,
        "…but not lang=\"english\" — \"en\" is not a dash-separated prefix of it"
    );

    doc.set_attribute(other, "lang", "en");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        margin_top(&doc, other.0),
        11.0,
        "a `lang` write at runtime must restyle and pick up the new match"
    );
}
