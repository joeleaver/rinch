//! A table part whose parent box is an inline (CSS 2.1 §17.2.1, CSS Tables 3
//! §3.4): CSS wraps it in an anonymous table, and in an inline parent that
//! table is an `inline-table`, an atomic inline on the line. Mail sets
//! `display: table-cell` on a `<span>` inside a link beside an icon, and the
//! count sits beside the icon in Chrome; rinch laid the cell out as a block
//! and broke the line around it.
//!
//! Every number is Chrome 157's (`--headless=new`, standards mode, the
//! bundled Inter registered as `ProbeFace` through `@font-face`,
//! `16px/20px`, a 400px block), `getBoundingClientRect` relative to the
//! block, rounded to 1/100 px: `[x, y, width, height]`.
//!
//! Issue #2510.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// A 40x30 PNG.
const PICTURE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";

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
    doc.set_inner_html(c, &inner.replace("IMG", PICTURE));
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> rinch_dom::node::RawNodeId {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "{m}");
    found[0]
}

/// `[x, y, width, height]` of `[data-m=m]`, relative to the block.
fn rect(doc: &RinchDocument, m: &str) -> [f32; 4] {
    let (id, c) = (one(doc, m), one(doc, "c"));
    let pos = |id| rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let (p, q) = (pos(id), pos(c));
    let l = &doc.tree.get(id).unwrap().layout;
    [(p.0 - q.0) as f32, (p.1 - q.1) as f32, l.width, l.height]
}

fn height(doc: &RinchDocument) -> f32 {
    doc.tree.get(one(doc, "c")).unwrap().layout.height
}

/// `x` and `width` within half a pixel, `y` and `height` exact: rinch
/// rounds an atomic inline's shrink-to-fit width to a whole pixel (16 for
/// Chrome's 16.27), which moves what follows it by the difference.
#[track_caller]
fn near(got: [f32; 4], chrome: [f32; 4], what: &str) {
    let tol = [0.5, 0.0, 0.5, 0.0];
    let close = (0..4).all(|i| (got[i] - chrome[i]).abs() <= tol[i] + 0.01);
    assert!(close, "{what}: rinch {got:?}, Chrome {chrome:?}");
}

/// The mail repro: a 16px icon and a `vertical-align: middle` table-cell
/// count in a link, in a `nowrap` cell. Chrome: the count beside the icon,
/// its content bottom on the baseline (the icon's bottom), 56 tall with the
/// padding. rinch broke the link around a block cell: 72, the count under
/// the icon at x 0.
#[test]
fn a_cell_in_a_link_sits_beside_the_icon() {
    let doc = lay_out(
        r##"<table cellspacing=0 cellpadding=0><tr><td style="white-space:nowrap;padding:16px 16px 16px 0"><a href="#" style="color:#0366d6;text-decoration:none"><img data-m=i width=16 height=16 src="IMG"><span data-m=s style="display:table-cell !important;vertical-align:middle !important;color:#6a737d">1</span></a></td></tr></table>"##,
    );
    assert_eq!(height(&doc), 56.0, "Chrome: 56");
    near(rect(&doc, "i"), [0.0, 20.0, 16.0, 16.0], "icon");
    near(rect(&doc, "s"), [16.0, 16.0, 6.52, 20.0], "count");
}

/// A bare cell in a plain inline span: on the line, as wide as its text.
/// rinch: a 400px block on a line of its own, 60 tall.
#[test]
fn a_cell_in_an_inline_is_on_the_line() {
    let doc = lay_out(
        r#"<div>ab <span>x<span data-m=s style="display:table-cell">12</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "Chrome: one line");
    near(rect(&doc, "s"), [32.02, 0.0, 16.27, 20.0], "cell");
}

/// Two cells side by side are one anonymous table: the space between them
/// is dropped, also with a comment beside it. rinch: two blocks, 80 tall.
#[test]
fn two_cells_share_a_table_and_drop_the_space_between() {
    for between in [" ", " <!-- c --> "] {
        let doc = lay_out(&format!(
            r#"<div>ab <span>x<span data-m=s style="display:table-cell">12</span>{between}<span data-m=t style="display:table-cell">34</span>y</span> z</div>"#
        ));
        assert_eq!(height(&doc), 20.0, "{between:?}: Chrome: one line");
        near(rect(&doc, "s"), [32.02, 0.0, 16.27, 20.0], "first");
        near(rect(&doc, "t"), [48.28, 0.0, 20.22, 20.0], "second");
    }
}

/// White space between a cell and text stays: it is outside the table.
#[test]
fn a_space_beside_text_stays() {
    let doc = lay_out(
        r#"<div>ab <span>x <span data-m=s style="display:table-cell">12</span> y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "Chrome: one line");
    near(
        rect(&doc, "s"),
        [36.52, 0.0, 16.27, 20.0],
        "cell after `x `",
    );
}

/// A cell in a row in a row group in an inline: the same inline-table.
/// A row holding no cell flows on the line too. rinch: the cell a block, 60.
#[test]
fn rows_and_row_groups_in_an_inline_stay_on_the_line() {
    let doc = lay_out(
        r#"<div>ab <span>x<span style="display:table-row-group"><span style="display:table-row"><span data-m=s style="display:table-cell">12</span></span></span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "Chrome: one line");
    near(rect(&doc, "s"), [32.02, 0.0, 16.27, 20.0], "cell");
    let doc = lay_out(
        r#"<div>ab <span>x<span style="display:table-row"><span>12</span></span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "a row of text; Chrome: one line");
}

/// In a block, the anonymous table is block-level: the cell is on a line
/// of its own, between `ab` and `z` (60; 70 with 5px padding). Unchanged.
/// (Chrome's table is as wide as the cell's content, 16.27 and 26.27;
/// rinch's cell is the block's width, which only a background shows.)
#[test]
fn a_cell_in_a_block_is_a_block_table() {
    let doc = lay_out(r#"<div>ab <span data-m=s style="display:table-cell">12</span> z</div>"#);
    assert_eq!(height(&doc), 60.0, "Chrome: 60");
    assert_eq!(rect(&doc, "s")[1], 20.0, "Chrome: y 20");
    let doc =
        lay_out(r#"<div>ab<span data-m=s style="display:table-cell;padding:5px">12</span>z</div>"#);
    assert_eq!(height(&doc), 70.0, "Chrome: 70");
    let s = rect(&doc, "s");
    assert_eq!((s[1], s[3]), (20.0, 30.0), "Chrome: y 20, 30 tall");
}

/// A `vertical-align: middle` cell beside a 40x30 picture: the table's
/// baseline is the cell's content bottom (CSS 2.1 §17.5.3), on the
/// picture's bottom: the cell at 7 (30 - 3 - 20), the line 34. An empty
/// baseline cell has no line: its content bottom too (14 + 2 = 16).
#[test]
fn a_middle_cell_lines_up_by_its_content_bottom() {
    let doc = lay_out(
        r#"<div>ab <span>x<img data-m=i width=40 height=30 src="IMG"><span data-m=s style="display:table-cell;vertical-align:middle;padding:3px">12</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 34.0, "Chrome: 34");
    near(rect(&doc, "i"), [32.02, 0.0, 40.0, 30.0], "picture");
    near(rect(&doc, "s"), [72.02, 7.0, 22.27, 26.0], "cell");
    let doc = lay_out(
        r#"<div>ab <span>x<span data-m=s style="display:table-cell;padding:2px"></span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "Chrome: 20");
    near(rect(&doc, "s"), [32.02, 14.0, 4.0, 4.0], "empty cell");
}

/// `nowrap` keeps the cell on the first line of a 60px block; with wrapping
/// it goes to the next line whole, as an atomic inline does.
#[test]
fn white_space_and_wrapping() {
    let doc = lay_out(
        r#"<div style="width:60px;white-space:nowrap">ab <span>x<span data-m=s style="display:table-cell">1234</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "nowrap; Chrome: one line");
    near(rect(&doc, "s"), [32.02, 0.0, 36.48, 20.0], "nowrap");
    let doc = lay_out(
        r#"<div style="width:60px">ab <span>x<span data-m=s style="display:table-cell">12 34</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 60.0, "wrapping; Chrome: 60");
    near(rect(&doc, "s"), [0.0, 20.0, 40.98, 20.0], "wrapping");
}

/// The cell is decided by its parent box's display: a parent restyled from
/// inline to block makes it a block-level table on a line of its own
/// (Chrome: 100 tall, the cell at y 40), and back. A display change
/// recascades the children, but not a row's cells, which inherit nothing
/// that changed from a row that did not change: those are recascaded by
/// the parent's change.
#[test]
fn a_parent_display_change_redecides_the_cell() {
    for (open, close) in [("", ""), (r#"<span style="display:table-row">"#, "</span>")] {
        let mut doc = lay_out(&format!(
            r#"<div>ab <span data-m=p>x{open}<span data-m=s style="display:table-cell">12</span>{close}y</span> z</div>"#
        ));
        assert_eq!(height(&doc), 20.0, "{open:?}: inline parent");
        let p = NodeId(one(&doc, "p"));
        doc.set_attribute(p, "style", "display: block");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(height(&doc), 100.0, "{open:?}: block parent; Chrome: 100");
        assert_eq!(
            rect(&doc, "s")[1],
            40.0,
            "{open:?}: Chrome: the cell at y 40"
        );
        doc.remove_attribute(p, "style");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(height(&doc), 20.0, "{open:?}: inline again");
        near(rect(&doc, "s"), [32.02, 0.0, 16.27, 20.0], "inline again");
    }
}

/// The anonymous table inherits `border-spacing`, its padding: 2px under a
/// `<table>` with no `cellspacing`, so the count is 2px right of the icon
/// and 2px lower, its content bottom 2px above the line's baseline. Chrome:
/// the cell at x 20 (the `<td>` at 2), the line 30 tall. rinch ignored the
/// spacing: 28, the cell at 18.
#[test]
fn the_default_two_pixel_spacing_pads_the_table() {
    let doc = lay_out(
        r##"<table cellpadding=0><tr><td data-m=d style="white-space:nowrap"><a href="#"><img data-m=i width=16 height=16 src="IMG"><span data-m=s style="display:table-cell;vertical-align:middle">1</span></a></td></tr></table>"##,
    );
    assert_eq!(height(&doc), 30.0, "Chrome: 30");
    near(rect(&doc, "d"), [2.0, 2.0, 26.52, 26.0], "the td");
    near(rect(&doc, "i"), [2.0, 8.0, 16.0, 16.0], "icon");
    near(rect(&doc, "s"), [20.0, 4.0, 6.52, 20.0], "count");
}

/// `border-spacing: 10px` on an ancestor: 10px around the cell (Chrome: at
/// x 42.02, y 10, the line 40); `3px 7px` is 3 across and 7 down, and an
/// inline-block after it starts 3px past it. Collapsed borders have no
/// spacing. rinch: 20 tall, the cell at 32.02.
#[test]
fn an_inherited_spacing_pads_the_table() {
    let doc = lay_out(
        r#"<div style="border-spacing:10px">ab <span>x<span data-m=s style="display:table-cell">12</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 40.0, "Chrome: 40");
    near(rect(&doc, "s"), [42.02, 10.0, 16.27, 20.0], "10px");
    let doc = lay_out(
        r#"<div style="border-spacing:3px 7px">ab <span>x<span data-m=s style="display:table-cell">12</span><span data-m=k style="display:inline-block">y</span></span> z</div>"#,
    );
    assert_eq!(height(&doc), 34.0, "Chrome: 34");
    near(rect(&doc, "s"), [35.02, 7.0, 16.27, 20.0], "3px 7px");
    near(rect(&doc, "k"), [54.28, 7.0, 9.0, 20.0], "after it");
    let doc = lay_out(
        r#"<div style="border-spacing:10px;border-collapse:collapse">ab <span>x<span data-m=s style="display:table-cell">12</span>y</span> z</div>"#,
    );
    assert_eq!(height(&doc), 20.0, "collapse; Chrome: 20");
    near(rect(&doc, "s"), [32.02, 0.0, 16.27, 20.0], "collapse");
}

/// Two cells are one table: one spacing between them, not two. Chrome:
/// 42.02, 68.28, then the inline-block at 98.5.
#[test]
fn two_cells_share_one_spacing() {
    let doc = lay_out(
        r#"<div style="border-spacing:10px">ab <span>x<span data-m=s style="display:table-cell">12</span> <span data-m=t style="display:table-cell">34</span><span data-m=k style="display:inline-block">y</span></span> z</div>"#,
    );
    assert_eq!(height(&doc), 40.0, "Chrome: 40");
    near(rect(&doc, "s"), [42.02, 10.0, 16.27, 20.0], "first");
    near(rect(&doc, "t"), [68.28, 10.0, 20.22, 20.0], "second");
    near(rect(&doc, "k"), [98.5, 10.0, 9.0, 20.0], "after");
}

/// What follows a cell: a white-space text node right after it goes
/// (Chrome takes it into the anonymous table), before an element or a
/// `<b>`; text that starts with a space keeps the space (4.5px). Chrome:
/// `k` at 48.28 after `ab x`, and at 34, 38.5 and 34 after `x`.
#[test]
fn what_follows_a_cell() {
    let at = |html: &str| rect(&lay_out(html), "k")[0];
    let k = r#"<span data-m=k style="display:inline-block">k</span>"#;
    let cell = r#"<span style="display:table-cell">12</span>"#;
    assert!((at(&format!("<div>ab <span>x{cell} {k}</span></div>")) - 48.28).abs() < 0.5);
    let after = |rest: &str| at(&format!("<div><span>x{cell}{rest}{k}</span></div>"));
    assert!((after("y") - 34.0).abs() < 0.5, "no space");
    assert!(
        (after(" y") - 38.5).abs() < 0.5,
        "text with a space: {}",
        after(" y")
    );
    assert!(
        (after(" <b>y</b>") - 34.0).abs() < 0.5,
        "a blank node: {}",
        after(" <b>y</b>")
    );
    let before = rect(
        &lay_out(&format!("<div><span>{k} {cell}</span></div>").replace(
            r#"style="display:table-cell""#,
            r#"data-m=s style="display:table-cell""#,
        )),
        "s",
    )[0];
    assert!(
        (before - 13.5).abs() < 0.5,
        "a space before a cell stays: {before}"
    );
}
