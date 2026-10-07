//! #658 and #662 — an **auto-width** atomic inline (`inline-block`,
//! `inline-flex`, `inline-grid`) is shrink-to-fit (CSS 2.1 §10.3.9,
//! css-sizing-3 §3.1): `min(max-content, max(min-content, available))`, where
//! `available` is its containing block's inner width less its own margins. It
//! used to be sized at max-content, so wrappable content wider than its
//! containing block never wrapped (#658), and a percentage `width` on a child
//! resolved against nothing (#662).
//!
//! Every number is **Chrome 153**'s (headless, standards mode, `file://`, the
//! bundled Inter registered as `ProbeFace` through `@font-face`, `16px/20px`),
//! read with `getBoundingClientRect`. The containing block is 400 wide and the
//! content's max-content width is well past it (~580), so "fill the block",
//! "max-content" and "shrink-to-fit" give three different answers — no row
//! sits on a value two of them share.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CB: &str = "width: 400px; font: 16px/20px ProbeFace;";
const LONG: &str = "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters";

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

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn size(doc: &RinchDocument, m: &str) -> (f32, f32) {
    let ids = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(ids.len(), 1, "one [data-m={m}]");
    let l = doc.tree.get(ids[0]).unwrap().layout;
    (l.width, l.height)
}

#[track_caller]
fn assert_size(doc: &RinchDocument, m: &str, want: (f32, f32)) {
    let got = size(doc, m);
    assert!(
        (got.0 - want.0).abs() < 1.0 && (got.1 - want.1).abs() < 0.5,
        "{m}: rinch {got:?}, Chrome 153 {want:?}"
    );
}

fn in_cb(cb_extra: &str, inner: &str) -> RinchDocument {
    lay_out(&format!(r#"<div style="{CB}{cb_extra}">{inner}</div>"#))
}

/// The issue's first row: capped at the containing block, so it wraps.
#[test]
fn wrappable_content_wider_than_the_block_is_capped_at_it() {
    let doc = in_cb(
        "",
        &format!(r#"<span data-m="t" style="display: inline-block">{LONG}</span>"#),
    );
    assert_size(&doc, "t", (400.0, 40.0));
}

/// The issue's second row: a percentage `min-width` that cannot bind changes
/// nothing (it took the percentage path, #592's pass A, before).
#[test]
fn a_non_binding_percentage_min_width_is_still_capped() {
    let doc = in_cb(
        "",
        &format!(r#"<span data-m="t" style="display: inline-block; min-width: 10%">{LONG}</span>"#),
    );
    assert_size(&doc, "t", (400.0, 40.0));
}

/// The issue's third row: min-content wins over the available width.
#[test]
fn an_unbreakable_word_overflows_at_its_min_content() {
    let doc = in_cb(
        "",
        r#"<span data-m="t" style="display: inline-block">Wavymillilitersmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm</span>"#,
    );
    assert_size(&doc, "t", (864.672, 20.0));
}

/// Content narrower than the block keeps its max-content width.
#[test]
fn short_content_keeps_its_max_content_width() {
    let doc = in_cb(
        "",
        r#"<span data-m="t" style="display: inline-block">short text</span>"#,
    );
    assert_size(&doc, "t", (71.844, 20.0));
}

#[test]
fn inline_flex_and_inline_grid_are_capped_too() {
    for display in ["inline-flex", "inline-grid"] {
        let doc = in_cb(
            "",
            &format!(r#"<span data-m="t" style="display: {display}">{LONG}</span>"#),
        );
        assert_size(&doc, "t", (400.0, 40.0));
    }
}

/// The box's own margins come out of the available width; its padding and
/// border are inside the border box that is capped.
#[test]
fn margins_come_out_of_the_available_width() {
    let doc = in_cb(
        "",
        &format!(
            r#"<span data-m="t" style="display: inline-block; margin: 0 30px 0 20px; padding: 0 7px; border: 3px solid">{LONG}</span>"#
        ),
    );
    assert_size(&doc, "t", (350.0, 46.0));
}

/// The containing block's padding and border are not available.
#[test]
fn the_containing_blocks_padding_is_not_available() {
    let doc = in_cb(
        "box-sizing: border-box; width: 439px; padding: 0 10px 0 25px; border-left: 4px solid;",
        &format!(r#"<span data-m="t" style="display: inline-block">{LONG}</span>"#),
    );
    assert_size(&doc, "t", (400.0, 40.0));
}

/// A `max-width` below the shrink-to-fit width still binds.
#[test]
fn a_smaller_max_width_still_binds() {
    let doc = in_cb(
        "",
        &format!(
            r#"<span data-m="t" style="display: inline-block; max-width: 300px">{LONG}</span>"#
        ),
    );
    assert_size(&doc, "t", (300.0, 40.0));
}

/// Text before the box on its line does not shrink the available width: the
/// box moves to the next line at the full width.
#[test]
fn text_before_the_box_does_not_narrow_it() {
    let doc = in_cb(
        "",
        &format!(r#"lead text <span data-m="t" style="display: inline-block">{LONG}</span>"#),
    );
    assert_size(&doc, "t", (400.0, 40.0));
}

/// Nested auto-width atomic inlines: the inner one is capped at the outer's
/// available width, which is the outer's containing block's.
#[test]
fn nested_auto_width_atomic_inlines_are_both_capped() {
    let doc = in_cb(
        "",
        &format!(
            r#"<span data-m="o" style="display: inline-block"><span data-m="i" style="display: inline-block">{LONG}</span></span>"#
        ),
    );
    assert_size(&doc, "o", (400.0, 40.0));
    assert_size(&doc, "i", (400.0, 40.0));
}

/// The twin table's `two` content in a 150px block: min-content 100,
/// max-content 200, so shrink-to-fit is the block's 150.
#[test]
fn two_boxes_in_a_narrow_block_wrap_at_the_block() {
    let doc = lay_out(
        r#"<div style="width: 150px"><span data-m="t" style="display: inline-block"><span style="display: inline-block; width: 100px; height: 20px; vertical-align: top"></span><span style="display: inline-block; width: 100px; height: 20px; vertical-align: top"></span></span></div>"#,
    );
    assert_size(&doc, "t", (150.0, 40.0));
}

/// #662: a percentage width on an atomic-inline child of an auto-width
/// `inline-block` resolves against the parent's shrink-to-fit width — which
/// is sized from the child's own `auto` contribution (the cyclic percentage
/// counts as `auto` there), so the child comes out at half of its own text.
#[test]
fn a_percentage_atomic_inline_child_resolves_against_the_shrink_to_fit_width() {
    let doc = in_cb(
        "",
        r#"<span data-m="o" style="display: inline-block"><span data-m="i" style="display: inline-block; width: 50%">Wavy</span></span>"#,
    );
    assert_size(&doc, "o", (41.688, 20.0));
    assert_size(&doc, "i", (20.844, 20.0));
}

/// #662 with a block child: its 50% resolves against the parent's
/// shrink-to-fit width (that of "more text here", the widest line).
#[test]
fn a_percentage_block_child_resolves_against_the_shrink_to_fit_width() {
    let doc = in_cb(
        "",
        r#"<span data-m="o" style="display: inline-block"><div data-m="i" style="width: 50%">Wavy milliliters</div>more text here</span>"#,
    );
    assert_size(&doc, "o", (112.328, 60.0));
    assert_size(&doc, "i", (56.156, 40.0));
}

/// #662 as the issue writes it, with the markup's own white space around the
/// child (collapsed away at the line's edges in Chrome).
#[test]
fn a_percentage_child_with_white_space_around_it() {
    let doc = in_cb(
        "",
        "<span data-m=\"o\" style=\"display: inline-block\">\n    <span data-m=\"i\" style=\"display: inline-block; width: 50%\">Wavy</span>\n  </span>",
    );
    assert_size(&doc, "o", (41.688, 20.0));
    assert_size(&doc, "i", (20.844, 20.0));
}
