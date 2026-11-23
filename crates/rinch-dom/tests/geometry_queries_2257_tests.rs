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

/// A scroll container keeps the range its bar and the wheel use: its
/// grandchild's overflow is not counted there yet (#2054; Chrome 158 gives
/// 460), so `scroll_width` agrees with the bar rather than with the walk a
/// box that does not scroll takes.
#[test]
fn a_scroll_container_answers_its_bar_range() {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(
        body,
        r#"<div id="s" style="width:260px;height:30px;overflow:auto"><div style="width:260px;height:10px"><div style="width:460px;height:5px"></div></div></div>"#,
    );
    d.resolve_layout(800.0, 600.0);
    let s = d.query_selector("#s").expect("#s");
    assert_eq!(
        d.scroll_width(s),
        rinch_dom::paint::scrollbar::content_extents(&d.tree, s.0).0
    );
    assert_eq!(d.scroll_width(s), 260.0, "#2054 still open");
}
