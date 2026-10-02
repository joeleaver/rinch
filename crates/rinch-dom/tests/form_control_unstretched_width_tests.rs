//! #1195: a `display: block` text control shrinks to its intrinsic width
//! instead of filling its container, and the form-control types #1177 left
//! at 0 (the date/time family, `file`, and the three buttons) get an
//! intrinsic width of their own.
//!
//! **Shrink-to-fit.** Chrome treats a text-entry control as "auto width fits
//! content" even at `display: block` (CSS 2.1 §10.3.4's replaced-element
//! rule). rinch's block algorithm stretched every `width: auto` child to its
//! container, text controls included, because nothing told Taffy's block
//! layout otherwise. `crate::replaced::is_unstretched_replaced` now answers
//! `true` for a line-sized control too (the same `item_is_replaced` flag an
//! `<img>` already carries), which is what keeps Taffy's block algorithm
//! from stretching it (`taffy::compute::block`: `is_table || is_replaced` ⇒
//! `known_dimensions = Size::NONE`, so the control's own measure — its
//! `size`/`cols` width — wins).
//!
//! **The other intrinsic widths.** `form_control.rs`'s
//! `picker_input_content_width` fits the date/time family and `file` to the
//! font size (a measured affine model — each paints an icon and a
//! locale/format string this crate does not shape); `button_label_content_width`
//! sizes `submit`/`reset`/`button` to their label text, shaped the way a
//! closed `<select>`'s label is. Every number below is measured in Chrome
//! 153 on Linux with this crate's bundled Inter loaded through `@font-face`
//! under the override name the fixtures register it with, `padding: 0;
//! border: 0`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

/// A control of `tag`/`attrs`, `display: block`, in an 800px body. The
/// bare style has no padding/border, so the control's border-box width is
/// its content-box width.
fn block_control(tag: &str, px: f32, attrs: &[(&str, &str)]) -> f32 {
    let mut doc = document();
    let body = doc.body();
    let c = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(c, k, v);
    }
    doc.set_attribute(
        c,
        "style",
        &format!("font: {px}px/20px ProbeFace; padding: 0; border: 0; display: block"),
    );
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    width(&doc, c)
}

/// A `display: block` text control keeps its `size`/`cols` width instead of
/// filling the 800px body (Chrome 153 at 16px Inter: 248 and 220 — the same
/// numbers `form_control_intrinsic_width_tests.rs` measures for the intrinsic
/// (inline) case).
#[test]
fn a_display_block_input_and_textarea_shrink_to_fit() {
    assert_eq!(block_control("input", 16.0, &[]), 248.0);
    assert_eq!(block_control("textarea", 16.0, &[]), 220.0);
}

/// `size`/`cols` still apply at `display: block` (Chrome 153 at 16px: 94 and
/// 67).
#[test]
fn display_block_still_reads_size_and_cols() {
    assert_eq!(block_control("input", 16.0, &[("size", "5")]), 94.0);
    assert_eq!(block_control("textarea", 16.0, &[("cols", "5")]), 67.0);
}

/// An author `width` still wins over the intrinsic shrink-to-fit (Chrome 153:
/// 300, the body's own clamp has no effect since 300 < 800).
#[test]
fn an_author_width_on_a_block_control_still_wins() {
    let mut doc = document();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "input",
        "font: 16px/20px ProbeFace; padding: 0; border: 0; display: block; width: 300px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, c), 300.0);
}

/// A `display: block` control whose checkbox/radio/range/color/image/hidden
/// type this crate does not size is unaffected by #1195 — `is_unstretched_replaced`
/// only answers `true` through `form_control_content_height`, which already
/// excludes them, so a `<input type=checkbox style="display:block">` keeps
/// whatever (unrelated, pre-existing) behaviour it had.
#[test]
fn a_block_checkbox_is_untouched_by_the_replaced_flag() {
    let before = block_control("input", 16.0, &[("type", "text")]);
    let after = block_control("input", 16.0, &[("type", "checkbox")]);
    // The text control shrinks (248); the checkbox does not pick up that
    // behaviour just because it sits in the same module.
    assert_eq!(before, 248.0);
    assert_ne!(after, before);
}

fn w_type(tag: &str, px: f32, ty: &str, attrs: &[(&str, &str)]) -> f32 {
    let mut doc = document();
    let body = doc.body();
    let c = doc.create_element(tag);
    doc.set_attribute(c, "type", ty);
    for (k, v) in attrs {
        doc.set_attribute(c, k, v);
    }
    doc.set_attribute(
        c,
        "style",
        &format!("font: {px}px/20px ProbeFace; padding: 0; border: 0"),
    );
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    width(&doc, c)
}

/// The date/time family and `file`, at 16px and 24px Inter (Chrome 153:
/// date 143/209, time 105.796875/149.1875, datetime-local 226/331,
/// month 169/249, week 157/231, file 344/515). `type` is case-insensitive
/// like every other `<input>` type.
#[test]
fn the_picker_types_have_an_intrinsic_width() {
    for (ty, at16, at24) in [
        ("date", 143.0, 209.0),
        ("DATE", 143.0, 209.0),
        ("time", 106.0, 149.0),
        ("datetime-local", 226.0, 331.0),
        ("month", 169.0, 249.0),
        ("week", 157.0, 231.0),
        ("file", 344.0, 515.0),
    ] {
        assert_eq!(w_type("input", 16.0, ty, &[]), at16, "type={ty} at 16px");
        assert_eq!(w_type("input", 24.0, ty, &[]), at24, "type={ty} at 24px");
    }
}

/// `submit`/`reset` with no `value` take their default label ("Submit" /
/// "Reset"); Chrome 153 at 16px Inter, `padding: 0; border: 0`: 52.65625 and
/// 42.40625 — the width of that label shaped in the same font, since a
/// button with no padding/border has no chrome of its own. Every node's
/// final layout is whole-pixel (`taffy::round_layout`), so the asserted
/// value is Chrome's rounded to the nearest pixel (53, 42) — this crate's
/// own shaping of "Submit" is close to Chrome's but not bit-identical
/// (52.648438 against 52.65625), and both round to 53.
#[test]
fn submit_and_reset_default_to_their_label_text() {
    assert_eq!(w_type("input", 16.0, "submit", &[]), 53.0);
    assert_eq!(w_type("input", 16.0, "reset", &[]), 42.0);
}

/// A `value` is the label, even an empty one — not the localized default
/// (Chrome 153: a `value=""` button is 0 wide, not "Submit"-wide). "Click" at
/// 16px Inter is 37.359375 in Chrome 153, which rounds to 37 the same way in
/// this crate's own shaping (an exact match here, unlike "Submit" above).
#[test]
fn a_value_attribute_is_the_label_even_when_empty() {
    assert_eq!(w_type("input", 16.0, "submit", &[("value", "")]), 0.0);
    assert_eq!(w_type("input", 16.0, "reset", &[("value", "")]), 0.0);
    assert_eq!(w_type("input", 16.0, "submit", &[("value", "Click")]), 37.0);
}

/// A bare `button` (no `value`) has no default label and is 0 wide (Chrome
/// 153); one with a `value` is that value's shaped width, rounded (Chrome
/// 153 at 16px: "Click" is 37.359375, rounds to 37).
#[test]
fn a_bare_button_is_zero_wide() {
    assert_eq!(w_type("input", 16.0, "button", &[]), 0.0);
    assert_eq!(w_type("input", 16.0, "button", &[("value", "Click")]), 37.0);
}
