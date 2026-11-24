//! #2257: `paint::painted_border_box` — the box `ClickContext`'s `element_*`
//! fields, the desktop `bounds_signal` and every other painted-box reader in
//! `rinch` take — answered `(x, y, 0, 0)` for a flowed inline element (a
//! `<span>` in a paragraph): its layout is a `0x0` ghost. Chrome's
//! `getBoundingClientRect()`, which rinch-web fills those fields from, is the
//! union of its fragments, and so is rinch-dom's own `bounding_rect` (#1779).
//! Every expectation is Chrome's (headless, 800x600), with the bundled Inter
//! loaded as `ProbeFace` so the horizontal geometry pins no host font.

#[path = "support/inline_geom_2616.rs"]
mod scene;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = "position: absolute; left: 100px; top: 50px; width: 400px; \
                 font: 16px/20px ProbeFace; padding: 7px 0 0 11px; margin: 0";

fn doc(extra: &str, inner: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css("* { box-sizing: border-box } body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, &format!(r#"<div style="{C}; {extra}">{inner}</div>"#));
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    let s = *rinch_dom::testing::query_selector(&doc.tree, "[id=s]")
        .first()
        .expect("[id=s]");
    (doc, NodeId(s))
}

fn painted(doc: &RinchDocument, id: NodeId, scale: f64) -> [f64; 4] {
    let r = rinch_dom::paint::painted_border_box(&doc.tree, id.0, scale);
    [r.x0, r.y0, r.width(), r.height()]
}

fn assert_chrome(got: [f64; 4], chrome: [f64; 4], what: &str) {
    assert!(
        got.iter().zip(chrome).all(|(g, c)| (g - c).abs() < 0.05),
        "{what}: painted {got:?}, Chrome {chrome:?}"
    );
}

/// The issue's case: a span after an inline-block. It was `(0, 0, 0, 0)`
/// in the issue's probe; here it sits off the origin and off the
/// inline-block, so neither a zero box nor the inline-block's passes.
#[test]
fn a_span_after_an_inline_block_is_painted_over_its_fragment() {
    let (d, s) = doc(
        "",
        r#"<span style="display:inline-block">mid</span> <span id="s">text</span>"#,
    );
    // Within half a pixel on x: rinch lays the inline-block out on a whole
    // pixel (Taffy rounds its width, 32.33px in Chrome), so the text after
    // it starts 0.3px right of Chrome's. The box at the origin, or an empty
    // one, is many pixels off.
    let got = painted(&d, s, 1.0);
    let chrome = [143.1875, 57.0, 28.328125, 20.0];
    assert!(
        (got[0] - chrome[0]).abs() < 0.5
            && got[1..]
                .iter()
                .zip(&chrome[1..])
                .all(|(g, c)| (g - c).abs() < 0.05),
        "#s: painted {got:?}, Chrome {chrome:?}"
    );
}

/// A span in a bigger font than its line takes its own font's ascent and
/// descent (39px, starting above the line), and a smaller padded one its
/// own: read from the shared borrow, through the font boxes shaping the
/// lines recorded, not from the line's metrics.
#[test]
fn a_span_in_another_font_is_its_own_fonts_height() {
    // Chrome measured the container at `top: 150px` / `250px`.
    let (d, s) = doc(
        "",
        r#"aaaa <span id="s" style="font-size:32px">Bbbb</span> cc"#,
    );
    assert_chrome(
        painted(&d, s, 1.0),
        [151.4375, 47.0, 79.71875, 39.0],
        "32px",
    );
    let (d, s) = doc(
        "",
        r#"aaaa <span id="s" style="font-size:10px;padding:2px 3px">bbbb</span> cc"#,
    );
    assert_chrome(painted(&d, s, 1.0), [151.4375, 61.0, 30.5, 16.0], "10px");
}

/// A span wrapped over two lines: the union of both fragments.
#[test]
fn a_wrapped_span_is_painted_over_the_union_of_its_fragments() {
    let (d, s) = doc(
        "width: 120px",
        r#"aaaa aaaa <span id="s">bbbb cccc dddd eeee</span> ff"#,
    );
    assert_chrome(painted(&d, s, 1.0), [111.0, 77.0, 81.0, 40.0], "#s");
}

/// Under a `scale(2)` on the block holding the line, the transformed union
/// (Chrome's, as `bounding_rect_1779_tests` pins it); and at a device scale
/// of 2 every edge doubles, as for any box.
#[test]
fn a_span_under_a_transform_and_a_device_scale() {
    let (d, s) = doc(
        "transform: scale(2); transform-origin: 0 0",
        r#"aaaa <span id="s">bbbb</span> cc"#,
    );
    assert_chrome(
        painted(&d, s, 1.0),
        [202.875, 64.0, 78.375, 40.0],
        "scale(2)",
    );
    let (d, s) = doc("", r#"aaaa <span id="s">bbbb</span> cc"#);
    assert_chrome(
        painted(&d, s, 2.0),
        [302.875, 114.0, 78.375, 40.0],
        "device scale 2",
    );
}

/// Wrapped elements in either direction, nested and unevenly padded
/// (#2616's scene): each painted box is Chrome 157's `getBoundingClientRect`,
/// and the same as `bounding_rect`, so the click path, the bounds signals
/// and `NodeHandle::bounding_rect` cannot disagree.
#[test]
fn wrapped_spans_in_either_direction_are_painted_as_chrome_reports_them() {
    let mut d = scene::document();
    let mut bad = Vec::new();
    for &(id, (x, y, w, h)) in scene::BOUNDS {
        let n = NodeId(rinch_dom::testing::query_selector(&d.tree, &format!("#{id}"))[0]);
        let got = painted(&d, n, 1.0);
        let b = d.bounding_rect(n).expect("a box");
        let bounds = [b.x, b.y, b.width, b.height].map(f64::from);
        if got
            .iter()
            .zip([x, y, w, h])
            .any(|(g, c)| (g - c).abs() > 0.01)
            || got.iter().zip(bounds).any(|(g, c)| (g - c).abs() > 0.01)
        {
            bad.push(format!(
                "#{id}: painted {got:?}, bounding_rect {bounds:?}, Chrome {:?}",
                (x, y, w, h)
            ));
        }
    }
    assert!(bad.is_empty(), "painted boxes:\n{}", bad.join("\n"));
}

// ── #2223: a box that does not scroll counts its descendants' overflow ──────
//
// `scroll_width` stopped at the box's children: a `260px` box holding a
// `260px` child holding a `460px` grandchild answered 260, Chrome 460.
// Chrome 158's `scrollWidth` / `scrollHeight` for each container (260x30,
// no padding, nothing scrolls). Each case overflows past the box, so
// Chrome's answer (at least the client size) is the extent rinch reports.

const NESTED: &[(&str, &str, f64, Option<f64>)] = &[
    // A grandchild wider than the box: the issue's 260 against 460.
    (
        "n1",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div>"#,
        460.0,
        None,
    ),
    // Taller: #2054's shape, in a box that does not scroll.
    (
        "n2",
        r#"<div style="width:100px;height:10px"><div style="width:10px;height:80px"></div></div>"#,
        100.0,
        Some(80.0),
    ),
    // An absolute grandchild contained by the box.
    (
        "n3",
        r#"<div style="width:260px;height:10px"><div style="position:absolute;left:300px;top:0;width:100px;height:5px"></div></div>"#,
        400.0,
        None,
    ),
    // ... and one contained above it, which is not its overflow.
    (
        "n4",
        r#"<div style="width:260px;height:10px"><div style="position:absolute;left:300px;top:0;width:100px;height:5px"></div></div>"#,
        260.0,
        None,
    ),
    // A child that clips keeps its grandchild's overflow.
    (
        "n5",
        r#"<div style="width:260px;height:10px;overflow:hidden"><div style="width:460px;height:50px"></div></div>"#,
        260.0,
        None,
    ),
    // A transformed grandchild counts where it is drawn.
    (
        "n6",
        r#"<div style="width:260px;height:10px"><div style="width:100px;height:5px;transform:translateX(300px)"></div></div>"#,
        400.0,
        None,
    ),
    // A right-to-left box: the grandchild overflows to the left.
    (
        "n7",
        r#"<div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div>"#,
        460.0,
        None,
    ),
    // A child's `nowrap` line wider than the child.
    (
        "n8",
        r#"<div style="width:100px;white-space:nowrap;font-size:0;height:10px"><span style="display:inline-block;width:200px;height:5px"></span><span style="display:inline-block;width:200px;height:5px"></span></div>"#,
        400.0,
        None,
    ),
    // A grandchild's border box, not its margins.
    (
        "n9",
        r#"<div style="width:260px;height:10px"><div style="width:300px;margin-left:100px;margin-right:200px;height:5px;margin-bottom:100px"></div></div>"#,
        400.0,
        None,
    ),
    // Offsets down the chain add, a relative one included.
    (
        "n10",
        r#"<div style="width:200px;height:10px;margin-left:20px"><div style="width:200px;height:10px;margin-left:30px"><div style="width:300px;height:5px;position:relative;left:15px"></div></div></div>"#,
        365.0,
        None,
    ),
    // An absolute child contained above the box (a modal's layer), and a
    // fixed one: neither they nor their content are the box's overflow.
    (
        "n11",
        r#"<div style="width:260px;height:10px"></div><div style="position:absolute;left:0;top:0;width:100px;height:5px"><div style="width:600px;height:5px"></div></div>"#,
        260.0,
        None,
    ),
    (
        "n12",
        r#"<div style="width:260px;height:10px"></div><div style="position:fixed;left:0;top:0;width:100px;height:5px"><div style="width:600px;height:5px"></div></div>"#,
        260.0,
        None,
    ),
];

#[test]
fn a_box_that_does_not_scroll_counts_its_descendants_overflow() {
    let mut html = String::new();
    for &(id, inner, _, _) in NESTED {
        let extra = match id {
            "n3" => "position:relative;",
            "n7" => "direction:rtl;",
            _ => "",
        };
        html.push_str(&format!(
            r#"<div id="{id}" style="width:260px;height:30px;{extra}">{inner}</div>"#
        ));
    }
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, &html);
    d.resolve_layout(800.0, 600.0);
    let mut bad = Vec::new();
    for &(id, _, w, h) in NESTED {
        let n = d.query_selector(&format!("#{id}")).expect(id);
        let (sw, sh) = (d.scroll_width(n), d.scroll_height(n));
        if (sw - w).abs() > 0.01 || h.is_some_and(|h| (sh - h).abs() > 0.01) {
            bad.push(format!(
                "#{id}: scroll_width {sw} scroll_height {sh}, Chrome {w} {h:?}"
            ));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

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
/// moves nothing under the scroller keeps it, and one that moves a box
/// under it walks again and answers the new range.
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
    let before = walks(&d);
    assert_eq!(d.scroll_width(s), 460.0);
    assert_eq!(walks(&d), before, "nothing moved under the scroller");

    // A colour on the grandchild (a hover): pushed paint-dirty, nothing the
    // walk read changed.
    let g = d.query_selector("#g").expect("#g");
    d.set_style(g, "background-color", "rgb(200, 0, 0)");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(d.scroll_width(s), 460.0);
    assert_eq!(walks(&d), before, "a colour keeps the walk");

    // One that moves the grandchild.
    d.set_attribute(g, "style", "width:560px;height:5px");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(d.scroll_width(s), 560.0, "the new range");
    assert_eq!(walks(&d), before + 1, "walked again");
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

// ── The kept walk against a fresh document (#2054 review) ─────────────────
//
// Each scenario primes the scroller's kept walk (a read, and a paint
// consuming the tree), makes its changes with a layout and a read after
// each, and reads the range in three orders; each must equal what a fresh
// document given the same changes, never read in between, answers.

type Step = fn(&mut RinchDocument);

fn kept_doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(800.0, 600.0);
    d
}

fn range_of(d: &RinchDocument) -> (f64, f64) {
    let s = d.query_selector("#s").expect("#s");
    (d.scroll_width(s), d.scroll_height(s))
}

fn by_id(d: &RinchDocument, id: &str) -> rinch_core::dom::NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

fn kept_matches_fresh(what: &str, html: &str, steps: &[Step]) {
    let fresh = {
        let mut d = kept_doc(html);
        for step in steps {
            step(&mut d);
            d.resolve_layout(800.0, 600.0);
        }
        range_of(&d)
    };
    for order in 0..3 {
        let mut d = kept_doc(html);
        let _ = range_of(&d);
        d.tree.consume_paint_dirty();
        let _ = range_of(&d);
        for step in steps {
            step(&mut d);
            d.resolve_layout(800.0, 600.0);
            if order != 1 {
                let _ = range_of(&d);
            }
        }
        let got = match order {
            0 => range_of(&d),
            1 => {
                d.tree.consume_paint_dirty();
                range_of(&d)
            }
            _ => {
                d.tree.consume_paint_dirty();
                let _ = range_of(&d);
                range_of(&d)
            }
        };
        assert_eq!(got, fresh, "{what}, read order {order}");
    }
}

const TALL: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="c" style="height:50px;width:100px"><div id="g" style="height:300px;width:150px"></div></div></div>"#;
const TALL_HIDDEN: &str = r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div id="c" style="height:50px;width:100px;overflow:hidden"><div id="g" style="height:300px;width:150px"></div></div></div>"#;

#[test]
fn a_box_made_to_clip_drops_the_kept_walk() {
    kept_matches_fresh(
        "overflow: hidden",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "hidden");
        }],
    );
    kept_matches_fresh(
        "overflow: clip",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "clip");
        }],
    );
}

#[test]
fn a_box_that_stops_clipping_drops_the_kept_walk() {
    kept_matches_fresh(
        "hidden to visible",
        TALL_HIDDEN,
        &[|d| {
            let c = by_id(d, "c");
            d.set_style(c, "overflow", "visible");
        }],
    );
}

/// A change and its undoing between two paints: the walk taken in between
/// must not stand.
#[test]
fn a_change_and_back_before_a_paint_drops_the_kept_walk() {
    kept_matches_fresh(
        "900 then 300",
        TALL,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "width", "900px");
            },
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "width", "150px");
            },
        ],
    );
}

/// More of the family: a grandchild's lines re-shaped wider, a transform
/// given with no layout, a box taken out of the display, a box added under
/// one the walk entered, and a block collapsed to an estimated height (its
/// content, laid out before, is not entered).
#[test]
fn other_changes_under_the_scroller_drop_the_kept_walk() {
    kept_matches_fresh(
        "lines re-shaped",
        r#"<div id="s" style="width:200px;height:100px;overflow:auto"><div style="width:100px;height:20px"><div id="p" style="width:100px;white-space:nowrap;font-size:0">x</div></div></div>"#,
        &[|d| {
            let p = by_id(d, "p");
            d.set_inner_html(
                p,
                r#"<span style="display:inline-block;width:400px;height:5px"></span>"#,
            );
        }],
    );
    kept_matches_fresh(
        "a transform given",
        TALL,
        &[|d| {
            let g = by_id(d, "g");
            d.set_style(g, "transform", "translateX(300px)");
        }],
    );
    kept_matches_fresh(
        "display: none",
        TALL,
        &[|d| {
            let g = by_id(d, "g");
            d.set_style(g, "display", "none");
        }],
    );
    kept_matches_fresh(
        "a box added",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            let n = d.create_element("div");
            d.set_attribute(n, "style", "width:700px;height:5px");
            d.append_child(c, n);
        }],
    );
    kept_matches_fresh(
        "a block collapsed",
        TALL,
        &[|d| {
            let c = by_id(d, "c");
            d.set_estimated_height(c.0, Some(50.0));
        }],
    );
}

/// A block collapsed by virtualization stands for its content at its
/// estimate: what it held, laid out before, is not in the range.
#[test]
fn a_collapsed_blocks_content_is_not_in_the_range() {
    let mut d = kept_doc(TALL);
    assert_eq!(range_of(&d).1, 300.0, "control: the grandchild is reached");
    let c = by_id(&d, "c");
    d.set_estimated_height(c.0, Some(50.0));
    d.resolve_layout(800.0, 600.0);
    assert_eq!(range_of(&d).1, 50.0, "the estimate alone");
}

// ── A change inside the scroller's first box keeps the walk (#2961) ───────
//
// The perf review of #2961: a page of paragraphs in the viewport's scroller
// walked every box again on each layout that resized one of them
// (`shell::relayout_beside_text`, +9%). What is inside a top box's border
// box adds nothing to the range, so a change there keeps the walk.

const ROWS: &str = r#"<div id="s" style="width:300px;height:100px;overflow:auto"><div id="t" style="position:relative;width:300px;height:400px"><div id="wide" style="height:10px;width:600px"></div><div id="a" style="width:200px;height:20px"></div><div id="b" style="width:100px;height:20px"></div><div id="gauge" style="position:absolute;left:150px;top:200px;width:100px;height:10px"></div></div></div>"#;

/// A row resized and an absolute box resized, each inside the top box:
/// the range is read from the kept walk, and is right. (A box counted in
/// the range that moves walks again: the wide row is first, so the rows
/// growing below it move nothing counted.)
#[test]
fn a_change_inside_the_top_box_keeps_the_walk() {
    let mut d = kept_doc(ROWS);
    let s = by_id(&d, "s");
    let walks = |d: &RinchDocument| rinch_dom::testing::deep_overflow_walks(&d.tree);
    assert_eq!(
        range_of(&d),
        (600.0, 400.0),
        "control: the wide row is reached"
    );
    d.tree.consume_paint_dirty();
    let _ = range_of(&d);
    let gauge = by_id(&d, "gauge");
    d.set_style(gauge, "width", "111px");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(range_of(&d), (600.0, 400.0));
    let _ = rinch_dom::paint::scrollbar::content_extents(&d.tree, s.0);
    assert_eq!(
        walks(&d),
        before,
        "the absolute box moved inside the top box"
    );
    d.tree.consume_paint_dirty();
    let a = by_id(&d, "a");
    d.set_style(a, "height", "60px");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(range_of(&d), (600.0, 400.0));
    assert_eq!(walks(&d), before, "the rows moved inside the top box");
    // Control: a row made to reach past the top box walks again.
    let b = by_id(&d, "b");
    d.set_style(b, "width", "700px");
    d.resolve_layout(800.0, 600.0);
    let before = walks(&d);
    assert_eq!(range_of(&d), (700.0, 400.0));
    assert_eq!(walks(&d), before + 1, "walked again");
}

/// Kept against fresh, for the changes a kept walk now stands through or
/// must drop on: a box moved inside, a counted box brought back inside, a
/// box carrying a child that reaches
/// past the top box, the top box shrunk under its content, and a child
/// grown inside before its parent moves (the parent's record must hold the
/// child's new extent).
#[test]
fn changes_inside_the_top_box_match_a_fresh_document() {
    kept_matches_fresh(
        "rows resized inside",
        ROWS,
        &[
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "height", "60px");
            },
            |d| {
                let g = by_id(d, "gauge");
                d.set_style(g, "top", "300px");
            },
        ],
    );
    kept_matches_fresh(
        "a counted row comes back inside",
        ROWS,
        &[|d| {
            let w = by_id(d, "wide");
            d.set_style(w, "width", "200px");
        }],
    );
    const CARRIER: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:100px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:40px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a parent moves its child past the top box",
        CARRIER,
        &[|d| {
            let a = by_id(d, "a");
            d.set_style(a, "margin-top", "70px");
        }],
    );
    kept_matches_fresh(
        "the top box shrinks under its content",
        CARRIER,
        &[|d| {
            let t = by_id(d, "t");
            d.set_style(t, "height", "30px");
        }],
    );
    kept_matches_fresh(
        "the top box grows",
        CARRIER,
        &[|d| {
            let t = by_id(d, "t");
            d.set_style(t, "height", "300px");
        }],
    );
    const GROWS: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:200px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a child grows inside, then its parent moves",
        GROWS,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "height", "150px");
            },
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "margin-top", "80px");
            },
        ],
    );
}

/// A box moved inside the top box adds what it now holds to the record of
/// each box above it (the ancestor join in `moved_inside`): `g` is moved past
/// its parent `a`, still inside `t`, and then `a` moves, taking `g` past `t`.
/// Without the join `a`'s record still holds `g` where it was, so the second
/// move is judged inside and the kept range stays at `t`'s 150.
#[test]
fn a_child_moved_past_its_parent_then_the_parent_moves() {
    const NESTED: &str = r#"<div id="s" style="width:100px;height:50px;overflow:auto"><div id="t" style="width:150px;height:200px"><div id="a" style="width:100px;height:10px"><div id="g" style="width:50px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a child moved past its parent, then the parent moves",
        NESTED,
        &[
            |d| {
                let g = by_id(d, "g");
                d.set_style(g, "margin-left", "90px");
            },
            |d| {
                let a = by_id(d, "a");
                d.set_style(a, "margin-left", "30px");
            },
        ],
    );
}

/// Padding moves what a box lays out from its content box — its lines, and
/// the atomic inlines its inline formatting context places — without moving
/// or resizing the box, so the kept walk's record of the box must say where
/// its content starts. Here `r` keeps its 280px border box and its
/// inline-block keeps its layout (relative to `r`'s content box), but the
/// padding takes the inline-block to 150..350, past `t`.
const PADDED: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:300px;height:100px"><div id="r" style="box-sizing:border-box;width:280px"><div style="display:inline-block;width:200px;height:20px;vertical-align:top"></div></div></div></div>"#;

#[test]
fn padding_that_moves_an_inline_block_drops_the_kept_walk() {
    kept_matches_fresh(
        "padding moves an inline-block",
        PADDED,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "padding-left", "150px");
        }],
    );
}

/// The same for a line: `lines_span` measures from the content origin.
#[test]
fn padding_that_moves_a_line_drops_the_kept_walk() {
    const TEXT: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:300px;height:100px"><div id="r" style="box-sizing:border-box;width:280px;white-space:nowrap;font-size:16px;line-height:20px">wwwwwwwwwwww wwwwwwwwwwww</div></div></div>"#;
    kept_matches_fresh(
        "padding moves a line",
        TEXT,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "padding-left", "150px");
        }],
    );
}

/// And for a border, the other half of the content origin.
#[test]
fn a_border_that_moves_an_inline_block_drops_the_kept_walk() {
    kept_matches_fresh(
        "a border moves an inline-block",
        PADDED,
        &[|d| {
            let r = by_id(d, "r");
            d.set_style(r, "border-left", "150px solid black");
        }],
    );
}

/// A `filter` makes its box the containing block of an absolute box below
/// it (#2719), which is then the container's overflow; taken off, the box's
/// containing block is above the container, and it is not. The absolute box
/// has no insets, so it stays where it is and nothing of it changes: only
/// the filtered box's record can say so. (Taking the filter off is the case
/// the walk alone answers: while it is on, the container's own measure lists
/// the absolute box too, `NodeTree::overflow_absolutes`.)
#[test]
fn a_filter_taken_off_an_absolute_boxs_containing_block_drops_the_kept_walk() {
    const ABS: &str = r#"<div id="s" style="width:200px;height:50px;overflow:auto"><div id="t" style="width:150px;height:100px"><div id="a" style="width:100px;height:10px;filter:blur(0px)"><div style="position:absolute;width:400px;height:10px"></div></div></div></div>"#;
    kept_matches_fresh(
        "a filter taken off an absolute box's containing block",
        ABS,
        &[|d| {
            let a = by_id(d, "a");
            d.set_style(a, "filter", "none");
        }],
    );
}
