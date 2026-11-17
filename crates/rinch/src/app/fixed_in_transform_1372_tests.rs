//! #1372: a `position: fixed` box inside a transformed ancestor is clipped,
//! stacked, painted and hit as a box contained by that ancestor — and hit
//! testing answers where paint draws. Every expected answer is Chrome 151's
//! `document.elementFromPoint` on the same markup (Playwright 1.62's
//! `chrome-headless-shell`, `body { margin: 0 }`, `* { box-sizing:
//! border-box }`, 800x600): `a` is the fixed box, painted green, and nothing
//! else in a case is green, so each point is checked twice — the hit test
//! names the box exactly where the software painter draws green.

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

/// (name, markup, the element scrolled by 47px or `""`, points: (x, y, the
/// `data-m` Chrome hits there, `-` for anything without one)).
type Case = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(f32, f32, &'static str)],
);

const GREEN: &str = "background:rgb(0,200,0)";

const CASES: &[Case] = &[
    // The transformed ancestor clips: past its box the fixed box is cut away.
    (
        "h01_transformed_clipper",
        r#"<div data-m="t" style="transform:translate(0);overflow:hidden;margin:13px 0 0 41px;width:133px;height:111px"><div data-m="a" style="position:fixed;left:101px;top:91px;width:77px;height:53px;{G}"></div></div>"#,
        "",
        &[
            (150.0, 110.0, "a"),
            (190.0, 130.0, "-"),
            (200.0, 150.0, "-"),
        ],
    ),
    // A clipper above the transformed ancestor clips the ancestor, and so the
    // box it contains.
    (
        "h02_clipper_above",
        r#"<div data-m="c" style="overflow:hidden;width:151px;height:121px;margin:7px"><div data-m="t" style="transform:translate(0);margin:13px 0 0 41px;width:83px;height:61px"><div data-m="a" style="position:fixed;left:61px;top:41px;width:77px;height:53px;{G}"></div></div></div>"#,
        "",
        &[(120.0, 70.0, "a"), (150.0, 120.0, "c"), (170.0, 130.0, "-")],
    ),
    // A clipper between the box and its containing block does not clip it…
    (
        "h03_clipper_between",
        r#"<div data-m="t" style="transform:translate(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="c" style="overflow:hidden;width:51px;height:31px"><div data-m="a" style="position:fixed;left:31px;top:21px;width:77px;height:53px;{G}"></div></div></div>"#,
        "",
        &[(80.0, 40.0, "a"), (130.0, 70.0, "a"), (100.0, 80.0, "a")],
    ),
    // …not even a positioned one, which an absolute box would stop at.
    (
        "h04_relative_clipper_between",
        r#"<div data-m="t" style="transform:translate(0);margin:13px 0 0 41px;width:333px;height:211px"><div data-m="c" style="position:relative;overflow:hidden;width:51px;height:31px"><div data-m="a" style="position:fixed;left:31px;top:21px;width:77px;height:53px;{G}"></div></div></div>"#,
        "",
        &[(80.0, 40.0, "a"), (130.0, 70.0, "a"), (100.0, 80.0, "a")],
    ),
    // The ancestor's translate carries it.
    (
        "h05_translate",
        r#"<div data-m="t" style="transform:translate(201px,103px);width:401px;height:301px"><div data-m="a" style="position:fixed;left:51px;top:61px;width:101px;height:41px;z-index:5;{G}"></div></div>"#,
        "",
        &[(60.0, 70.0, "-"), (260.0, 170.0, "a"), (350.0, 200.0, "a")],
    ),
    // Its `z-index` is local to the transformed ancestor's stacking context,
    // which a later `z-index: 1` sibling paints over.
    (
        "h06_stacking",
        r#"<div data-m="t" style="transform:translate(0);width:201px;height:101px"><div data-m="a" style="position:fixed;left:11px;top:13px;width:101px;height:41px;z-index:5;{G}"></div></div><div data-m="s" style="position:absolute;z-index:1;left:61px;top:23px;width:101px;height:41px;background:rgb(0,0,200)"></div>"#,
        "",
        &[(30.0, 30.0, "a"), (80.0, 40.0, "s"), (140.0, 50.0, "s")],
    ),
    // A scale grows its box.
    (
        "h07_scale",
        r#"<div data-m="t" style="transform:scale(1.5);transform-origin:0 0;margin:13px 0 0 41px;width:133px;height:111px"><div data-m="a" style="position:fixed;left:11px;top:9px;width:57px;height:23px;{G}"></div></div>"#,
        "",
        &[(60.0, 30.0, "a"), (130.0, 55.0, "a"), (145.0, 58.0, "t")],
    ),
    // Scrolled with its transformed scroller's content.
    (
        "h08_scroller",
        r#"<div id="s" data-m="t" style="transform:translate(0);overflow:auto;margin:13px 0 0 41px;width:333px;height:211px"><div data-m="i" style="height:900px"></div><div data-m="a" style="position:fixed;left:11px;top:79px;width:57px;height:23px;{G}"></div></div>"#,
        "s",
        &[(60.0, 50.0, "a"), (60.0, 100.0, "i")],
    ),
    (
        "h09_rotate",
        r#"<div data-m="t" style="transform:rotate(90deg);transform-origin:0 0;margin-left:301px;width:201px;height:101px"><div data-m="a" style="position:fixed;left:11px;top:13px;width:61px;height:21px;{G}"></div></div>"#,
        "",
        &[(275.0, 20.0, "a"), (280.0, 70.0, "a"), (30.0, 20.0, "-")],
    ),
];

fn lay_out(html: &str, scrolled: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "* { box-sizing: border-box; } html, body { margin: 0; background: rgb(255, 255, 255); }",
    );
    let body = doc.body();
    doc.set_inner_html(body, &html.replace("{G}", GREEN));
    doc.resolve_layout(800.0, 600.0);
    if !scrolled.is_empty() {
        let s = query_selector(&doc.tree, &format!("#{scrolled}"))[0];
        doc.set_scroll_top(NodeId(s), 47.0);
        assert_eq!(doc.scroll_top(NodeId(s)), 47.0);
        doc.resolve_layout(800.0, 600.0);
    }
    doc
}

/// The `data-m` of `id` or of its nearest ancestor that has one below the
/// body, `-` for none — what `elementFromPoint` names, read up to the marked
/// box (Chrome's answer is the element itself; an unmarked hit is `-`).
fn mark(doc: &RinchDocument, id: Option<u64>) -> String {
    let Some(id) = id else {
        return "-".into();
    };
    doc.tree
        .get(id)
        .and_then(|n| n.attributes.get("data-m").map(|v| v.to_string()))
        .unwrap_or_else(|| "-".into())
}

fn painted_green(doc: &mut RinchDocument, points: &[(f32, f32, &str)]) -> Vec<bool> {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let d = painter.pixels();
    points
        .iter()
        .map(|&(x, y, _)| {
            let i = ((y as u32 * painter.width() + x as u32) * 4) as usize;
            [d[i], d[i + 1], d[i + 2]] == [0, 200, 0]
        })
        .collect()
}

#[test]
fn contained_fixed_boxes_are_hit_and_painted_as_chrome_151_does() {
    let mut bad = Vec::new();
    let mut checked = 0;
    for (name, html, scrolled, points) in CASES {
        let mut doc = lay_out(html, scrolled);
        let green = painted_green(&mut doc, points);
        for (&(x, y, want), green) in points.iter().zip(green) {
            checked += 1;
            let got = mark(&doc, hit_test(&doc.tree, x, y));
            if got != want {
                bad.push(format!("{name} ({x}, {y}): chrome[{want}] hit[{got}]"));
            }
            if green != (want == "a") {
                bad.push(format!(
                    "{name} ({x}, {y}): chrome[{want}] painted green: {green}"
                ));
            }
        }
    }
    assert!(checked >= 25, "the table is being read: {checked}");
    assert!(bad.is_empty(), "{bad:#?}");
}
