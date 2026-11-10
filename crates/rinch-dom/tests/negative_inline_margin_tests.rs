//! A negative inline-start or inline-end margin on an inline element (#2406).
//! It takes room off the line like a positive one adds it: what follows is
//! pulled back by it, and the line still fits what it then has room for.
//! rinch used to drop a negative edge (`ifc::inline_edge_widths` clamped it to
//! 0 and no edge box was pushed), so nothing moved.
//!
//! # Chrome 156, headless, standards mode
//!
//! Every container is `width: 400px; font-size: 12px; line-height: 23px`;
//! `.ib` is a 10x10 `inline-block`. The numbers are the `inline-block`'s
//! left, from boxes of declared widths only, so no number depends on fonts.
//!
//! | content | Chrome | rinch before |
//! |---|---|---|
//! | `<span margin-left:-5px></span><ib>` | -5 | 0 |
//! | `<ib><span margin-right:-7px></span><ib>` | 3 | 10 |
//! | `<ib><span margin-left:-3px;padding-left:5px></span><ib>` | 12 | 15 |
//! | 25px wide: `<ib><ib><span margin-left:-15px></span><ib>` | 5, on line 1 | 0, on line 2 |
//!
//! The `-3 + 5` row passed before too (its total is positive); it holds the
//! sum against a fix that drops the margin or the padding.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const IB: &str = r#"<span class="ib"></span>"#;

/// Lay `inner` out in a `.c` container `width` wide and return the `#t`
/// box's `(x, y)` in the container.
fn target(width: f32, inner: &str) -> (f32, f32) {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "body { margin: 0 } .c { font-size: 12px; line-height: 23px } \
         .ib { display: inline-block; width: 10px; height: 10px }",
    );
    let body = doc.body();
    doc.set_inner_html(
        body,
        &format!(r#"<div class="c" style="width:{width}px">{inner}</div>"#),
    );
    doc.resolve_layout(720.0, 800.0);
    let t = rinch_dom::query::query_all(&doc.tree, doc.tree.root_id, "#t")[0];
    let l = doc.tree.nodes[t].layout;
    (l.x, l.y)
}

const T: &str = r#"<span class="ib" id="t"></span>"#;

#[test]
fn a_negative_inline_start_margin_pulls_what_follows_back() {
    let got = target(
        400.0,
        &format!(r#"<span style="margin-left:-5px"></span>{T}"#),
    );
    assert_eq!(got.0, -5.0, "Chrome -5");
}

#[test]
fn a_negative_inline_end_margin_pulls_what_follows_back() {
    let got = target(
        400.0,
        &format!(r#"{IB}<span style="margin-right:-7px"></span>{T}"#),
    );
    assert_eq!(got.0, 3.0, "Chrome 3");
}

/// The edge is the margin, border and padding together: -3 + 5 = 2.
#[test]
fn a_negative_margin_and_a_padding_on_one_side_add_up() {
    let got = target(
        400.0,
        &format!(r#"{IB}<span style="margin-left:-3px;padding-left:5px"></span>{T}"#),
    );
    assert_eq!(got.0, 12.0, "Chrome 12");
}

/// 10 + 10 - 15 + 10 = 15 fits a 25px line, so the third box stays on the
/// first line, at 5 (30 would not have fit).
#[test]
fn a_negative_margin_gives_the_line_room_back() {
    let (x, y) = target(
        25.0,
        &format!(r#"{IB}{IB}<span style="margin-left:-15px"></span>{T}"#),
    );
    let first_line = target(25.0, &format!(r#"{T}{IB}"#)).1;
    assert_eq!((x, y), (5.0, first_line), "Chrome x 5, on the first line");
}
