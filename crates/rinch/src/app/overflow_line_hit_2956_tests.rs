//! #2956: text on a line that overflows its block is drawn, and Chrome 158's
//! `elementFromPoint` answers it: the block (or the inline element holding
//! the text) over the line's glyphs, wherever the line is. rinch answered a
//! node only inside its border box (or an inline element's padding past it,
//! #2647), so a point over the overflowing text found what was behind it.
//!
//! Each case is set in Inter at a declared `font: 16px/20px` in a `400px`
//! host at the origin (`body { margin: 0 }`), and the answers are Chrome 158
//! headless at 1x on the same markup; rinch's answer is the hit test's,
//! refined to the element under the glyphs as a press is delivered
//! (`event_target_at`). `"host"` is the host; `"-"` is the root above it
//! (rinch's body).

use super::{event_target_at, hit_test};
use rinch_core::dom::DomDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// `(x, y, the id Chrome answers)`.
type Probe = (f32, f32, &'static str);

/// `(markup, probes)`.
const CASES: &[(&str, &[Probe])] = &[
    // The second line, below a 20px box: over its glyph, in its leading
    // above the glyph, and past its end.
    (
        "<div id=a style='height:20px'>one<br>two</div>",
        &[(5.0, 30.0, "a"), (5.0, 21.0, "a"), (200.0, 30.0, "-")],
    ),
    // The text is an inline element's: the element answers.
    (
        "<div id=a style='height:20px'>one<br><span id=s>two</span></div>",
        &[(5.0, 30.0, "s")],
    ),
    // Overflowing sideways: a line wider than its 20px box.
    (
        "<div id=a style='width:20px;white-space:nowrap'>abcdefgh</div>",
        &[(40.0, 10.0, "a"), (100.0, 10.0, "host")],
    ),
    // A box that clips draws no overflow, and is not hit there.
    (
        "<div id=a style='height:20px;overflow:hidden'>one<br>two</div>",
        &[(5.0, 30.0, "-")],
    ),
    // `pointer-events: none` text passes the point on.
    (
        "<div id=a style='height:20px;pointer-events:none'>one<br>two</div>",
        &[(5.0, 30.0, "-")],
    ),
    // Overflowing an ancestor too: the prune must not skip the 10px box
    // the line is far outside of.
    (
        "<div id=o style='height:10px'><div id=a style='height:20px'>one<br>two</div></div>",
        &[(5.0, 30.0, "a"), (5.0, 15.0, "a")],
    ),
    // A line no taller than its 30px box, pushed out of it by the padding
    // (box 0..30, line 20..40): the prune must place the lines at the
    // content origin, not compare their size with the box's.
    (
        "<div id=a style='box-sizing:border-box;height:30px;padding-top:20px'>two</div>",
        &[(5.0, 35.0, "a"), (200.0, 35.0, "-")],
    ),
    // A line that starts left of its box (a negative `text-indent`; box
    // 100..200, text from 40): the prune must take the lines' own bounds,
    // not `0..width` from the content origin.
    (
        "<div id=b style='margin-left:100px;text-indent:-60px;height:20px;width:100px'>abcdef</div>",
        &[(45.0, 10.0, "b"), (75.0, 10.0, "b"), (30.0, 10.0, "host")],
    ),
    // An rtl `nowrap` line wider than its box is aligned to the box's right
    // edge and overflows to the left (box 200..240).
    (
        "<div id=c style='direction:rtl;width:40px;margin-left:200px;white-space:nowrap'>abcdefghij</div>",
        &[
            (190.0, 10.0, "c"),
            (170.0, 10.0, "c"),
            (100.0, 10.0, "host"),
        ],
    ),
    // A clipping root the prune never skips (a stacking context holding a
    // positioned box): its hidden second line is still not hit.
    (
        "<div id=a style='height:20px;overflow:hidden;position:relative;z-index:0'>one<br>two<span id=p style='position:absolute'>x</span></div>",
        &[(5.0, 30.0, "-"), (5.0, 10.0, "a")],
    ),
    // Clipping one axis (`overflow-x: clip`) leaves the other's overflow
    // drawn, and hit, also past an ancestor; the clipped axis's is neither.
    (
        "<div id=a style='height:20px;overflow-x:clip'>one<br>two</div>",
        &[(5.0, 30.0, "a"), (200.0, 30.0, "-")],
    ),
    (
        "<div id=o style='height:10px'><div id=a style='height:20px;overflow-x:clip'>one<br>two</div></div>",
        &[(5.0, 30.0, "a")],
    ),
    (
        "<div id=a style='width:20px;white-space:nowrap;overflow-x:clip'>abcdefgh</div>",
        &[(40.0, 10.0, "host")],
    ),
    // `overflow-y: clip`, the same the other way round.
    (
        "<div id=a style='width:20px;white-space:nowrap;overflow-y:clip'>abcdefgh</div>",
        &[(40.0, 10.0, "a"), (100.0, 10.0, "host")],
    ),
    (
        "<div id=a style='height:20px;overflow-y:clip'>one<br>two</div>",
        &[(5.0, 30.0, "-")],
    ),
];

fn document(markup: &str) -> (rinch_dom::RinchDocument, u64) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = rinch_dom::RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css("body { margin: 0; font: 16px/20px ProbeFace; }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.set_attribute(host, "style", "width: 400px");
    doc.append_child(body, host);
    doc.set_inner_html(host, markup);
    doc.resolve_layout(800.0, 600.0);
    (doc, host.0)
}

/// The element a press at `(x, y)` reaches, as Chrome's `elementFromPoint`
/// names it: the hit, and inside an IFC root the element whose text is under
/// the point (`event_target_at`, as the click path delivers it).
fn element_at(doc: &rinch_dom::RinchDocument, x: f32, y: f32) -> Option<u64> {
    let hit = hit_test(&doc.tree, x, y)?;
    let Some(target) = event_target_at(&doc.tree, hit, x, y) else {
        return Some(hit);
    };
    let node = doc.tree.get(target)?;
    Some(if node.is_text() { node.parent? } else { target })
}

fn name(doc: &rinch_dom::RinchDocument, host: u64, hit: Option<u64>) -> String {
    match hit {
        Some(h) if h == host => "host".into(),
        Some(h) if h == doc.tree.body_id => "-".into(),
        None => "-".into(),
        Some(h) => doc
            .tree
            .get(h)
            .and_then(|n| n.attributes.get("id").cloned())
            .unwrap_or_else(|| format!("<node {h}>")),
    }
}

/// Every case against Chrome 158.
///
/// Mutants: the old bounds test (border box or inline padding only) — every
/// `a`/`s` point over an overflowing line answers what is behind it;
/// leaving the lines out of `flow_extent` — the same points are pruned
/// before the bounds test is reached; the lines' extent taken as `0..width`
/// from the content origin — the negative `text-indent` and rtl points are
/// pruned; accepting a line in a box that clips (the guard dropped) — the
/// stacking context's hidden line answers `a`; the guard on both axes at
/// once — the `overflow-x: clip` box's line below it answers what is behind
/// it.
#[test]
fn text_on_an_overflowing_line_is_hit_as_chrome_158_hits_it() {
    let mut wrong = Vec::new();
    for &(markup, probes) in CASES {
        let (doc, host) = document(markup);
        for &(x, y, want) in probes {
            let got = name(&doc, host, element_at(&doc, x, y));
            if got != want {
                wrong.push(format!(
                    "{markup}\n  ({x}, {y}): rinch {got:?}, Chrome {want:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
