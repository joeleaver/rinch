//! #688 — an HTML attribute name is ASCII case-insensitive; an SVG one is not.
//!
//! The CSS-property-name half of the same class is #711
//! (`inline_style_case_tests.rs`). This is the attribute half, and it is
//! harder in one specific way: the property rule is unconditional, where the
//! attribute rule depends on which content the element sits in. HTML folds,
//! SVG does not — `viewBox` and `preserveAspectRatio` are read by exact
//! spelling in `paint/svg.rs`, and `gradientUnits`, `stdDeviation`,
//! `markerWidth` and `startOffset` are the same class on SVG *children*.
//!
//! Every answer asserted here was measured in Chrome 150 first, on the same
//! markup, via a `data:` URL plus `getComputedStyle` / `getAttribute`.
//!
//! The fold lives at `RinchDocument::set_attribute` (and its `remove`/`get`
//! twins), keyed on the element's **tag**, because at the moment an attribute
//! is written the element usually has no parent: both writers that build a
//! tree — the `rsx!` codegen (`dom_codegen/html.rs`) and
//! `create_node_from_parsed` — set every attribute on a fresh element and
//! append it to its parent afterwards. A parent walk would answer "not SVG"
//! for every element in a freshly built `<svg>`. See `dom_impl`'s
//! `is_svg_content_tag` for the list and its ambiguous-tag caveat.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

const RED: &str = "#ff0000";
const GREEN: &str = "#00ff00";

fn hex(doc: &RinchDocument, n: NodeId) -> String {
    let c = doc.tree.get(n.0).unwrap().computed_style.color;
    let c = c.expect("color should resolve");
    let rgba = c.to_rgba8();
    format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
}

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

/// A `<div>` under `<body>` carrying the given attributes, written in the
/// order given — which is the order `rsx!` writes them in, i.e. before the
/// element is appended.
fn div(doc: &mut RinchDocument, attrs: &[(&str, &str)]) -> NodeId {
    let body = doc.body();
    let n = doc.create_element("div");
    for (k, v) in attrs {
        doc.set_attribute(n, k, v);
    }
    doc.append_child(body, n);
    n
}

/// The keys actually stored on a node, sorted.
fn keys(doc: &RinchDocument, n: NodeId) -> Vec<String> {
    let mut k: Vec<String> = doc
        .tree
        .get(n.0)
        .unwrap()
        .attributes
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

// ── The issue's own case ────────────────────────────────────────────────────

/// `#up { … }` against `<div ID="up">`, with a lowercase twin as the positive
/// control. Chrome 150: both divs get the margin.
///
/// This is the assertion the issue was filed on. It also exercises the
/// interned-`id` path (#685): `Node::write_attribute` keys the atom on the
/// literal name `"id"`, so an unfolded `ID` never reaches Stylo's id bucket
/// at all.
#[test]
fn an_uppercase_id_attribute_matches_an_id_selector() {
    let mut doc = RinchDocument::new();
    doc.load_css("#up { margin-top: 11px; } div { margin-bottom: 17px; }");

    let upper = div(&mut doc, &[("ID", "up")]);
    let lower = div(&mut doc, &[("id", "up")]);
    let bare = div(&mut doc, &[]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (
            margin_bottom(&doc, upper),
            margin_bottom(&doc, lower),
            margin_bottom(&doc, bare)
        ),
        (17.0, 17.0, 17.0),
        "positive control: the tag rule must land on all three, or the \
         stylesheet never parsed"
    );
    assert_eq!(
        margin_top(&doc, lower),
        11.0,
        "positive control: the lowercase twin matches (it always did)"
    );
    assert_eq!(
        margin_top(&doc, bare),
        0.0,
        "negative control: nothing gives the bare div a top margin"
    );
    assert_eq!(
        margin_top(&doc, upper),
        11.0,
        "#688: `ID=` is the same attribute as `id=` in HTML content"
    );
    assert_eq!(
        doc.tree
            .get(upper.0)
            .unwrap()
            .id_atom()
            .map(|a| a.to_string()),
        Some("up".to_string()),
        "…and the interned id (#685) is written from the folded name, not the \
         literal one"
    );
}

/// `STYLE="color: red"` must be an inline style.
///
/// This is the one the issue calls out by name: `RinchDocument::set_attribute`
/// branches on `name == "style"` to fill `style_attribute_cache`, so an
/// attribute spelled `STYLE` used to be stored and then silently not be a
/// style. Chrome 150 computes red for both spellings.
///
/// Sampled off the fixed point: the stylesheet paints the element green, so
/// "the inline style did nothing" and "the inline style won" are different
/// colours rather than both black.
#[test]
fn an_uppercase_style_attribute_is_an_inline_style() {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!("div {{ color: {GREEN}; margin-bottom: 17px; }}"));

    let upper = div(&mut doc, &[("STYLE", &format!("color: {RED}"))]);
    let lower = div(&mut doc, &[("style", &format!("color: {RED}"))]);
    let bare = div(&mut doc, &[]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, upper),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        (hex(&doc, lower), hex(&doc, bare)),
        (RED.to_string(), GREEN.to_string()),
        "positive/negative control: the lowercase spelling wins over the \
         stylesheet, and an element with no inline style does not"
    );
    assert_eq!(
        hex(&doc, upper),
        RED,
        "#688: `STYLE=` must reach the inline-style cache"
    );
    assert_eq!(
        keys(&doc, upper),
        vec!["style".to_string()],
        "…and be stored under the folded key, so a later `set_style` merges \
         into it rather than appending a second attribute"
    );
}

/// `CLASS="x"` against `.x { … }`. Chrome 150: matches.
///
/// `each_class` reads `attributes.get("class")` by literal key, so this is the
/// class half of the same single lookup the `id` half goes through.
#[test]
fn an_uppercase_class_attribute_matches_a_class_selector() {
    let mut doc = RinchDocument::new();
    doc.load_css(".x { margin-top: 11px; } div { margin-bottom: 17px; }");

    let upper = div(&mut doc, &[("CLASS", "x")]);
    let lower = div(&mut doc, &[("class", "x")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, upper),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        margin_top(&doc, lower),
        11.0,
        "positive control: the lowercase twin matches"
    );
    assert_eq!(margin_top(&doc, upper), 11.0, "#688: `CLASS=` is `class=`");
}

// ── The second consumer: Stylo's attribute-selector path ────────────────────

/// `[data-x="1"]` and `[DATA-X="1"]` are the same selector, and both match
/// `data-x="1"` and `DATA-X="1"`. Chrome 150: all four combinations match.
///
/// The selector half needs no change of its own. Stylo parses an attribute
/// selector into a `local_name` *and* a `local_name_lower`, and the
/// `selectors` crate picks between them on `is_html_element_in_html_document`,
/// which rinch answers `true` unconditionally — so the selector arrives at
/// `attr_matches` already lowercased. The store was the only half that was
/// wrong. This fixture is what says so: it goes red at HEAD on the *attribute*
/// spellings only, and it is what would catch a future change to
/// `is_html_element_in_html_document` that broke the selector half.
#[test]
fn an_attribute_selector_folds_at_both_ends() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "[data-x=\"1\"] { margin-top: 11px; } \
         [DATA-Y] { margin-left: 23px; } \
         div { margin-bottom: 17px; }",
    );

    let lower = div(&mut doc, &[("data-x", "1"), ("data-y", "")]);
    let upper = div(&mut doc, &[("DATA-X", "1"), ("DATA-Y", "")]);
    let bare = div(&mut doc, &[]);

    doc.resolve_layout(VW, VH);

    let margin_left = |n: NodeId| {
        doc.tree
            .get(n.0)
            .unwrap()
            .computed_style
            .margin_left
            .to_px()
    };

    assert_eq!(
        margin_bottom(&doc, upper),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        (margin_top(&doc, bare), margin_left(bare)),
        (0.0, 0.0),
        "negative control: neither attribute rule reaches an element with no \
         attributes"
    );
    assert_eq!(
        (margin_top(&doc, lower), margin_left(lower)),
        (11.0, 23.0),
        "positive control: a lowercase attribute matches both a lowercase and \
         an UPPERCASE attribute selector — the selector end already folded"
    );
    assert_eq!(
        (margin_top(&doc, upper), margin_left(upper)),
        (11.0, 23.0),
        "#688: and so does an UPPERCASE attribute"
    );
}

// ── The readers ─────────────────────────────────────────────────────────────

/// `get_attribute` and `remove_attribute` fold too, so the three sides of the
/// same attribute agree. Chrome 150: after `setAttribute("ID", "a")`,
/// `getAttribute("id") === "a"`, `getAttribute("ID") === "a"`, and
/// `removeAttribute("ID")` clears it.
///
/// `remove_attribute` is not cosmetic here. `NodeHandle::write_attribute`
/// (#551) turns a falsey boolean attribute into a `remove_attribute` call with
/// the *caller's* spelling — so a writer that folded and a remover that did
/// not could never turn one off again.
#[test]
fn reading_and_removing_fold_the_name_too() {
    let mut doc = RinchDocument::new();
    let n = div(&mut doc, &[("ID", "a"), ("Data-Thing", "t")]);

    assert_eq!(
        keys(&doc, n),
        vec!["data-thing".to_string(), "id".to_string()],
        "both names are stored folded"
    );
    assert_eq!(doc.get_attribute(n, "id").as_deref(), Some("a"));
    assert_eq!(
        doc.get_attribute(n, "ID").as_deref(),
        Some("a"),
        "#688: a read folds its own name, so either spelling finds it"
    );
    assert_eq!(doc.get_attribute(n, "data-thing").as_deref(), Some("t"));
    assert_eq!(doc.get_attribute(n, "DATA-THING").as_deref(), Some("t"));

    doc.remove_attribute(n, "Id");
    assert_eq!(
        doc.get_attribute(n, "id"),
        None,
        "#688: a remove folds its own name"
    );
    assert!(
        doc.tree.get(n.0).unwrap().id_atom().is_none(),
        "…and clears the interned id with it (#685)"
    );
    assert_eq!(keys(&doc, n), vec!["data-thing".to_string()]);
}

/// The value is untouched. Attribute *names* are case-insensitive in HTML;
/// values are not, and an id is matched case-sensitively outside quirks mode
/// (`id_selector_tests::an_id_is_case_sensitive_outside_quirks_mode`).
///
/// Kills the mutant that lowercases the pair rather than the name.
#[test]
fn only_the_name_folds_never_the_value() {
    let mut doc = RinchDocument::new();
    doc.load_css("#CamelId { margin-top: 11px; } div { margin-bottom: 17px; }");

    let kept = div(&mut doc, &[("ID", "CamelId")]);
    let flattened = div(&mut doc, &[("ID", "camelid")]);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        margin_bottom(&doc, kept),
        17.0,
        "positive control: the tag rule landed"
    );
    assert_eq!(
        doc.get_attribute(kept, "id").as_deref(),
        Some("CamelId"),
        "the value keeps its case"
    );
    assert_eq!(
        (margin_top(&doc, kept), margin_top(&doc, flattened)),
        (11.0, 0.0),
        "an id still matches case-sensitively; only the *name* folded"
    );
}

/// The `Element::Html` writer. `create_node_from_parsed` funnels through
/// `DomDocument::set_attribute` exactly as `rsx!` does, and this is what says
/// so — and it is the writer whose ordering the tag-keyed design turns on,
/// since it too sets every attribute before appending the element.
#[test]
fn an_uppercase_attribute_from_parsed_html_folds() {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "#h {{ margin-top: 11px; }} .c {{ margin-left: 23px; }} div {{ color: {GREEN}; margin-bottom: 17px; }}"
    ));
    let body = doc.body();
    doc.set_inner_html(
        body,
        &format!(r#"<div ID="h" CLASS="c" STYLE="color: {RED}"></div><div></div>"#),
    );

    doc.resolve_layout(VW, VH);

    let kids: Vec<_> = doc.tree.get(body.0).unwrap().children.clone();
    let parsed = NodeId(kids[0]);
    let bare = NodeId(kids[1]);

    assert_eq!(
        margin_bottom(&doc, parsed),
        17.0,
        "positive control: the tag rule landed on the parsed element"
    );
    assert_eq!(
        (margin_top(&doc, bare), hex(&doc, bare)),
        (0.0, GREEN.to_string()),
        "negative control: the bare sibling gets neither"
    );
    assert_eq!(
        (
            margin_top(&doc, parsed),
            doc.tree
                .get(parsed.0)
                .unwrap()
                .computed_style
                .margin_left
                .to_px(),
            hex(&doc, parsed)
        ),
        (11.0, 23.0, RED.to_string()),
        "#688: id, class and style all fold on the parse path"
    );
}

// ── The counter-case: SVG is case-SENSITIVE ─────────────────────────────────

/// `viewBox` on `<svg>` must survive verbatim. This is the store-level half;
/// `svg_viewbox_still_scales_after_the_fold` below is the behavioural one.
///
/// Kills the blanket-fold mutant — lowercasing every attribute name — which is
/// the obvious wrong fix for this issue.
#[test]
fn svg_camelcase_attributes_are_not_folded() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let svg = doc.create_element("svg");
    doc.set_attribute(svg, "viewBox", "0 0 10 10");
    doc.set_attribute(svg, "preserveAspectRatio", "none");
    doc.append_child(body, svg);

    // The children are the half a tag-local `tag != "svg"` test would get
    // wrong, and the reason the fold is keyed on a list rather than one name.
    let grad = doc.create_element("linearGradient");
    doc.set_attribute(grad, "gradientUnits", "userSpaceOnUse");
    doc.append_child(svg, grad);

    let blur = doc.create_element("feGaussianBlur");
    doc.set_attribute(blur, "stdDeviation", "2");
    doc.append_child(svg, blur);

    let marker = doc.create_element("marker");
    doc.set_attribute(marker, "markerWidth", "6");
    doc.append_child(svg, marker);

    let tp = doc.create_element("textPath");
    doc.set_attribute(tp, "startOffset", "50%");
    doc.append_child(svg, tp);

    assert_eq!(
        keys(&doc, svg),
        vec!["preserveAspectRatio".to_string(), "viewBox".to_string()]
    );
    assert_eq!(keys(&doc, grad), vec!["gradientUnits".to_string()]);
    assert_eq!(keys(&doc, blur), vec!["stdDeviation".to_string()]);
    assert_eq!(keys(&doc, marker), vec!["markerWidth".to_string()]);
    assert_eq!(keys(&doc, tp), vec!["startOffset".to_string()]);

    assert_eq!(
        doc.get_attribute(svg, "viewBox").as_deref(),
        Some("0 0 10 10"),
        "a read of an SVG attribute does not fold either"
    );
    assert_eq!(
        doc.get_attribute(svg, "viewbox"),
        None,
        "…and the folded spelling is a different attribute, as in SVG"
    );
}

/// `<foreignObject>` re-enters HTML content, so a `<div>` inside one folds.
///
/// It falls out of the tag-keyed design for free — the descendant's own tag is
/// an HTML one — which is the design's answer to the case a parent walk would
/// have had to special-case. `foreignObject` itself stays SVG.
#[test]
fn html_inside_a_foreign_object_folds() {
    let mut doc = RinchDocument::new();
    doc.load_css("#fo { margin-top: 11px; } div { margin-bottom: 17px; }");
    let body = doc.body();

    let svg = doc.create_element("svg");
    doc.set_attribute(svg, "viewBox", "0 0 10 10");
    doc.append_child(body, svg);
    let fo = doc.create_element("foreignObject");
    doc.set_attribute(fo, "requiredExtensions", "x");
    doc.append_child(svg, fo);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "ID", "fo");
    doc.append_child(fo, inner);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        keys(&doc, fo),
        vec!["requiredExtensions".to_string()],
        "the foreignObject element is itself SVG content"
    );
    assert_eq!(
        keys(&doc, inner),
        vec!["id".to_string()],
        "#688: its HTML descendants are not"
    );
    assert_eq!(
        margin_top(&doc, inner),
        11.0,
        "…and the folded id reaches the id bucket"
    );
}

/// An element whose tag is not in either list — a custom element — is HTML
/// content and folds. Chrome 150: `<my-widget ID="w">` answers
/// `getAttribute("id")`.
#[test]
fn an_unknown_tag_is_html_content() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let n = doc.create_element("my-widget");
    doc.set_attribute(n, "ID", "w");
    doc.append_child(body, n);

    assert_eq!(keys(&doc, n), vec!["id".to_string()]);
}

/// The behavioural counter-case: a folded `viewBox` would silently fall back
/// to `paint/svg.rs`'s `0 0 24 24` default, which changes what is painted.
///
/// A 20×20 `<svg viewBox="0 0 10 10">` holding a `10×10` `<rect fill="red">`
/// covers the whole box, so its centre pixel is red. Under the default
/// viewBox the same rect scales to 20·10/24 ≈ 8.3px in the top-left corner and
/// the centre pixel is empty — i.e. this fixture is **off** the fixed point
/// that a same-size viewBox would sit on.
#[cfg(feature = "software-renderer")]
#[test]
fn svg_viewbox_still_scales_after_the_fold() {
    use peniko::Brush;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let svg = doc.create_element("svg");
    doc.set_attribute(svg, "viewBox", "0 0 10 10");
    doc.set_attribute(
        svg,
        "style",
        "display: block; width: 20px; height: 20px; margin: 0",
    );
    doc.append_child(body, svg);
    let rect = doc.create_element("rect");
    doc.set_attribute(rect, "width", "10");
    doc.set_attribute(rect, "height", "10");
    doc.set_attribute(rect, "fill", "red");
    doc.append_child(svg, rect);
    doc.resolve_layout(VW, VH);

    let layout = doc.tree.get(svg.0).unwrap().layout;
    assert_eq!(
        (layout.x, layout.y, layout.width, layout.height),
        (0.0, 0.0, 20.0, 20.0),
        "positive control: the svg sits at the page origin at its styled size"
    );

    let mut painter = TinySkiaPainter::new(40, 40);
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
    let idx = ((10 * painter.width() + 10) * 4) as usize;
    let px = &painter.pixels()[idx..idx + 4];

    assert_eq!(
        (px[0], px[1], px[2], px[3]),
        (255, 0, 0, 255),
        "the rect must still fill the 20×20 box — a `viewBox` folded to \
         `viewbox` falls back to `0 0 24 24` and leaves this pixel empty"
    );
}

// ── The boolean-attribute writer, which the fold makes reachable ─────────────

/// `write_attribute` (#551) maps a *falsey* value onto the **absence** of the
/// attribute, and it decides which attributes get that treatment by name. An
/// uppercase name used to miss `is_boolean_attribute` and fall through to the
/// literal writer, which wrote `CHECKED="false"`.
///
/// That was harmless only by accident: the unfolded key was invisible to every
/// reader. Folding the store (#688) makes it `checked="false"` — a *present*
/// boolean attribute, which `:checked` matches, exactly the #551 latch under a
/// different spelling. So the name half of `is_boolean_attribute` had to fold
/// too, and this is the fixture that says so end to end.
///
/// Both directions are asserted, because "present means on" is only evidence
/// about the writer if "absent means off" is checked beside it.
#[test]
fn a_falsey_boolean_attribute_written_in_uppercase_is_still_removed() {
    use rinch_core::dom::NodeHandle;
    use std::cell::RefCell;
    use std::rc::Rc;

    let doc = RinchDocument::new();
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(doc));
    let weak = Rc::downgrade(&doc);

    let (upper, lower) = {
        let mut d = doc.borrow_mut();
        let body = d.body();
        let upper = d.create_element("input");
        let lower = d.create_element("input");
        d.append_child(body, upper);
        d.append_child(body, lower);
        (upper, lower)
    };
    let upper = NodeHandle::new(upper, weak.clone());
    let lower = NodeHandle::new(lower, weak);

    upper.write_attribute("CHECKED", "true");
    lower.write_attribute("checked", "true");
    assert_eq!(
        (
            upper.get_attribute("checked"),
            lower.get_attribute("checked")
        ),
        (Some(String::new()), Some(String::new())),
        "positive control: a truthy value writes the bare presence form under \
         the folded key, in either spelling"
    );

    upper.write_attribute("CHECKED", "false");
    lower.write_attribute("checked", "false");
    assert_eq!(
        lower.get_attribute("checked"),
        None,
        "positive control: the lowercase spelling removes (it always did)"
    );
    assert_eq!(
        upper.get_attribute("checked"),
        None,
        "#688: and so does the uppercase one — left as `checked=\"false\"` it \
         would be a present boolean attribute, i.e. #551 again"
    );
}
