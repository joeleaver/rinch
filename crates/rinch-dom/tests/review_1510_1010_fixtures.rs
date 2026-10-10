//! Review of #1510 (2026-10-10): two shapes the PR's table does not pin.
//! Chrome numbers are Chrome 155.0.8059.39 (same harness as the PR's 153
//! table: `--headless=new`, standards mode, `* { box-sizing: border-box }`,
//! 800x600, the bundled Inter as `ProbeFace`, `16px/20px`).
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = r#"<div data-m="c" style="position:relative;width:400px;height:300px;padding:5px 0 0 12px;border:3px solid;margin:9px 0 0 14px;font:16px/20px ProbeFace;"#;
const T: &str = r#"<span data-m="t" style="display:inline-block">Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters</span>"#;

fn mount(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn one(doc: &RinchDocument, sel: &str) -> usize {
    query_selector(&doc.tree, sel)[0]
}

/// `(x, y, w, h)` of mark `m` relative to the container's border box.
fn rect(doc: &RinchDocument, m: &str) -> (f32, f32, f32, f32) {
    let pos = |id| {
        let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
        (x as f32, y as f32)
    };
    let (cx, cy) = pos(one(doc, "[data-m=c]"));
    let id = one(doc, &format!("[data-m={m}]"));
    let (x, y) = pos(id);
    let l = doc.tree.get(id).unwrap().layout;
    (x - cx, y - cy, l.width, l.height)
}

/// An auto-width box with no inline insets in a `justify-content: flex-end`
/// flex container (not its containing block). Chrome 155: `a=3,8,242,60` —
/// shrunk to the room between the containing block's padding edge (3) and
/// the flex end (245), and starting at that edge. main: `45,8,200,80` (in
/// the block, wrong width). #1510: `-107,8,352,40` — 110px outside its
/// containing block's padding edge, where an `overflow: hidden` block would
/// cut it off. Same shape with `center`: Chrome `3,8,284,40`, #1510 `-31`.
#[test]
fn a_flex_end_placed_static_box_starts_inside_its_containing_block() {
    for (jc, chrome_w) in [("flex-end", 242.0), ("center", 284.0)] {
        let doc = mount(&format!(
            r#"{C}"><div data-m="p" style="width:200px;height:200px;margin-left:30px;display:flex;justify-content:{jc};"><div data-m="a" style="position:absolute;">{T}</div></div></div>"#
        ));
        let (x, _, w, _) = rect(&doc, "a");
        assert!(
            x >= 3.0 - 0.75,
            "{jc}: the box starts at {x}, left of its containing block's padding edge (3); Chrome 155 3, width {chrome_w} (got {w})"
        );
    }
}

/// Percentage margins on a box shrunk to fit from its static position,
/// resolved against a non-parent containing block: narrowing the block and
/// widening it back must give a fresh layout's answer. #1510: narrowed
/// `225` wide around `224` of content (fresh `224`), widened back `280`
/// around `281` of content (fresh `281`, Chrome 155 `280.92`).
#[test]
fn a_static_box_with_percentage_margins_follows_its_containing_block_incrementally() {
    let html = |w: u32| {
        format!(
            r#"{}"><div style="margin-left:42px"><div data-m="a" style="position:absolute;margin-left:5%;margin-right:10%">{T}</div></div></div>"#,
            C.replace("width:400px", &format!("width:{w}px"))
        )
    };
    let mut doc = mount(&html(400));
    let wide = (rect(&doc, "a"), rect(&doc, "t"));
    let c = NodeId(one(&doc, "[data-m=c]"));
    let style = doc.get_attribute(c, "style").unwrap();
    doc.set_attribute(c, "style", &style.replace("width:400px", "width:333px"));
    doc.resolve_layout(800.0, 600.0);
    let narrowed = (rect(&doc, "a"), rect(&doc, "t"));
    let fresh = mount(&html(333));
    assert_eq!(
        narrowed,
        (rect(&fresh, "a"), rect(&fresh, "t")),
        "narrowed incrementally vs fresh"
    );
    doc.set_attribute(c, "style", &style);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        (rect(&doc, "a"), rect(&doc, "t")),
        wide,
        "widened back vs the first layout"
    );
}
