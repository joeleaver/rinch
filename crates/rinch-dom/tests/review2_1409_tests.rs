//! Round-2 review of PR #1409 (#386): staleness probes for the redesigned
//! bookkeeping. RED at fad0fc09: `p1_…` (finding 1), `c1_…` and `c3_…`
//! (finding 2); green with reports/review2-1409-1005-experimental-fix.patch.
//! `q1_…` prints the cost in finding 3 (run with --nocapture). Each scenario mutates a laid-out document, resolves at the
//! SAME viewport (what a frame does), and compares every `[data-m]` box with
//! a fresh document built from the final markup (twin oracle).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;
use std::collections::BTreeMap;

const VP: (f32, f32) = (800.0, 600.0);

fn build(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}
fn m(doc: &RinchDocument, name: &str) -> NodeId {
    NodeId(one(doc, &format!("[data-m={name}]")))
}

fn rects(doc: &RinchDocument) -> BTreeMap<String, [f32; 4]> {
    let mut out = BTreeMap::new();
    for id in query_selector(&doc.tree, "[data-m]") {
        let n = doc.tree.get(id).unwrap();
        let name = n.attributes.get("data-m").unwrap().to_string();
        let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
        out.insert(name, [x as f32, y as f32, n.layout.width, n.layout.height]);
    }
    out
}

/// Compare with a fresh layout of `final_html`. Returns the list of diffs.
fn diff(doc: &RinchDocument, final_html: &str) -> Vec<String> {
    let fresh = build(final_html);
    let (got, want) = (rects(doc), rects(&fresh));
    let mut out = Vec::new();
    for (k, w) in &want {
        match got.get(k) {
            Some(g) if g.iter().zip(w).all(|(a, b)| (a - b).abs() < 0.01) => {}
            Some(g) => out.push(format!("{k}: got {g:?}, fresh {w:?}")),
            None => out.push(format!("{k}: missing")),
        }
    }
    out
}

#[track_caller]
fn twin(doc: &RinchDocument, final_html: &str, what: &str) {
    let d = diff(doc, final_html);
    assert!(
        d.is_empty(),
        "{what}: stale against a fresh layout:\n  {}",
        d.join("\n  ")
    );
}

const CBS: &str = "width: 400px; height: 300px; margin: 13px 0 0 17px;";
const MID: &str = "width: 300px; height: 200px; margin: 0 0 0 30px;";
const BOXES: &str = r#"<div data-m="a" style="position: absolute; right: 5px; bottom: 7px; width: 40px; height: 20px"></div><div data-m="b" style="position: absolute; inset: 3px"><div data-m="bc" style="width: 50%; height: 50%"></div></div><div data-m="s" style="position: absolute; width: 10px; height: 10px"></div>"#;

fn page(cb_style: &str, mid_style: &str, inner: &str) -> String {
    format!(
        r#"<div data-m="cb" style="{CBS}{cb_style}"><div style="height: 20px"></div><div data-m="mid" style="{MID}{mid_style}">{inner}</div></div><div data-m="cb2" style="{CBS}position: relative"><div data-m="mid2" style="{MID}"></div></div>"#
    )
}

fn frame(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
}

// ---- 1. membership ---------------------------------------------------

#[test]
fn a1_ancestor_gains_and_loses_position_transform_contents() {
    for (name, on) in [
        ("position", "position: relative;"),
        ("transform", "transform: translateX(0);"),
        ("transform-nz", "transform: translate(4px, 6px);"),
    ] {
        let mut doc = build(&page("", "", BOXES));
        let cb = m(&doc, "cb");
        doc.set_attribute(cb, "style", &format!("{CBS}{on}"));
        frame(&mut doc);
        twin(&doc, &page(on, "", BOXES), &format!("gain {name}"));
        doc.set_attribute(cb, "style", CBS);
        frame(&mut doc);
        twin(&doc, &page("", "", BOXES), &format!("lose {name}"));
        // through set_style
        let (p, v) = on.trim_end_matches(';').split_once(": ").unwrap();
        doc.set_style(cb, p, v);
        frame(&mut doc);
        twin(
            &doc,
            &page(on, "", BOXES),
            &format!("set_style gain {name}"),
        );
    }
    // the cb becomes display: contents (stops establishing) and back
    let on = "position: relative;";
    let mut doc = build(&page(on, "", BOXES));
    let cb = m(&doc, "cb");
    doc.set_attribute(cb, "style", &format!("{CBS}{on}display: contents;"));
    frame(&mut doc);
    twin(
        &doc,
        &page(&format!("{on}display: contents;"), "", BOXES),
        "cb contents",
    );
    doc.set_attribute(cb, "style", &format!("{CBS}{on}"));
    frame(&mut doc);
    twin(&doc, &page(on, "", BOXES), "cb back from contents");
}

#[test]
fn a2_mid_toggles_contents_inline_position_none() {
    let on = "position: relative;";
    for (name, mid) in [
        ("contents", "display: contents;"),
        ("inline", "display: inline;"),
        ("inline-block", "display: inline-block;"),
        ("relative", "position: relative;"),
        ("transform", "transform: scale(1);"),
        ("none", "display: none;"),
        ("flex", "display: flex;"),
    ] {
        let mut doc = build(&page(on, "", BOXES));
        let mid_id = m(&doc, "mid");
        doc.set_attribute(mid_id, "style", &format!("{MID}{mid}"));
        frame(&mut doc);
        twin(&doc, &page(on, mid, BOXES), &format!("mid -> {name}"));
        doc.set_attribute(mid_id, "style", MID);
        frame(&mut doc);
        twin(&doc, &page(on, "", BOXES), &format!("mid back from {name}"));
        // and starting from the toggled state
        let mut doc = build(&page(on, mid, BOXES));
        let mid_id = m(&doc, "mid");
        doc.set_attribute(mid_id, "style", MID);
        frame(&mut doc);
        twin(&doc, &page(on, "", BOXES), &format!("from {name}"));
    }
}

#[test]
fn a3_moves_removes_reinserts() {
    let on = "position: relative;";
    let empty = "";
    // move a to mid2 (other containing block)
    let mut doc = build(&page(on, "", BOXES));
    for name in ["a", "b", "s"] {
        let (x, mid2) = (m(&doc, name), m(&doc, "mid2"));
        doc.append_child(mid2, x);
    }
    frame(&mut doc);
    let moved = format!(
        r#"<div data-m="cb" style="{CBS}{on}"><div style="height: 20px"></div><div data-m="mid" style="{MID}">{empty}</div></div><div data-m="cb2" style="{CBS}position: relative"><div data-m="mid2" style="{MID}">{BOXES}</div></div>"#
    );
    twin(&doc, &moved, "moved to another containing block");
    // move to be direct children of cb2 (Taffy's own answer)
    for name in ["a", "b", "s"] {
        let (x, cb2) = (m(&doc, name), m(&doc, "cb2"));
        doc.append_child(cb2, x);
    }
    frame(&mut doc);
    let direct = format!(
        r#"<div data-m="cb" style="{CBS}{on}"><div style="height: 20px"></div><div data-m="mid" style="{MID}"></div></div><div data-m="cb2" style="{CBS}position: relative"><div data-m="mid2" style="{MID}"></div>{BOXES}</div>"#
    );
    twin(&doc, &direct, "moved to direct children");
    // back under mid (first cb), with remove + frame + reinsert
    for name in ["a", "b", "s"] {
        let x = m(&doc, name);
        doc.remove_node(x);
        frame(&mut doc);
        let mid = m(&doc, "mid");
        doc.append_child(mid, x);
        frame(&mut doc);
    }
    twin(&doc, &page(on, "", BOXES), "removed, framed, re-inserted");
    // remove and reinsert in one frame, reversed order (keyed reorder shape)
    let mid = m(&doc, "mid");
    let (a, s) = (m(&doc, "a"), m(&doc, "s"));
    doc.insert_before(mid, s, a);
    frame(&mut doc);
    let reordered = BOXES.to_string();
    let d = diff(&doc, &page(on, "", &reordered));
    assert!(d.is_empty(), "reorder: {d:?}");
}

#[test]
fn a4_inner_html_and_slab_reuse() {
    let on = "position: relative;";
    let mut doc = build(&page(on, "", BOXES));
    let mid = m(&doc, "mid");
    // replace with plain boxes (ids recycled to non-absolute nodes), frame
    let plain = r#"<div data-m="p1" style="width: 20px; height: 20px"></div><div data-m="p2" style="width: 50%; height: 20px"><div data-m="p3" style="width: 50%; height: 5px"></div></div><div data-m="p4" style="height: 9px"></div>"#;
    doc.set_inner_html(mid, plain);
    frame(&mut doc);
    twin(&doc, &page(on, "", plain), "recycled as plain");
    assert!(
        doc.tree.ancestor_absolutes.is_empty(),
        "set not drained: {:?}",
        doc.tree.ancestor_absolutes
    );
    // recycle straight into direct-parent absolutes with no frame between
    doc.set_inner_html(mid, BOXES);
    let direct = r#"<div data-m="w" style="position: relative; width: 100px; height: 50px; margin-left: 9px"><div data-m="d1" style="position: absolute; inset: 2px"></div><div data-m="d2" style="position: absolute; right: 1px; width: 50%; height: 4px"></div></div>"#;
    doc.set_inner_html(mid, direct);
    frame(&mut doc);
    twin(&doc, &page(on, "", direct), "recycled as direct absolutes");
    // and with a scroll between (stale placed_absolutes ids)
    doc.set_inner_html(mid, BOXES);
    frame(&mut doc);
    doc.set_inner_html(mid, direct);
    doc.set_scroll_top(mid, 5.0);
    doc.set_scroll_top(m(&doc, "cb"), 3.0);
    doc.set_scroll_top(mid, 0.0);
    doc.set_scroll_top(m(&doc, "cb"), 0.0);
    frame(&mut doc);
    twin(
        &doc,
        &page(on, "", direct),
        "recycled with a scroll before the frame",
    );
}

#[test]
fn a5_display_none_roundtrips() {
    let on = "position: relative;";
    for target in ["cb", "mid", "a", "b"] {
        let mut doc = build(&page(on, "", BOXES));
        let t = m(&doc, target);
        doc.set_style(t, "display", "none");
        frame(&mut doc);
        // change the cb while hidden
        let cb = m(&doc, "cb");
        doc.set_style(cb, "width", "350px");
        frame(&mut doc);
        doc.set_style(t, "display", "block");
        frame(&mut doc);
        let mut fresh = build(&page(on, "", BOXES));
        let t2 = m(&fresh, target);
        let cb2 = m(&fresh, "cb");
        fresh.set_style(t2, "display", "block");
        fresh.set_style(cb2, "width", "350px");
        fresh.resolve_layout(VP.0, VP.1 + 40.0);
        let (g, w) = (rects(&doc), rects(&fresh));
        // y positions unaffected by viewport height here (no bottom-anchored ICB)
        assert_eq!(g, w, "hidden {target}, resized, shown");
    }
}

// ---- 1b. flips no cascade sees --------------------------------------

fn finish_transitions(doc: &mut RinchDocument, id: NodeId) {
    for t in doc
        .tree
        .active_transitions
        .get_mut(&id.0)
        .unwrap()
        .values_mut()
    {
        t.start_time_ms -= 100_000.0;
    }
}

#[test]
fn b1_transform_transition_finishing_at_none_position_only_box() {
    let tr = "transition: transform 1000ms linear;";
    let mut doc = build(&page(
        &format!("{tr}transform: translateX(20px);"),
        "",
        BOXES,
    ));
    let cb = m(&doc, "cb");
    doc.set_attribute(cb, "style", &format!("{CBS}{tr}transform: none;"));
    frame(&mut doc);
    assert!(
        doc.tree.active_transitions.contains_key(&cb.0),
        "control: transition runs"
    );
    let _ = doc.take_dirty_nodes();
    finish_transitions(&mut doc, cb);
    doc.tick_transitions();
    assert!(
        !doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control"
    );
    frame(&mut doc);
    twin(
        &doc,
        &page(&format!("{tr}transform: none;"), "", BOXES),
        "transition finished at none",
    );
}

#[test]
fn b2_transform_transition_starting_from_none() {
    let tr = "transition: transform 1000ms linear;";
    let mut doc = build(&page(tr, "", BOXES));
    let cb = m(&doc, "cb");
    doc.set_attribute(
        cb,
        "style",
        &format!("{CBS}{tr}transform: translateX(20px);"),
    );
    frame(&mut doc);
    assert!(
        doc.tree.active_transitions.contains_key(&cb.0),
        "control: transition runs"
    );
    let _ = doc.take_dirty_nodes();
    finish_transitions(&mut doc, cb);
    doc.tick_transitions();
    frame(&mut doc);
    // compare relative to cb: the fresh twin has the final transform
    twin(
        &doc,
        &page(&format!("{tr}transform: translateX(20px);"), "", BOXES),
        "transition from none finished",
    );
}

fn anim_page(class: &str) -> String {
    format!(
        r#"<style>@keyframes k {{ from {{ transform: translateX(30px); }} to {{ transform: translateX(10px); }} }} .run {{ animation: k 1000ms linear; }}</style>{}"#,
        page("", "", BOXES).replacen(
            r#"data-m="cb""#,
            &format!(r#"data-m="cb" class="{class}""#),
            1
        )
    )
}

#[test]
fn b3_animation_start_after_first_layout() {
    let mut doc = build(&anim_page(""));
    let cb = m(&doc, "cb");
    doc.set_attribute(cb, "class", "run");
    frame(&mut doc);
    assert!(
        doc.tree.active_animations.contains_key(&cb.0),
        "control: animation runs"
    );
    assert!(
        doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control: cb while animating"
    );
    // the twin: a document that has the class from the start
    let fresh = build(&anim_page("run"));
    let rel = |d: &RinchDocument, n: &str| {
        let r = rects(d);
        let (c, x) = (r["cb"], r[n]);
        [x[0] - c[0], x[1] - c[1], x[2], x[3]]
    };
    for n in ["a", "b", "bc", "s"] {
        assert_eq!(
            rel(&doc, n),
            rel(&fresh, n),
            "{n} once the animation started"
        );
    }
}

/// PRE-EXISTING, not this PR (fails the same way on main ee5123ba): a
/// finished animation with no fill is dropped by `animation::tick_animations`
/// (`AnimationResult::Complete`) and nothing restores the base style, so the
/// last sample stays in `computed_style` — here the cb keeps a transform, and
/// stays a containing block — until something re-cascades the node.
/// Issue draft: reports/issue-draft-review2-1409-1.md.
#[test]
#[ignore = "pre-existing on main: a finished non-fill animation leaves its last sample"]
fn b4_animation_ending_without_fill() {
    let mut doc = build(&anim_page("run"));
    let cb = m(&doc, "cb");
    assert!(
        doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control"
    );
    let _ = doc.take_dirty_nodes();
    for a in doc.tree.active_animations.get_mut(&cb.0).unwrap() {
        a.start_time_ms -= 100_000.0;
    }
    doc.tick_animations();
    assert!(
        !doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control: finished, no fill"
    );
    frame(&mut doc);
    twin(
        &doc,
        &page("", "", BOXES),
        "animation finished without fill",
    );
}

// ---- 3. late writers ------------------------------------------------

fn ib_page(align: &str, lead: &str) -> String {
    format!(
        r#"<div data-m="cb" style="{CBS}position: relative; font-size: 16px; line-height: 20px; text-align: {align}">{lead}<span data-m="ib" style="display: inline-block; width: 60px; height: 20px"><div data-m="a" style="position: absolute; left: 7px; top: 9px; width: 5px; height: 5px"></div><div data-m="s" style="position: absolute; width: 5px; height: 5px"></div></span></div>"#
    )
}

#[test]
fn c1_text_align_moves_the_inline_block_in_the_text_only_path() {
    for lead in [
        "",
        "<span data-m=\"sp\" style=\"display: inline-block; width: 50px; height: 10px\"></span>",
    ] {
        let mut doc = build(&ib_page("left", lead));
        let cb = m(&doc, "cb");
        let before = rects(&doc)["ib"];
        doc.set_style(cb, "text-align", "right");
        frame(&mut doc);
        assert_ne!(rects(&doc)["ib"], before, "control: the inline-block moved");
        twin(&doc, &ib_page("right", lead), "text-align changed");
    }
}

#[test]
fn c2_text_before_the_inline_block_changes() {
    let mut doc = build(&ib_page("left", r#"<span data-m="t">ab</span>"#));
    let t = m(&doc, "t");
    doc.set_text_content(t, "abcdefgh ijkl");
    frame(&mut doc);
    twin(
        &doc,
        &ib_page("left", r#"<span data-m="t">abcdefgh ijkl</span>"#),
        "text grew before the chip",
    );
    // letter-spacing on the lead span only
    doc.set_style(t, "letter-spacing", "3px");
    frame(&mut doc);
    twin(
        &doc,
        &ib_page(
            "left",
            r#"<span data-m="t" style="letter-spacing: 3px">abcdefgh ijkl</span>"#,
        ),
        "letter-spacing before the chip",
    );
}

/// ICB twin of c1 — the same late writer for the box main already corrected.
#[test]
fn c3_text_align_icb() {
    let p = |align: &str| ib_page(align, "").replace("position: relative;", "");
    let mut doc = build(&p("left"));
    let cb = m(&doc, "cb");
    doc.set_style(cb, "text-align", "right");
    frame(&mut doc);
    twin(&doc, &p("right"), "text-align changed (ICB)");
}

/// An anonymous block box between: text beside a block child.
#[test]
fn c4_anonymous_box_between() {
    let p = |h: u32| {
        format!(
            r#"<div data-m="cb" style="{CBS}position: relative; font-size: 16px; line-height: 20px;"><div data-m="blk" style="height: {h}px"></div>text <span>more<div data-m="a" style="position: absolute; left: 7px; top: 9px; width: 5px; height: 5px"></div><div data-m="s" style="position: absolute; width: 5px; height: 5px"></div></span></div>"#
        )
    };
    let mut doc = build(&p(10));
    doc.set_style(m(&doc, "blk"), "height", "33px");
    frame(&mut doc);
    twin(&doc, &p(33), "block above an anonymous box grew");
}

// ---- 2. the chain flag ------------------------------------------------

fn scroll_page() -> String {
    format!(
        r#"<div data-m="cb" style="{CBS}position: relative"><div style="height: 20px"></div>
<div data-m="outer" style="{MID} overflow: auto; height: 100px;"><div style="width: 900px; height: 40px"></div>
<div data-m="inner" style="overflow: auto; width: 200px; height: 60px; margin-left: 11px"><div style="width: 700px; height: 400px"></div>
<div data-m="a" style="position: absolute; left: 5px; top: 6px; width: 20px; height: 20px"></div><div data-m="s" style="position: absolute; width: 20px; height: 20px"></div></div>
<div style="height: 300px"></div></div></div>
<div data-m="other" style="overflow: auto; width: 100px; height: 50px"><div style="height: 500px"></div></div>"#
    )
}

fn rel(doc: &RinchDocument, n: &str, to: &str) -> [f32; 2] {
    let r = rects(doc);
    [r[n][0] - r[to][0], r[n][1] - r[to][1]]
}

#[test]
fn d1_chain_flags_follow_structure() {
    let mut doc = build(&scroll_page());
    let (outer, inner, cb) = (m(&doc, "outer"), m(&doc, "inner"), m(&doc, "cb"));
    let a0 = rel(&doc, "a", "cb");
    let s0 = rel(&doc, "s", "cb");
    // a layout between scrolls (flags cleared and rebuilt)
    doc.set_scroll_top(inner, 23.0);
    doc.set_style(m(&doc, "other"), "height", "55px");
    frame(&mut doc);
    doc.set_scroll_left(inner, 31.0);
    doc.set_scroll_top(outer, 17.0);
    assert_eq!(rel(&doc, "a", "cb"), a0, "inset box after layout + scrolls");
    assert_eq!(
        rel(&doc, "s", "cb"),
        s0,
        "static box after layout + scrolls"
    );
    // the inner scroller becomes the containing block: its own scroll carries
    doc.set_style(inner, "position", "relative");
    frame(&mut doc);
    let ai = rel(&doc, "a", "inner");
    doc.set_scroll_top(inner, 40.0);
    assert_eq!(
        rel(&doc, "a", "inner"),
        [ai[0], ai[1] - 17.0],
        "own scroll carries"
    );
    // outer is no longer between a and its cb: scrolling it moves a with inner
    doc.set_scroll_top(outer, 30.0);
    assert_eq!(
        rel(&doc, "a", "inner"),
        [ai[0], ai[1] - 17.0],
        "outer scroll: box rides with inner"
    );
    // back to static: outer and inner between again, with NO scroll before layout
    doc.set_style(inner, "position", "static");
    frame(&mut doc);
    assert_eq!(rel(&doc, "a", "cb"), a0, "back under cb");
    doc.set_scroll_top(inner, 5.0);
    doc.set_scroll_left(outer, 9.0);
    assert_eq!(rel(&doc, "a", "cb"), a0, "scrolls after the flip back");
    assert_eq!(
        rel(&doc, "s", "cb"),
        s0,
        "static, scrolls after the flip back"
    );
    // cb's own scroll (set_scroll_top on a non-scroller writes an offset)
    doc.set_scroll_top(cb, 4.0);
    assert_eq!(
        rel(&doc, "a", "cb"),
        [a0[0], a0[1] - 4.0],
        "cb's own scroll carries"
    );
}

/// A scroll between a change that makes the scroller "between" and the layout
/// that change owes: flags are from the old layout.
#[test]
fn d2_scroll_before_the_owed_layout() {
    let mut doc = build(&scroll_page());
    let (outer, inner) = (m(&doc, "outer"), m(&doc, "inner"));
    doc.set_style(inner, "position", "relative");
    frame(&mut doc);
    // inner stops being the cb; scroll outer BEFORE the layout
    doc.set_style(inner, "position", "static");
    doc.set_scroll_top(outer, 21.0);
    doc.set_scroll_top(inner, 13.0);
    frame(&mut doc);
    let mut fresh = build(&scroll_page());
    let a0 = rel(&fresh, "a", "cb");
    let s0 = rel(&fresh, "s", "cb");
    let _ = &mut fresh;
    assert_eq!(rel(&doc, "a", "cb"), a0);
    assert_eq!(rel(&doc, "s", "cb"), s0);
}

/// A box moved INTO a scroller, no layout change otherwise... then scrolled
/// after the frame.
#[test]
fn d3_box_moved_into_and_out_of_a_scroller() {
    let mut doc = build(&scroll_page());
    let (outer, inner, cb) = (m(&doc, "outer"), m(&doc, "inner"), m(&doc, "cb"));
    let a = m(&doc, "a");
    doc.append_child(cb, a);
    frame(&mut doc);
    let a_direct = rel(&doc, "a", "cb");
    doc.set_scroll_top(outer, 12.0);
    doc.set_scroll_top(inner, 12.0);
    assert_eq!(
        rel(&doc, "a", "cb"),
        a_direct,
        "direct child: no scroller between"
    );
    doc.append_child(inner, a);
    frame(&mut doc);
    assert_eq!(rel(&doc, "a", "cb"), [5.0, 6.0]);
    doc.set_scroll_top(outer, 30.0);
    doc.set_scroll_top(inner, 44.0);
    assert_eq!(rel(&doc, "a", "cb"), [5.0, 6.0], "moved in, then scrolled");
}

// ---- position-only twins (no size-dependent box to drag a layout in) ----

const POS: &str = r#"<div data-m="a" style="position: absolute; right: 5px; bottom: 7px; width: 40px; height: 20px"></div><div data-m="s" style="position: absolute; width: 10px; height: 10px"></div>"#;

#[test]
fn p1_transition_finishing_at_none_position_only() {
    let tr = "transition: transform 1000ms linear;";
    let mut doc = build(&page(&format!("{tr}transform: translateX(20px);"), "", POS));
    let cb = m(&doc, "cb");
    doc.set_attribute(cb, "style", &format!("{CBS}{tr}transform: none;"));
    frame(&mut doc);
    assert!(
        doc.tree.active_transitions.contains_key(&cb.0),
        "control: transition runs"
    );
    let _ = doc.take_dirty_nodes();
    finish_transitions(&mut doc, cb);
    doc.tick_transitions();
    assert!(
        !doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control"
    );
    frame(&mut doc);
    twin(
        &doc,
        &page(&format!("{tr}transform: none;"), "", POS),
        "transition finished at none (position only)",
    );
}

#[test]
fn p2_transition_starting_from_none_position_only() {
    let tr = "transition: transform 1000ms linear;";
    let mut doc = build(&page(tr, "", POS));
    let cb = m(&doc, "cb");
    doc.set_attribute(
        cb,
        "style",
        &format!("{CBS}{tr}transform: translateX(20px);"),
    );
    frame(&mut doc);
    assert!(
        doc.tree.active_transitions.contains_key(&cb.0),
        "control: transition runs"
    );
    // first frame of the run: the cb contains the boxes already
    let rel = |d: &RinchDocument, n: &str| {
        let r = rects(d);
        let (c, x) = (r["cb"], r[n]);
        [x[0] - c[0], x[1] - c[1]]
    };
    assert_eq!(
        rel(&doc, "a"),
        [355.0, 273.0],
        "mid-run: resolved against the cb"
    );
}

#[test]
fn p3_position_only_membership() {
    for (name, on) in [
        ("position", "position: relative;"),
        ("transform", "transform: translateX(0);"),
    ] {
        let mut doc = build(&page("", "", POS));
        let cb = m(&doc, "cb");
        let (p, v) = on.trim_end_matches(';').split_once(": ").unwrap();
        doc.set_style(cb, p, v);
        frame(&mut doc);
        twin(&doc, &page(on, "", POS), &format!("gain {name}"));
        doc.set_attribute(cb, "style", CBS);
        frame(&mut doc);
        twin(&doc, &page("", "", POS), &format!("lose {name}"));
    }
    let on = "position: relative;";
    for (name, mid) in [
        ("contents", "display: contents;"),
        ("inline", "display: inline;"),
        ("inline-block", "display: inline-block;"),
        ("relative", "position: relative;"),
        ("none", "display: none;"),
    ] {
        let mut doc = build(&page(on, "", POS));
        let mid_id = m(&doc, "mid");
        doc.set_attribute(mid_id, "style", &format!("{MID}{mid}"));
        frame(&mut doc);
        twin(&doc, &page(on, mid, POS), &format!("mid -> {name}"));
        doc.set_attribute(mid_id, "style", MID);
        frame(&mut doc);
        twin(&doc, &page(on, "", POS), &format!("mid back from {name}"));
    }
    // an extra wrapper between mid and the boxes toggles contents
    let wrap = |w: &str| {
        page(
            on,
            "",
            &format!(
                r#"<div data-m="w" style="margin: 4px 0 0 6px; width: 50px; height: 50px; {w}">{POS}</div>"#
            ),
        )
    };
    let mut doc = build(&wrap("display: contents;"));
    let w = m(&doc, "w");
    doc.set_style(w, "display", "block");
    frame(&mut doc);
    twin(&doc, &wrap("display: block;"), "wrapper left contents");
    doc.set_style(w, "display", "contents");
    frame(&mut doc);
    twin(
        &doc,
        &wrap("display: contents;"),
        "wrapper back to contents",
    );
}

/// Animation with a transform starts on the cb after the first layout:
/// position-only boxes.
#[test]
fn p4_animation_start_position_only() {
    let pg = |class: &str| {
        format!(
            r#"<style>@keyframes k {{ from {{ transform: translateX(30px); }} to {{ transform: translateX(10px); }} }} .run {{ animation: k 1000ms linear; }}</style>{}"#,
            page("", "", POS).replacen(
                r#"data-m="cb""#,
                &format!(r#"data-m="cb" class="{class}""#),
                1
            )
        )
    };
    let mut doc = build(&pg(""));
    let cb = m(&doc, "cb");
    doc.set_attribute(cb, "class", "run");
    frame(&mut doc);
    assert!(
        doc.tree.nodes[cb.0].establishes_abs_containing_block(),
        "control: cb while animating"
    );
    let rel = |d: &RinchDocument, n: &str| {
        let r = rects(d);
        let (c, x) = (r["cb"], r[n]);
        [x[0] - c[0], x[1] - c[1]]
    };
    assert_eq!(
        rel(&doc, "a"),
        [355.0, 273.0],
        "resolved against the animated cb"
    );
}

// ---- cost probes (printed; run with --nocapture) ----------------------

fn rows_page(n: usize, card: &str, badge_parent: &str) -> String {
    let mut s = format!(r#"<div data-m="card" style="width: 500px; {card}">"#);
    for i in 0..n {
        s += &format!(
            r#"<div style="{badge_parent} height: 20px"><div style="position: absolute; right: 2px; top: 2px; width: 8px; height: 8px"></div><span>row {i}</span></div>"#
        );
    }
    s + "</div>"
}

#[test]
fn q1_flip_cost_with_unaffected_absolutes() {
    for n in [20usize, 500] {
        let mut doc = build(&rows_page(n, "", "position: relative;"));
        let card = m(&doc, "card");
        let _ = doc.take_dirty_nodes();
        let mut out = vec![];
        for v in ["translateY(-2px)", "none", "translateY(-2px)", "none"] {
            let before = doc.tree.taffy_computes;
            let t = std::time::Instant::now();
            doc.set_style(card, "transform", v);
            frame(&mut doc);
            out.push((doc.tree.taffy_computes - before, t.elapsed().as_micros()));
            let _ = doc.take_dirty_nodes();
        }
        eprintln!("Q1 n={n}: (computes, us) per hover flip = {out:?}");
        assert!(
            out.iter().all(|o| o.0 == 0),
            "a flip that reaches no absolute box ran Taffy: {out:?}"
        );
        // colour-only control
        let before = doc.tree.taffy_computes;
        let t = std::time::Instant::now();
        doc.set_style(card, "background-color", "red");
        frame(&mut doc);
        eprintln!(
            "Q1 n={n}: colour control computes={} us={}",
            doc.tree.taffy_computes - before,
            t.elapsed().as_micros()
        );
    }
}

#[test]
fn q2_transition_ticks_run_no_compute() {
    let tr = "transition: transform 1000ms linear;";
    let mut doc = build(&page(tr, "", BOXES));
    let cb = m(&doc, "cb");
    let c0 = doc.tree.taffy_computes;
    doc.set_attribute(
        cb,
        "style",
        &format!("{CBS}{tr}transform: translateX(20px);"),
    );
    frame(&mut doc);
    let start = doc.tree.taffy_computes - c0;
    let mut ticks = vec![];
    for _ in 0..3 {
        let c = doc.tree.taffy_computes;
        std::thread::sleep(std::time::Duration::from_millis(20));
        doc.tick_transitions();
        frame(&mut doc);
        ticks.push(doc.tree.taffy_computes - c);
        let _ = doc.take_dirty_nodes();
    }
    let c = doc.tree.taffy_computes;
    finish_transitions(&mut doc, cb);
    doc.tick_transitions();
    frame(&mut doc);
    eprintln!(
        "Q2 start={start} ticks={ticks:?} finish={}",
        doc.tree.taffy_computes - c
    );
    assert_eq!(ticks, vec![0, 0, 0], "a mid-run tick ran Taffy");
}

#[test]
fn q3_inset_drag_computes() {
    for (name, cbs, mids, style) in [
        (
            "direct child",
            "",
            "position: relative;",
            "left: 5px; top: 5px; width: 40px; height: 20px",
        ),
        (
            "ancestor, position only",
            "position: relative;",
            "",
            "left: 5px; top: 5px; width: 40px; height: 20px",
        ),
        (
            "ancestor, paired insets",
            "position: relative;",
            "",
            "left: 5px; right: 5px; top: 5px; height: 20px",
        ),
        (
            "ancestor, pct size",
            "position: relative;",
            "",
            "left: 5px; top: 5px; width: 50%; height: 20px",
        ),
        (
            "icb",
            "",
            "",
            "left: 5px; top: 5px; width: 40px; height: 20px",
        ),
    ] {
        let mut doc = build(&page(
            cbs,
            mids,
            &format!(
                r#"<div data-m="a" style="position: absolute; {style}"><div data-m="k" style="width: 50%; height: 50%"></div></div>"#
            ),
        ));
        let a = m(&doc, "a");
        let mut per = vec![];
        for i in 0..4 {
            let c = doc.tree.taffy_computes;
            let l = format!("{}px", 10 + i * 7);
            doc.set_styles(a, &[("left", &l), ("top", &l)]);
            frame(&mut doc);
            per.push(doc.tree.taffy_computes - c);
        }
        eprintln!("Q3 {name}: computes per drag move = {per:?}");
        let final_style = style
            .replace("left: 5px", "left: 31px")
            .replace("top: 5px", "top: 31px");
        twin(
            &doc,
            &page(
                cbs,
                mids,
                &format!(
                    r#"<div data-m="a" style="position: absolute; {final_style}"><div data-m="k" style="width: 50%; height: 50%"></div></div>"#
                ),
            ),
            name,
        );
        assert!(per.iter().all(|&c| c == 1), "{name}: {per:?}");
    }
}
