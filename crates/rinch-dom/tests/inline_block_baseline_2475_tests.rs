//! An inline-block's last baseline, and an `<img>`'s `align`/`valign`, as
//! Chrome lines them up (#2475). Each case is a 5px height difference in
//! real mail: the strut's descent below a box that sat on its bottom edge in
//! one engine and on a line in the other.
//!
//! - White space after an inline-block's last block child is an IFC with no
//!   content, which has no line box (CSS 2.1 §9.4.2), so no baseline: the
//!   box is lined up by the last line before it.
//! - Chrome's inline-block baseline passes over tables, through block
//!   wrappers, `display: table` included: the last line outside one, or the
//!   bottom edge.
//! - `<img valign>` is `vertical-align` (Blink's `HTMLImageElement`);
//!   `align=bottom` is `baseline`, `align=center` is `middle`.
//!
//! Every number is Chrome 157's (`--headless=new`, standards mode, the
//! bundled Inter registered as `ProbeFace` through `@font-face`,
//! `16px/20px`, a 400px block), `getBoundingClientRect` relative to the
//! block. `k` is a one-line inline-block before the box: its top is the
//! line's baseline less 16.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A 40x30 PNG.
const FORTY_BY_THIRTY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";

fn lay_out(inner: &str) -> RinchDocument {
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
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let c = doc.create_element("div");
    doc.set_attribute(c, "data-m", "c");
    doc.set_attribute(c, "style", "font: 16px/20px ProbeFace; width: 400px");
    doc.set_inner_html(c, &inner.replace("IMG", FORTY_BY_THIRTY));
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(y relative to the block, height)` of `[data-m=m]`.
fn at(doc: &RinchDocument, m: &str) -> (f32, f32) {
    let one = |m: &str| {
        let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
        assert_eq!(found.len(), 1, "{m}");
        found[0]
    };
    let (id, c) = (one(m), one("c"));
    let y = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0).1 as f32;
    (y(id) - y(c), doc.tree.get(id).unwrap().layout.height)
}

/// `k`, then an inline-block 100px wide holding `content`.
fn beside(content: &str) -> String {
    format!(
        r#"<div data-m="r"><span data-m="k" style="display:inline-block">k</span><span style="display:inline-block;width:100px">{content}</span></div>"#
    )
}

/// `(the line's height, k's top)`.
fn line(content: &str) -> (f32, f32) {
    let doc = lay_out(&beside(content));
    (at(&doc, "r").1, at(&doc, "k").0)
}

/// The space after the last block is no line: the box sits on `b`'s line,
/// as without the space. Chrome: 52, k at 26, both ways. rinch made the
/// space's empty line the baseline, at the box's bottom: 56, k at 36.
#[test]
fn white_space_after_the_last_block_is_no_line_box() {
    let blocks = r#"<div>a</div><div style="padding:6px">b</div>"#;
    assert_eq!(line(blocks), (52.0, 26.0), "Chrome: 52, k at 26");
    assert_eq!(
        line(&format!("{blocks} ")),
        (52.0, 26.0),
        "a trailing space; Chrome: 52, k at 26"
    );
}

/// A table after a line: the box sits on `a`'s line, the first. Chrome: 52,
/// k at 0. rinch took the table's cell: k at 26.
#[test]
fn a_table_after_a_line_is_passed_over() {
    let table =
        r#"<table cellspacing="0" cellpadding="0"><tr><td style="padding:6px">b</td></tr></table>"#;
    assert_eq!(
        line(&format!("<div>a</div>{table}")),
        (52.0, 0.0),
        "Chrome: 52, k at 0"
    );
    assert_eq!(
        line(&format!("<div>a</div><div>{table}</div>")),
        (52.0, 0.0),
        "inside a block; Chrome: 52, k at 0"
    );
    assert_eq!(
        line(r#"<div>a</div><div style="display:table;padding:6px">b</div>"#),
        (52.0, 0.0),
        "display: table; Chrome: 52, k at 0"
    );
}

/// A table alone: no line outside it, so the box sits on its bottom edge and
/// the strut's descent hangs below. Chrome: 36, k at 16. rinch: 32, k at 6.
#[test]
fn a_box_holding_only_a_table_sits_on_its_bottom() {
    let table =
        r#"<table cellspacing="0" cellpadding="0"><tr><td style="padding:6px">b</td></tr></table>"#;
    assert_eq!(line(table), (36.0, 16.0), "Chrome: 36, k at 16");
}

/// `(the line's height, the picture's top)` of a 40x30 picture in a line,
/// after `before`, with `attrs`.
fn picture(before: &str, attrs: &str) -> (f32, f32) {
    let doc = lay_out(&format!(
        r#"<div data-m="r">{before}<img data-m="k" {attrs} src="IMG"></div>"#
    ));
    (at(&doc, "r").1, at(&doc, "k").0)
}

/// `valign` on a picture is its `vertical-align`. Chrome: `bottom` 30
/// (no descent under it), `top` 30, a value that is no keyword the
/// baseline's 34. rinch ignored it: 34.
#[test]
fn a_pictures_valign_is_its_vertical_align() {
    assert_eq!(picture("", r#"valign="bottom""#), (30.0, 0.0), "Chrome: 30");
    assert_eq!(picture("", r#"valign="top""#), (30.0, 0.0), "Chrome: 30");
    assert_eq!(picture("", r#"valign="foo""#), (34.0, 0.0), "Chrome: 34");
    assert_eq!(
        picture("", r#"align="foo" valign="top""#),
        (30.0, 0.0),
        "Chrome: 30"
    );
}

/// `align`'s keywords as Chrome maps them: `bottom` is the baseline (34),
/// `absbottom` the bottom and `absmiddle` the middle (30). rinch mapped
/// `bottom` as the bottom (30).
#[test]
fn a_pictures_align_keywords_are_chromes() {
    assert_eq!(picture("", r#"align="bottom""#), (34.0, 0.0), "Chrome: 34");
    assert_eq!(
        picture("", r#"align="absbottom""#),
        (30.0, 0.0),
        "Chrome: 30"
    );
    assert_eq!(
        picture("", r#"align="absmiddle""#),
        (30.0, 0.0),
        "Chrome: 30"
    );
}

/// `align=center` is `align=middle` in Chrome (31 alone in its line, the
/// picture at 1, both; 34 unmapped). rinch ignored `center`: the baseline.
/// Both are Blink's `-webkit-baseline-middle`, which rinch lays out as
/// `middle` (30, the picture at 0): a 1px difference this does not fix.
#[test]
fn a_pictures_align_center_is_align_middle() {
    for before in ["", "x"] {
        assert_eq!(
            picture(before, r#"align="center""#),
            picture(before, r#"align="middle""#),
            "after {before:?}: Chrome lines both up alike"
        );
    }
    assert_ne!(picture("", r#"align="center""#).0, 34.0, "not the baseline");
}
