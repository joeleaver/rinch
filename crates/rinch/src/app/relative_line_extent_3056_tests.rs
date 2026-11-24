//! #3056: text that a `position: relative` inline element moves past its
//! block's box is hit where it is drawn (#2346, #2956), and the prune must
//! reach it there. `flow_extent` held an IFC root's lines where they were
//! laid out, not where the relative offset drew them, so a point on the
//! moved text outside every box (here below a 10px parent and the 20px
//! block) was skipped before the walk got to the root: the hit test answered
//! what was behind the text. `random_documents_hit_identically` (seed 8)
//! found it, as the pruned and unpruned walks disagreeing.
//!
//! Each case is set in Inter at a declared `font: 16px/20px` in a `400px`
//! host at the origin (`body { margin: 0 }`). `"host"` is the host; `"-"`
//! is the root above it (rinch's body).

use super::{event_target_at, hit_test};
use rinch_core::dom::DomDocument;

const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// `(x, y, the id that answers)`.
type Probe = (f32, f32, &'static str);

/// `(markup, probes)`. Each span holds a block (an empty `div`), so it is
/// split around it (block-in-inline): its text is in an anonymous block's
/// lines, which that block draws moved, and the span has no entry of its own
/// in the stacking sequence (`paints_in_own_entry` is false) — the walk
/// into the anonymous block is the only way to the text. The text is drawn
/// 40px lower (`top: 40px`, lines `40..60`) or 120px further right
/// (`left: 120px`) than it was laid out, outside the 10px parent `o`, the
/// block `a` and the anonymous block alike.
const CASES: &[(&str, &[Probe])] = &[
    (
        "<div id=o style='height:10px'><div id=a>\
         <span id=s style='position:relative;top:40px'>two<div></div></span></div></div>",
        &[(5.0, 50.0, "s"), (200.0, 50.0, "-"), (5.0, 70.0, "-")],
    ),
    (
        "<div id=o style='height:10px;width:60px'><div id=a style='width:60px'>\
         <span id=s style='position:relative;left:120px'>two<div></div></span></div></div>",
        &[(125.0, 5.0, "s"), (300.0, 5.0, "host")],
    ),
    // Moved up and left, past the box's top-left corner, inside a block
    // pushed down and right so the moved text is still on the page.
    (
        "<div id=o style='height:10px;margin:100px 0 0 150px'><div id=a>\
         <span id=s style='position:relative;left:-120px;top:-60px'>two<div></div></span></div></div>",
        &[(35.0, 50.0, "s"), (35.0, 105.0, "host")],
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

/// The element a press at `(x, y)` reaches: the hit, refined inside an IFC
/// root to the element whose text is under the point (`event_target_at`).
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

/// Mutant: `flow_extent`'s lines left where they were laid out (the code
/// before this fix) — every `s` point is pruned and answers `host` or `-`.
/// The `host`/`-`/`o` points are the other side: the extent may not grow
/// past where the text is drawn into a box that is not there.
#[test]
fn text_moved_by_a_relative_inline_is_hit_past_every_box() {
    let mut wrong = Vec::new();
    for &(markup, probes) in CASES {
        let (doc, host) = document(markup);
        for &(x, y, want) in probes {
            let got = name(&doc, host, element_at(&doc, x, y));
            if got != want {
                wrong.push(format!(
                    "{markup}\n  ({x}, {y}): rinch {got:?}, want {want:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
