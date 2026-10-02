//! #1236 — the layout the Taffy 0.12 → 0.14 bump DID change. Found by review
//! of PR #1253 with a whole-suite layout differential (every node of every
//! `resolve_layout` in the rinch-dom, rinch, rinch-components and
//! rinch-editor-view test suites, plus every ui-zoo section) and a 114-case
//! probe corpus: every difference is one of the shapes below, and every one
//! moved to Chrome 153's answer (each expected number is Chrome's, measured
//! with the same markup, `body { margin: 0 }`, standards mode). Against Taffy
//! 0.12 the five layout fixtures fail; the last fixture (`patch_dim`'s
//! indefinite-basis arm) passes on 0.12 too and is here as a mutant pin.
//!
//! One component moves with these shapes: a horizontal `Tabs` nested inside a
//! vertical `Tabs` (the shape `css_hook_760_tests` documents) gets an inner
//! panel 44px wide where 0.12 gave 85 (Chrome 153: 42.9).
//!
//! No text is laid out: every box has a declared size.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

fn lay_out(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `(x, y)` of the element with `id`, relative to its parent's box.
fn at(doc: &RinchDocument, id: &str) -> (f32, f32) {
    let mut stack = vec![doc.body()];
    while let Some(n) = stack.pop() {
        if doc.get_attribute(n, "id").as_deref() == Some(id) {
            let l = &doc.tree.get(n.0).unwrap().layout;
            return (l.x, l.y);
        }
        let node = doc.tree.get(n.0).unwrap();
        stack.extend(node.children.iter().map(|c| NodeId(*c)));
    }
    panic!("no #{id}");
}

/// Auto margins take the free space before `justify-content` sees any
/// (css-flexbox-1 §8.1). Taffy 0.12 centred the line first and then pushed the
/// auto-margin item further right, off the end of a 400px container
/// (`a` 450, `b` 500); Chrome 153: 300, 350.
#[test]
fn an_auto_margin_absorbs_free_space_before_justify_content_center() {
    let d = lay_out(
        r#"<div style="display:flex;justify-content:center;width:400px"><div id="a" style="margin-left:auto;width:50px;height:10px"></div><div id="b" style="width:50px;height:10px"></div></div>"#,
    );
    assert_eq!(at(&d, "a").0, 300.0);
    assert_eq!(at(&d, "b").0, 350.0);
}

/// The same with `flex-end` and a button group's shape — `OK` pushed right by
/// `margin-left: auto`. Taffy 0.12 put `b` at 650 in a 400px box; Chrome 153: 150, 350.
#[test]
fn auto_margins_beat_justify_content_flex_end() {
    let d = lay_out(
        r#"<div style="display:flex;justify-content:flex-end;width:400px"><div id="a" style="margin:0 auto;width:50px;height:10px"></div><div id="b" style="width:50px;height:10px"></div></div>"#,
    );
    assert_eq!(at(&d, "a").0, 150.0);
    assert_eq!(at(&d, "b").0, 350.0);
}

/// Column axis: `margin-top: auto` under `justify-content: center` in a 200px
/// column. Taffy 0.12: `a` at 240 (past the box); Chrome 153: 160.
#[test]
fn an_auto_margin_beats_justify_content_in_a_column() {
    let d = lay_out(
        r#"<div style="display:flex;flex-direction:column;justify-content:center;height:200px"><div id="a" style="margin-top:auto;height:20px"></div><div id="b" style="height:20px"></div></div>"#,
    );
    assert_eq!(at(&d, "a").1, 160.0);
    assert_eq!(at(&d, "b").1, 180.0);
}

/// A last child's bottom margin does not collapse through a parent whose
/// `min-height` made it taller than its content. Taffy 0.12 collapsed it, so
/// the next sibling sat at 130; Chrome 153: 110.
#[test]
fn a_min_height_parent_keeps_its_last_childs_bottom_margin() {
    let d = lay_out(
        r#"<div id="o" style="min-height:100px"><div id="a" style="margin-bottom:30px;height:10px"></div></div><div id="s" style="margin-top:10px;height:10px"></div>"#,
    );
    assert_eq!(at(&d, "s").1, 110.0);
}

/// An empty scroll container's own top and bottom margins collapse through
/// it, and then with the next sibling's. Taffy 0.12 put `c` at -8; Chrome 153: -16.
#[test]
fn an_empty_scroll_containers_margins_collapse_through_it() {
    let d = lay_out(
        r#"<div style="overflow:hidden;width:300px;height:70px"><section id="b" style="margin:-8px;overflow-y:scroll"></section><div id="c" style="height:67px;margin:-6px"></div></div>"#,
    );
    assert_eq!(at(&d, "c").1, -16.0);
}

/// Not a change — a pin for `calc_layout::patch_dim`'s indefinite-basis arm,
/// which #1236 made generic over `Dimension` (size) and `LengthPercentageAuto`
/// (min/max) and which nothing exercised: a mixed `calc()` against an
/// indefinite basis behaves as `auto` (CSS 10.5), for `height` and for
/// `max-height` alike. Mutating that arm to a zero length leaves the whole
/// rinch-dom suite green. Chrome 153: both boxes 30px tall.
#[test]
fn a_mixed_calc_against_an_indefinite_height_is_auto_on_size_and_max_size() {
    let d = lay_out(
        r#"<div><div id="h" style="height:calc(50% + 10px)"><div style="height:30px"></div></div><div id="m" style="max-height:calc(50% + 10px)"><div style="height:30px"></div></div></div>"#,
    );
    for id in ["h", "m"] {
        let mut stack = vec![d.body()];
        let mut found = None;
        while let Some(n) = stack.pop() {
            if d.get_attribute(n, "id").as_deref() == Some(id) {
                found = Some(d.tree.get(n.0).unwrap().layout.height);
            }
            stack.extend(d.tree.get(n.0).unwrap().children.iter().map(|c| NodeId(*c)));
        }
        assert_eq!(found, Some(30.0), "#{id}");
    }
}
