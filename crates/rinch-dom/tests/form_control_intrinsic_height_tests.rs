//! A text-entry form control is as tall as its own lines (#297).
//!
//! `<input>` and `<textarea>` keep their value in an attribute, so they are
//! childless however much text they show and nothing in the box tree gives them
//! a height. A browser sizes them from their own metrics — one line box for an
//! `<input>`, `rows` for a `<textarea>` — with the padding and border on top.
//! rinch answers that through a Taffy measure (`NodeContext::FormControl`,
//! `form_control.rs`). Before it, an inline `<input>` was as tall as its padding
//! and border and nothing more, and a blockified one only reached a line because
//! every childless block was floored at one (#296) — a border-box floor, so
//! padding and border ate the line it guaranteed.
//!
//! Every number is measured in Chrome 153 on the same markup, with
//! `* { box-sizing: border-box; margin: 0 }` to match rinch's global
//! `border-box`, and `line-height` declared so no glyph metric enters it.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A 10px-font, 20px-line field with 6px/10px padding and a 1px border: a
/// content box of one line (20) plus 12 + 2 — Chrome 153: 34px, in every
/// formatting context below.
const PADDED: &str =
    "font-size: 10px; line-height: 20px; padding: 6px 10px; border: 1px solid black";
/// The same field with no padding or border: the content box alone.
const BARE: &str = "font-size: 10px; line-height: 20px; padding: 0; border: 0";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn container(doc: &mut RinchDocument, style: &str) -> NodeId {
    let body = doc.body();
    el(
        doc,
        body,
        "div",
        &format!("width: 300px; font-size: 10px; line-height: 20px; {style}"),
    )
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// Chrome 153: an inline `<input>` (the UA sheet's `inline-block`) is 34px with
/// the padded style and 20px bare. rinch gave 14 and 0: the padding and border,
/// and no line at all, because an atomic inline was never floored.
#[test]
fn an_inline_input_is_one_line_plus_its_padding_and_border() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let padded = el(&mut doc, c, "input", &format!("{PADDED}; width: 100px"));
    let c2 = container(&mut doc, "");
    let bare = el(&mut doc, c2, "input", &format!("{BARE}; width: 100px"));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, padded), 34.0, "Chrome 153: 34");
    assert_eq!(height(&doc, bare), 20.0, "Chrome 153: 20");
    // The line box around it grows with it, as in Chrome (34).
    assert_eq!(height(&doc, c), 34.0, "Chrome 153: the line box is 34");
}

/// #296 item 1: a blockified `<input>` — here a flex item, and here
/// `display: block` — is 34px in Chrome 153. rinch gave 20: the line floor was a
/// border-box `min-height`, so the padding and border ate 14px of the line.
#[test]
fn a_blockified_padded_input_keeps_its_whole_line() {
    let mut doc = RinchDocument::new();
    let row = container(&mut doc, "display: flex");
    let item = el(&mut doc, row, "input", &format!("{PADDED}; flex: 1"));
    let c = container(&mut doc, "");
    let block = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; {PADDED}; width: 100px"),
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, item), 34.0, "Chrome 153: 34");
    assert_eq!(height(&doc, block), 34.0, "Chrome 153: 34");
}

/// A `min-height` is a floor under the content height, not a replacement for
/// it. Chrome 153: `min-height: 0` leaves a block input at 34, `min-height:
/// 50px` makes it 50.
#[test]
fn an_author_min_height_is_a_floor_under_the_control_not_over_it() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let zero = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; min-height: 0; {PADDED}; width: 100px"),
    );
    let fifty = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; min-height: 50px; {PADDED}; width: 100px"),
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, zero), 34.0, "Chrome 153: 34");
    assert_eq!(height(&doc, fifty), 50.0, "Chrome 153: 50");
}

/// Why the content height is a *measure* and not a `min-height` rinch writes:
/// in a 10px column, a control with `min-height: 0` shrinks to the column
/// (Chrome 153: 10) while one with `min-height: auto` keeps its content height
/// as its automatic minimum (Chrome 153: 20). A written `min-height` answers 20
/// for both, which is what rinch did — for the `<input>` through the line floor
/// and for the `<textarea>` through its own `rows` `min-height`.
#[test]
fn a_column_flex_item_with_min_height_zero_shrinks_below_its_line() {
    let mut doc = RinchDocument::new();
    let col = container(
        &mut doc,
        "display: flex; flex-direction: column; height: 10px",
    );
    let shrink = el(&mut doc, col, "input", &format!("min-height: 0; {BARE}"));
    let col2 = container(
        &mut doc,
        "display: flex; flex-direction: column; height: 10px",
    );
    let keep = el(&mut doc, col2, "input", BARE);
    let col3 = container(
        &mut doc,
        "display: flex; flex-direction: column; height: 10px",
    );
    let ta = el(
        &mut doc,
        col3,
        "textarea",
        &format!("min-height: 0; {BARE}"),
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, shrink), 10.0, "Chrome 153: 10");
    assert_eq!(height(&doc, keep), 20.0, "Chrome 153: 20");
    assert_eq!(height(&doc, ta), 10.0, "Chrome 153: 10");
}

/// And a `max-height` caps it. Chrome 153: a block input under `max-height:
/// 10px` is 10, a two-row textarea under `max-height: 30px` is 30. The
/// `<textarea>`'s old `rows` `min-height` beat any `max-height` (CSS resolves
/// `min` over `max`), so it stayed 40.
#[test]
fn a_max_height_caps_the_control() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let input = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; max-height: 10px; {BARE}; width: 100px"),
    );
    let ta = el(
        &mut doc,
        c,
        "textarea",
        &format!("display: block; max-height: 30px; {BARE}"),
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, input), 10.0, "Chrome 153: 10");
    assert_eq!(height(&doc, ta), 30.0, "Chrome 153: 30");
}

/// `rows` lines, 2 when `rows` is absent, below 1 or unparsable. Chrome 153:
/// `rows=3` padded is 3 × 20 + 14 = 74; `rows="2.5"` is 40 (2 rows);
/// `rows="0"` and `rows="0.5"` are 40 (the default). 3 and 2.5 are off the
/// default on purpose: a fixture at `rows=2` cannot tell a parsed `rows` from
/// the default. `rows` is read as a float, not by HTML's integer rules, so
/// `"3abc"` and `"1e1"` still disagree with Chrome — #1153.
#[test]
fn a_textarea_is_rows_lines_tall() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let three = el(
        &mut doc,
        c,
        "textarea",
        &format!("display: block; {PADDED}"),
    );
    doc.set_attribute(three, "rows", "3");
    let fractional = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    doc.set_attribute(fractional, "rows", "2.5");
    let zero = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    doc.set_attribute(zero, "rows", "0");
    let half = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    doc.set_attribute(half, "rows", "0.5");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, half), 40.0, "Chrome 153: 40");
    assert_eq!(height(&doc, three), 74.0, "Chrome 153: 74");
    assert_eq!(height(&doc, fractional), 40.0, "Chrome 153: 40");
    assert_eq!(height(&doc, zero), 40.0, "Chrome 153: 40");
}

/// The line height is not a Taffy property, so a `line-height` or `font-size`
/// change moves no Taffy style — the measure has to be re-synced by the
/// cascade, or the control keeps the height it was first laid out at.
/// Chrome 153: `font-size: 15px; line-height: 2` is 30. And a `rows` change
/// after the first layout re-sizes the textarea.
#[test]
fn a_restyle_or_a_rows_change_re_measures_the_control() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let input = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; {BARE}; width: 100px"),
    );
    let ta = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    // An inline one is an atomic inline, sized by its own compute outside the
    // root's (#661), so the re-measure has to reach that sizer as well.
    let c2 = container(&mut doc, "");
    let inline = el(&mut doc, c2, "input", &format!("{BARE}; width: 100px"));
    // A `rows` change moves no style at all, so nothing but the measure's own
    // re-sync tells that sizer the box changed.
    let c3 = container(&mut doc, "");
    let inline_ta = el(&mut doc, c3, "textarea", BARE);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, inline_ta), 40.0);
    assert_eq!(height(&doc, input), 20.0);
    assert_eq!(height(&doc, ta), 40.0);
    assert_eq!(height(&doc, inline), 20.0);

    doc.set_attribute(
        input,
        "style",
        "display: block; font-size: 15px; line-height: 2; padding: 0; border: 0; width: 100px",
    );
    doc.set_attribute(ta, "rows", "5");
    doc.set_style(inline, "line-height", "30px");
    doc.set_attribute(inline_ta, "rows", "3");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, inline_ta), 60.0, "3 rows of 20px");
    assert_eq!(height(&doc, input), 30.0, "Chrome 153: 30");
    assert_eq!(height(&doc, ta), 100.0, "5 rows of 20px");
    assert_eq!(height(&doc, inline), 30.0, "a 30px line");
}

/// A `font-size` transition frame writes `computed_style` directly and rebuilds
/// the Taffy style in `tick_transitions` — the cascade never sees it — so the
/// tick has to re-sync the measure too. `line-height: 2` makes the line follow
/// the font size.
#[test]
fn a_transition_tick_re_measures_the_control() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let input = el(
        &mut doc,
        c,
        "input",
        "display: block; font-size: 10px; line-height: 2; padding: 0; border: 0; width: 100px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, input), 20.0);

    doc.tree.nodes[input.0].computed_style.font_size = 17.0;
    doc.tree.dirty_nodes.insert(input.0);
    doc.tick_transitions();
    doc.tree.layout_dirty = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, input), 34.0, "2 × 17px");
}

/// The animation tick's twin of the transition fixture.
#[test]
fn an_animation_tick_re_measures_the_control() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let input = el(
        &mut doc,
        c,
        "input",
        "display: block; font-size: 10px; line-height: 2; padding: 0; border: 0; width: 100px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, input), 20.0);

    doc.tree.nodes[input.0].computed_style.font_size = 17.0;
    doc.tree.dirty_nodes.insert(input.0);
    doc.tick_animations();
    doc.tree.layout_dirty = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, input), 34.0, "2 × 17px");
}

/// `type=number` is a text field too (Chrome 153: 20), and a `type` the text
/// engine does not edit is not measured by this rule at all. (Chrome gives a
/// checkbox a 13x13 box of its own, which rinch does not model; the assertion
/// is on the measure context, not on a height, so it pins no wrong number.)
#[test]
fn the_text_like_types_are_measured_and_a_checkbox_is_not() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let number = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; {BARE}; width: 100px"),
    );
    doc.set_attribute(number, "type", "number");
    let text = el(
        &mut doc,
        c,
        "input",
        &format!("display: block; {BARE}; width: 100px"),
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, number), 20.0, "Chrome 153: 20");
    assert_eq!(height(&doc, text), 20.0);
    let measured = |doc: &RinchDocument, id: NodeId| {
        let t = doc.tree.get(id.0).unwrap().taffy_id.unwrap();
        matches!(
            doc.tree.taffy.get_node_context(t),
            Some(rinch_dom::node::NodeContext::FormControl { .. })
        )
    };
    assert!(measured(&doc, text));

    // A text field that becomes a checkbox gives its measure back — which
    // needs the `type` write to restyle the node, since no selector has to
    // depend on `type` for Stylo to think it changed anything.
    doc.set_attribute(text, "type", "checkbox");
    doc.resolve_layout(800.0, 600.0);
    assert!(!measured(&doc, text), "a checkbox is not a text control");
}

/// A `<textarea>` whose value arrives as a text **child** — `textarea { "hi" }`
/// in rsx, `set_text_content`, a parsed `<textarea>hi</textarea>` — is still
/// `rows` lines (Chrome 153: 40 for 2 rows, 60 for 3), and stays so across a
/// restyle. Such a textarea is an IFC root; its measure used to trade places
/// with the form-control measure, so it was one line (20) after the structural
/// pass and `rows` lines after the next restyle — it grew when clicked into
/// (PR #1152 review, F1). The restyle is at a new viewport so it is laid out
/// again.
#[test]
fn a_textarea_with_a_text_child_is_rows_lines_across_a_restyle() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let two = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    doc.set_text_content(two, "abc");
    let three = el(&mut doc, c, "textarea", &format!("display: block; {BARE}"));
    doc.set_attribute(three, "rows", "3");
    doc.set_text_content(three, "abc");
    // And one that stays inline, sized by the atomic-inline compute.
    let c2 = container(&mut doc, "");
    let inline = el(&mut doc, c2, "textarea", BARE);
    doc.set_text_content(inline, "abc");
    doc.resolve_layout(800.0, 600.0);
    let first = (height(&doc, two), height(&doc, three), height(&doc, inline));
    for t in [two, three, inline] {
        doc.set_style(t, "color", "red");
    }
    doc.resolve_layout(801.0, 600.0);
    let second = (height(&doc, two), height(&doc, three), height(&doc, inline));
    assert_eq!(first, (40.0, 60.0, 40.0), "Chrome 153: 40 / 60 / 40");
    assert_eq!(second, (40.0, 60.0, 40.0), "and the same after a restyle");

    // A `rows` change reaches it too, though its measure context is the IFC's.
    doc.set_attribute(three, "rows", "4");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(height(&doc, three), 80.0, "4 rows of 20px");
}

/// Chrome 153: a `<br>` that is a flex item, or `display: block`, is one line
/// (20). The removed empty-block floor gave it that; the measure does now
/// (PR #1152 review, F2).
#[test]
fn a_blockified_br_is_one_line() {
    let mut doc = RinchDocument::new();
    let flex = container(&mut doc, "display: flex");
    el(&mut doc, flex, "br", "");
    let block = container(&mut doc, "");
    el(&mut doc, block, "br", "display: block");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, flex), 20.0, "Chrome 153: 20");
    assert_eq!(height(&doc, block), 20.0, "Chrome 153: 20");
}

/// Every `<input>` type is a one-line field except the six with a size of
/// their own. Chrome 153 at `line-height: 20px`: an invalid `type="foo"` is 20
/// (HTML's invalid-value default is Text), `TEXT` is 20 (the attribute is
/// case-insensitive), the button types and `file` are 20, and the date/time
/// family 22 — rinch gives those the 20px line, leaving Chrome's 2px of
/// internal padding unmodelled (PR #1152 review, F3).
#[test]
fn every_input_type_but_six_is_a_line() {
    let mut doc = RinchDocument::new();
    let c = container(&mut doc, "");
    let mut ids = Vec::new();
    for t in [
        "foo", "TEXT", "submit", "button", "reset", "file", "date", "time",
    ] {
        let i = el(
            &mut doc,
            c,
            "input",
            &format!("display: block; {BARE}; width: 100px"),
        );
        doc.set_attribute(i, "type", t);
        ids.push((t, i));
    }
    let mut none = Vec::new();
    for t in ["checkbox", "Radio", "range", "color", "image"] {
        let i = el(
            &mut doc,
            c,
            "input",
            &format!("display: block; {BARE}; width: 100px"),
        );
        doc.set_attribute(i, "type", t);
        none.push((t, i));
    }
    doc.resolve_layout(800.0, 600.0);
    for (t, i) in ids {
        let h = height(&doc, i);
        if matches!(t, "date" | "time") {
            // Chrome 153: 22. rinch gives the one line and not Chrome's 2px of
            // internal padding; the pin is that it is a line, not 0.
            assert!((20.0..=22.0).contains(&h), "type={t}: one line, got {h}");
        } else {
            assert_eq!(h, 20.0, "type={t}: Chrome 153: 20");
        }
    }
    // Chrome sizes these itself (13, 16, 27, the image's); rinch models none of
    // it, and must not hand them a text line instead.
    for (t, i) in none {
        assert_eq!(height(&doc, i), 0.0, "type={t}: not a text line");
    }
}

/// A `type` change alone re-sizes the control — same viewport, no other style
/// change — so the cascade's re-sync has to owe a layout, not just rewrite the
/// context (PR #1152 review, F5; it asserted the context before).
#[test]
fn a_type_change_alone_resizes_the_control() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let i = el(
        &mut doc,
        body,
        "input",
        &format!("display: block; {BARE}; width: 100px"),
    );
    doc.set_attribute(i, "type", "checkbox");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, i), 0.0);
    doc.set_attribute(i, "type", "text");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, i), 20.0, "checkbox -> text");
    doc.set_attribute(i, "type", "checkbox");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, i), 0.0, "text -> checkbox");
}
