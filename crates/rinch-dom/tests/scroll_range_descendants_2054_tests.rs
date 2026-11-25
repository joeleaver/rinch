//! #2054: a scroll container's range counts its descendants' overflow, not
//! only its children's: each case against Chrome's answer, and the walk that
//! measures it kept between reads. (Split out of
//! `geometry_queries_2257_tests.rs`; the kept walk's own rules are pinned in
//! `deep_overflow_kept_2054_tests.rs`.)

use rinch_core::dom::{DomContent, DomScroll, DomTree};
use rinch_dom::RinchDocument;

// ── #2054: a scroll container's range counts its descendants' overflow ─────
//
// Chrome 158, `::-webkit-scrollbar { width: 0; height: 0 }`: each scroller
// is 260x30 (`overflow: auto`) unless it says otherwise. The range
// (`scrollWidth - clientWidth`, `scrollHeight - clientHeight`) is compared:
// rinch measures `scroll_width` and `client_width` from the content box, the
// browser from the padding box, and their difference is the range on both.

const SCROLLERS: &[(&str, &str, &str, f64, f64)] = &[
    // A grandchild wider than the scroller: 460 - 260.
    (
        "s1",
        "",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div>"#,
        200.0,
        0.0,
    ),
    // Padded: the grandchild where it is drawn, with no end padding.
    (
        "s2",
        "padding:10px",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div>"#,
        190.0,
        0.0,
    ),
    // The issue's shape: a 300px box in a `height: 50px` child of a 200x100
    // scroller.
    (
        "s3",
        "width:200px;height:100px",
        r#"<div style="height:50px"><div style="height:300px;width:10px"></div></div>"#,
        0.0,
        200.0,
    ),
    // A child that clips keeps its overflow.
    (
        "s4",
        "",
        r#"<div style="width:260px;height:10px;overflow:hidden"><div style="width:460px;height:50px"></div></div>"#,
        0.0,
        0.0,
    ),
    // An absolute grandchild contained above the scroller, and a fixed one
    // with content: neither is its overflow.
    (
        "s5",
        "",
        r#"<div style="width:260px;height:10px"><div style="position:absolute;left:300px;top:0;width:100px;height:5px"></div></div>"#,
        0.0,
        0.0,
    ),
    (
        "s6",
        "",
        r#"<div style="width:260px;height:10px"><div style="position:fixed;left:300px;top:0;width:100px;height:5px"><div style="width:600px;height:5px"></div></div></div>"#,
        0.0,
        0.0,
    ),
    // A transformed grandchild, where it is drawn.
    (
        "s7",
        "",
        r#"<div style="width:260px;height:10px"><div style="width:100px;height:5px;transform:translateX(300px)"></div></div>"#,
        140.0,
        0.0,
    ),
    // Right to left: the grandchild overflows to the left.
    (
        "s8",
        "direction:rtl",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div>"#,
        200.0,
        0.0,
    ),
    // An absolute box the scroller contains, holding a wider box.
    (
        "s9",
        "position:relative",
        r#"<div style="width:260px;height:10px"><div style="position:absolute;left:0;top:0;width:100px;height:5px"><div style="width:500px;height:5px"></div></div></div>"#,
        240.0,
        0.0,
    ),
    // A grandchild's margins count nothing.
    (
        "s10",
        "padding:10px",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px;margin-right:50px;margin-bottom:60px"></div></div>"#,
        190.0,
        0.0,
    ),
    // Bottom padding: not added below the grandchild.
    (
        "s11",
        "width:200px;height:100px;padding:0 0 20px 0",
        r#"<div style="height:50px"><div style="height:300px;width:10px"></div></div>"#,
        0.0,
        180.0,
    ),
];

fn scroller_doc() -> RinchDocument {
    let mut html = String::new();
    for &(id, extra, inner, _, _) in SCROLLERS {
        html.push_str(&format!(
            r#"<div id="{id}" style="width:260px;height:30px;overflow:auto;{extra}">{inner}</div>"#
        ));
    }
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, &html);
    d.resolve_layout(800.0, 600.0);
    d
}

/// The wheel's and the bar's range, and `scroll_width`/`scroll_height`,
/// count every descendant that overflows its own parent, as Chrome 158 does
/// (#2054); the s1 scroller answered 260, Chrome 460.
#[test]
fn a_scroll_containers_range_counts_its_descendants_overflow() {
    let d = scroller_doc();
    let mut bad = Vec::new();
    for &(id, _, _, rx, ry) in SCROLLERS {
        let n = d.query_selector(&format!("#{id}")).expect(id);
        let range = (
            (d.scroll_width(n) - d.client_width(n)).max(0.0),
            (d.scroll_height(n) - d.client_height(n)).max(0.0),
        );
        let bar = rinch_dom::paint::scrollbar::content_extents(&d.tree, n.0);
        if (range.0 - rx).abs() > 0.01
            || (range.1 - ry).abs() > 0.01
            || bar != (d.scroll_width(n), d.scroll_height(n))
        {
            bad.push(format!(
                "#{id}: range {range:?}, bar extents {bar:?}, Chrome {:?}",
                (rx, ry)
            ));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

/// The walk is kept: reading the range again walks nothing, a layout that
/// moves nothing under the scroller measures nothing, and one that moves a
/// box under it measures that box and the box above it, walks nothing, and
/// answers the new range.
#[test]
fn a_scroll_containers_descendant_walk_is_kept_until_layout_under_it_changes() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(
        body,
        r#"<div id="s" style="width:260px;height:30px;overflow:auto"><div style="width:260px;height:10px"><div id="g" style="width:460px;height:5px"></div></div></div><div id="other" style="width:50px;height:10px"></div>"#,
    );
    d.resolve_layout(800.0, 600.0);
    let s = d.query_selector("#s").expect("#s");
    let walks = |d: &RinchDocument| rinch_dom::testing::deep_overflow_walks(&d.tree);
    let measured = |d: &RinchDocument| rinch_dom::testing::deep_overflow_measured(&d.tree);
    assert_eq!(d.scroll_width(s), 460.0);
    let after_first = walks(&d);
    for _ in 0..5 {
        assert_eq!(d.scroll_width(s), 460.0);
        let _ = rinch_dom::paint::scrollbar::content_extents(&d.tree, s.0);
    }
    assert_eq!(walks(&d), after_first, "reads in between walk nothing");

    // A layout pass that moves a box outside the scroller only. (The
    // counter counts every container's walks: the page's own scroller may
    // walk too, so each step counts across the scroller's read alone.)
    let other = d.query_selector("#other").expect("#other");
    d.tree.consume_paint_dirty();
    d.set_attribute(other, "style", "width:80px;height:10px");
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(d.scroll_width(s), 460.0);
    assert_eq!(walks(&d), before, "nothing moved under the scroller");
    assert_eq!(
        measured(&d) - measured_before,
        1,
        "nothing of the scroller's: #other alone, in the root element's own kept walk"
    );

    // A colour on the grandchild (a hover): pushed paint-dirty, nothing the
    // walk read changed.
    let g = d.query_selector("#g").expect("#g");
    d.set_style(g, "background-color", "rgb(200, 0, 0)");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(d.scroll_width(s), 460.0);
    assert_eq!(walks(&d), before, "a colour keeps the walk");

    // One that moves the grandchild.
    d.tree.consume_paint_dirty();
    d.set_attribute(g, "style", "width:560px;height:5px");
    d.resolve_layout(800.0, 600.0);
    let (before, measured_before) = (walks(&d), measured(&d));
    assert_eq!(d.scroll_width(s), 560.0, "the new range");
    assert_eq!(walks(&d), before, "brought up to date, not walked");
    assert_eq!(
        measured(&d) - measured_before,
        2,
        "the grandchild and its parent"
    );
}

/// A transformed box the walk met is measured where it is drawn on each
/// read: a new `transform` moves the range with nothing laid out again.
#[test]
fn a_transformed_descendant_moves_the_kept_range() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(
        body,
        r#"<div id="s" style="width:260px;height:30px;overflow:auto"><div style="width:260px;height:10px"><div id="t" style="width:100px;height:5px;transform:translateX(300px)"></div></div></div>"#,
    );
    d.resolve_layout(800.0, 600.0);
    let s = d.query_selector("#s").expect("#s");
    assert_eq!(d.scroll_width(s), 400.0);
    let t = d.query_selector("#t").expect("#t");
    d.set_style(t, "transform", "translateX(350px)");
    d.resolve_layout(800.0, 600.0);
    assert_eq!(d.scroll_width(s), 450.0);
}
