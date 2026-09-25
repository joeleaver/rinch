//! The pruned, cached hit test answers exactly what the unpruned one did.
//!
//! `hit_test` skips subtrees whose flow extent misses the point and reuses
//! stacking sequences across probes (`rinch_dom::hit_cache`). Both are pure
//! optimisations, so the oracle here is the walk they replaced, kept verbatim
//! as [`reference_hit_test_node`] — no extent, no cache, a fresh
//! `stacking_paint_order` per stacking root per probe. Every fixture compares
//! the two over a grid of points that reaches past the window on every side.
//!
//! The documents are generated from a fixed seed and mix everything the walk
//! special-cases: `relative`/`absolute`/`fixed`/`sticky` boxes with and
//! without `z-index`, negative offsets that overflow a parent, transforms,
//! `opacity`, `overflow: hidden`/`auto`/`clip` scrolled to a non-zero offset,
//! `pointer-events: none`, `visibility: hidden`, `display: contents`/`none`,
//! inline-blocks in text flow, and flex containers. The cache half is pinned
//! by probing, mutating (a style write, a scroll, a structural change, each
//! followed by a layout), and probing again: a stale memo would answer from
//! the old geometry.

use super::{Frame, descend, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::stacking::{paints_at_stacking_root, stacking_paint_order};

/// The oracle: `hit_test` as it was before the prune and the cache.
fn reference_hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    reference_hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

fn random_style(r: &mut Rng, depth: usize) -> String {
    let mut s = String::new();
    let display = r.pick(&[
        "block",
        "block",
        "block",
        "flex",
        "inline-block",
        "inline",
        "contents",
        "grid",
        "none",
    ]);
    // `none` rarely, or whole generated subtrees vanish.
    let display = if display == "none" && !r.chance(30) {
        "block"
    } else {
        display
    };
    s += &format!("display: {display}; ");
    if r.chance(60) {
        s += &format!("width: {}px; ", 10 + r.below(300));
    }
    if r.chance(60) {
        s += &format!("height: {}px; ", 5 + r.below(200));
    }
    if r.chance(30) {
        s += &format!("margin: {}px; ", r.below(30) as i64 - 10);
    }
    if r.chance(25) {
        s += &format!("padding: {}px; ", r.below(20));
    }
    if r.chance(15) {
        s += &format!("border: {}px solid black; ", r.below(6));
    }
    match r.below(12) {
        0 | 1 => s += "position: relative; ",
        2 | 3 => s += "position: absolute; ",
        4 if depth > 0 => s += "position: fixed; ",
        5 => s += "position: sticky; ",
        _ => {}
    }
    if r.chance(40) {
        s += &format!(
            "left: {}px; top: {}px; ",
            r.below(400) as i64 - 120,
            r.below(300) as i64 - 90
        );
    }
    if r.chance(25) {
        s += &format!("z-index: {}; ", r.below(7) as i64 - 3);
    }
    match r.below(10) {
        0 => s += "overflow: hidden; ",
        1 => s += "overflow: auto; ",
        2 => s += "overflow: clip; ",
        3 => s += "overflow-y: scroll; ",
        _ => {}
    }
    match r.below(14) {
        0 => {
            s += &format!(
                "transform: translate({}px, {}px); ",
                r.below(80) as i64 - 40,
                r.below(80) as i64 - 40
            )
        }
        1 => s += "transform: scale(1.5); ",
        2 => s += "transform: rotate(20deg); ",
        3 => s += "transform: scale(0); ",
        _ => {}
    }
    if r.chance(8) {
        s += "opacity: 0.5; ";
    }
    if r.chance(10) {
        s += "pointer-events: none; ";
    }
    if r.chance(6) {
        s += "visibility: hidden; ";
    }
    if r.chance(6) {
        s += "pointer-events: auto; visibility: visible; ";
    }
    s
}

fn build(
    doc: &mut RinchDocument,
    r: &mut Rng,
    parent: NodeId,
    depth: usize,
    all: &mut Vec<NodeId>,
) {
    let kids = if depth >= 5 { 0 } else { 1 + r.below(4) };
    for _ in 0..kids {
        match r.below(10) {
            0 | 1 => {
                let t = doc.create_text(r.pick(&[
                    "hello world ",
                    "x",
                    "some longer text that wraps around ",
                ]));
                doc.append_child(parent, t);
            }
            2 => {
                let b = doc.create_element("button");
                let style = format!(
                    "width: {}px; height: {}px; ",
                    10 + r.below(60),
                    8 + r.below(30)
                );
                doc.set_attribute(b, "style", &style);
                doc.append_child(parent, b);
                all.push(b);
            }
            _ => {
                let tag = r.pick(&["div", "div", "div", "span", "section", "p"]);
                let el = doc.create_element(tag);
                let style = random_style(r, depth);
                doc.set_attribute(el, "style", &style);
                doc.append_child(parent, el);
                all.push(el);
                build(doc, r, el, depth + 1, all);
            }
        }
    }
}

fn scroll_everything(doc: &mut RinchDocument, r: &mut Rng, all: &[NodeId]) {
    for &n in all {
        if r.chance(60) {
            let v = r.below(120) as f64;
            doc.set_scroll_top(n, v);
            if r.chance(40) {
                doc.set_scroll_left(n, r.below(60) as f64);
            }
        }
    }
}

/// Compare the two over a grid; returns how many points were probed.
#[track_caller]
fn assert_agree(doc: &RinchDocument, label: &str) -> usize {
    let mut probed = 0;
    let mut y = -23.0f32;
    while y < 640.0 {
        let mut x = -23.0f32;
        while x < 840.0 {
            let want = reference_hit_test(&doc.tree, x, y);
            let got = hit_test(&doc.tree, x, y);
            assert_eq!(
                got, want,
                "{label}: the pruned hit test disagrees at ({x}, {y})"
            );
            probed += 1;
            x += 7.0;
        }
        y += 7.0;
    }
    probed
}

#[test]
fn random_documents_hit_identically() {
    let mut hits_seen = 0usize;
    for seed in 1..=60u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let mut all = Vec::new();
        build(&mut doc, &mut r, body, 0, &mut all);
        doc.resolve_layout(800.0, 600.0);
        assert_agree(&doc, &format!("seed {seed}, as laid out"));

        // Scroll every scroll container somewhere off zero, through the
        // public API, and probe again — a stale stacking sequence or extent
        // would answer from before the scroll.
        scroll_everything(&mut doc, &mut r, &all);
        assert_agree(&doc, &format!("seed {seed}, scrolled"));

        // One node at a time, warm between: a scroll keeps every extent but
        // the scrolled node's chain (#911), so a chain that stops one link
        // short answers from before the scroll. A paint-only layout pass in
        // between keeps the memo too, and must still agree.
        for round in 0..4 {
            if all.is_empty() {
                break;
            }
            let n = all[r.below(all.len() as u64) as usize];
            doc.set_scroll_top(n, r.below(150) as f64);
            if r.chance(50) {
                doc.set_scroll_left(n, r.below(80) as f64);
            }
            if round % 2 == 1 {
                doc.resolve_layout(800.0, 600.0);
            }
            assert_agree(&doc, &format!("seed {seed}, scrolled one, round {round}"));
        }

        // Restyle some boxes and move one subtree, then lay out again.
        for &n in &all {
            if r.chance(20) {
                let style = random_style(&mut r, 1);
                doc.set_attribute(n, "style", &style);
            }
        }
        if all.len() > 2 {
            let a = all[r.below(all.len() as u64) as usize];
            doc.remove_node(a);
        }
        doc.resolve_layout(800.0, 600.0);
        assert_agree(&doc, &format!("seed {seed}, restyled"));

        // A mutation with no layout after it: the old code answered from the
        // stale layout it found, and so must the new one.
        if let Some(&n) = all.first() {
            doc.set_scroll_top(n, 7.0);
        }
        let probed = assert_agree(&doc, &format!("seed {seed}, before relayout"));
        assert!(probed > 10_000);
        hits_seen += (0..600)
            .step_by(37)
            .filter(|&v| hit_test(&doc.tree, v as f32, v as f32).is_some())
            .count();
    }
    // Positive control: the grids did find boxes, so agreement was not two
    // walks that both answered `None` everywhere.
    assert!(hits_seen > 100, "hits seen: {hits_seen}");
}

/// A long list in a scroller — the shape the prune exists for — scrolled to
/// the middle, with positioned rows and a nested clipper so the clip chain and
/// the `PositionedAuto` entries are exercised too.
#[test]
fn a_scrolled_list_hits_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "height: 400px; overflow-y: auto; margin: 10px",
    );
    doc.append_child(body, scroller);
    for i in 0..300 {
        let row = doc.create_element("div");
        let pos = if i % 7 == 0 {
            "position: relative; "
        } else {
            ""
        };
        doc.set_attribute(
            row,
            "style",
            &format!("{pos}height: 20px; margin-left: {}px", i % 5),
        );
        let t = doc.create_text(&format!("row {i}"));
        doc.append_child(row, t);
        doc.append_child(scroller, row);
        if i % 11 == 0 {
            let over = doc.create_element("div");
            doc.set_attribute(
                over,
                "style",
                "position: relative; left: -30px; top: 35px; width: 50px; height: 50px",
            );
            doc.append_child(row, over);
        }
    }
    doc.resolve_layout(800.0, 600.0);
    for top in [0.0, 1234.5, 5000.0] {
        doc.set_scroll_top(scroller, top);
        assert_agree(&doc, &format!("scrolled to {top}"));
    }
}

/// Scrollers inside scrollers, each holding `sticky`, `fixed`, `absolute`
/// and `relative` children, scrolled one at a time with the cache warm
/// between — the shape #911's partial invalidation has to get right. A scroll
/// drops every stacking sequence (the positioned rows are entries of the
/// body's, at offsets that moved), and keeps every extent but the chain above
/// a scrolled box whose extent reads its scroll offset.
///
/// A real scroller clips, so its extent is its own box and no extent anywhere
/// moves when it scrolls. The chain half is reached only by a box that does
/// **not** clip and still has a scroll offset — which only a programmatic
/// `set_scroll_top` gives one — so `wrapper` is that box, and a chain that
/// stops one link short leaves `<body>`'s children answering from before its
/// scroll.
///
/// Every probe grid runs on a warm cache: the previous grid filled it, and
/// the `resolve_layout` between some rounds takes the paint-only path, which
/// keeps it.
#[test]
fn nested_scrollers_scrolled_one_at_a_time_hit_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let mut scrollers = Vec::new();
    // Index 3: not a scroll container — no `overflow` — but scrolled anyway.
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "padding-top: 6px; margin-left: 9px");
    doc.append_child(body, wrapper);
    let mut parent = wrapper;
    // Three levels: body > outer > middle > inner, each a scroller of rows
    // and positioned children, the next level nested in its third row.
    for level in 0..3 {
        let sc = doc.create_element("div");
        let (top, h) = (10 + level * 7, 420 - level * 90);
        doc.set_attribute(
            sc,
            "style",
            &format!(
                "height: {h}px; width: {}px; overflow-y: auto; overflow-x: auto; \
                 margin-top: {top}px; margin-left: {}px",
                700 - level * 120,
                level * 13
            ),
        );
        doc.append_child(parent, sc);
        scrollers.push(sc);
        let mut next_parent = sc;
        for i in 0..40 {
            let row = doc.create_element("div");
            let style = match i % 9 {
                0 => "position: relative; height: 18px; width: 900px".to_string(),
                3 => "position: sticky; top: 0px; height: 16px; background: red".to_string(),
                5 => "height: 22px; margin-left: -15px".to_string(),
                _ => format!("height: {}px; margin-left: {}px", 14 + i % 4, i % 6),
            };
            doc.set_attribute(row, "style", &style);
            doc.append_child(sc, row);
            if i % 7 == 2 {
                let abs = doc.create_element("div");
                doc.set_attribute(
                    abs,
                    "style",
                    "position: absolute; left: 40px; top: 30px; width: 60px; height: 25px",
                );
                doc.append_child(row, abs);
            }
            if i % 13 == 4 {
                let fixed = doc.create_element("div");
                doc.set_attribute(
                    fixed,
                    "style",
                    &format!(
                        "position: fixed; left: {}px; top: {}px; width: 30px; height: 30px",
                        500 + level * 40,
                        20 + i * 7
                    ),
                );
                doc.append_child(row, fixed);
            }
            if i % 5 == 1 {
                // An in-flow child that overflows its row to the left and
                // down, so the row's extent is not its own box.
                let over = doc.create_element("div");
                doc.set_attribute(
                    over,
                    "style",
                    "margin-left: -25px; margin-top: 4px; width: 45px; height: 40px",
                );
                doc.append_child(row, over);
            }
            if i == 2 {
                next_parent = row;
            }
        }
        parent = next_parent;
    }
    scrollers.push(wrapper);
    doc.resolve_layout(800.0, 600.0);
    assert_agree(&doc, "as laid out");

    // Deepest first, then outwards and back in, off every fixed point.
    let steps: [(usize, f64, f64); 12] = [
        (2, 37.0, 0.0),
        (1, 55.5, 11.0),
        (3, 17.0, 5.0),
        (0, 91.0, 0.0),
        (2, 140.0, 23.0),
        (3, 44.0, 0.0),
        (0, 12.0, 7.0),
        (1, 0.0, 0.0),
        (2, 3.0, 0.0),
        (3, 0.0, 31.0),
        (1, 210.0, 40.0),
        (0, 260.0, 0.0),
    ];
    let mut hits_seen = 0;
    for (i, &(which, top, left)) in steps.iter().enumerate() {
        doc.set_scroll_top(scrollers[which], top);
        doc.set_scroll_left(scrollers[which], left);
        if i % 3 == 2 {
            doc.resolve_layout(800.0, 600.0);
        }
        assert_agree(
            &doc,
            &format!("step {i}: scroller {which} to ({left}, {top})"),
        );
        hits_seen += (0..600)
            .step_by(23)
            .filter(|&v| hit_test(&doc.tree, v as f32 * 1.3, v as f32).is_some())
            .count();
    }
    assert!(hits_seen > 50, "positive control: hits seen {hits_seen}");
}

/// A viewport resize moves percentage-sized boxes with no restyle and no
/// `DomDocument` write — only the layout pass sees it. Since #911 that pass
/// keeps the hit memo on its paint-only path, so this pins that the path which
/// does move boxes still drops it: the probes before the resize fill the cache
/// with extents at the old width.
#[test]
fn a_resize_that_moves_percentage_boxes_drops_the_memo() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    for i in 0..12 {
        let row = doc.create_element("div");
        doc.set_attribute(
            row,
            "style",
            &format!(
                "width: {}%; height: 30px; margin-left: {}%",
                20 + i * 5,
                i * 3
            ),
        );
        doc.append_child(body, row);
        let over = doc.create_element("div");
        // Overflows the row to the right by a percentage of the row, so the
        // row's extent — not just its box — moves with the viewport.
        doc.set_attribute(over, "style", "width: 150%; height: 12px");
        doc.append_child(row, over);
    }
    doc.resolve_layout(800.0, 600.0);
    assert_agree(&doc, "at 800");
    doc.resolve_layout(530.0, 600.0);
    let before = reference_hit_test(&doc.tree, 700.0, 20.0);
    assert_agree(&doc, "at 530");
    // Positive control: the resize did move a box across a probed point.
    doc.resolve_layout(800.0, 600.0);
    assert_ne!(
        reference_hit_test(&doc.tree, 700.0, 20.0),
        before,
        "the resize moved nothing under (700, 20)"
    );
}

// ── Ticks: the `HitStyleKey` fields a tick can write ────────────────────────
//
// A transition or animation tick writes `computed_style` with no layout around
// it, and invalidates the hit cache only when the node's `HitStyleKey`
// changed. These fixtures drive `rinch_dom::animation::tick_animations` at a
// controlled clock, so each lands mid-animation deterministically, and check
// the cached walk against the oracle straight after the tick, with the cache
// warmed by the probes before it. Each one kills the mutant that drops its
// field from the key.
//
// Only fields a tick can reach are here: `TransitionProperty` animates
// opacity, transform, padding, margin, border widths and radii, sizes, colours
// and font-size — no `display`, `position`, `overflow`, `z-index`,
// `visibility` or `pointer-events`. Those key fields cannot change without a
// cascade, which invalidates on its own; they have no tick fixture because no
// tick can write them.

const ANIM_MS: f64 = 1000.0;

/// A document with `css` loaded and `html` in the body, laid out, and the
/// clock time its (single) animation's first iteration starts. Every fixture
/// animation carries a `0.3s` delay, so the layout sees the base style — the
/// first keyframe is sampled only by the fixture's own tick.
fn animated_doc(css: &str, html: &str) -> (RinchDocument, f64) {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    let start = doc
        .tree
        .active_animations
        .values()
        .flatten()
        .map(|a| a.start_time_ms + a.delay_ms)
        .next()
        .expect("positive control: the animation started");
    (doc, start)
}

fn tick_at(doc: &mut RinchDocument, start: f64, fraction: f64) {
    rinch_dom::animation::tick_animations(&mut doc.tree, start + ANIM_MS * fraction);
}

/// The oracle's answers over the probe grid, to show a tick changed some.
fn oracle_grid(doc: &RinchDocument) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    for y in (0..600).step_by(7) {
        for x in (0..800).step_by(7) {
            out.push(reference_hit_test(&doc.tree, x as f32, y as f32));
        }
    }
    out
}

fn id_of(doc: &RinchDocument, class: &str) -> usize {
    doc.query_selector(&format!(".{class}"))
        .expect("fixture element")
        .0
}

/// `opacity` crossing below 1 turns an in-flow box into a stacking context.
/// The flow walk then skips it (`paints_at_stacking_root` is read live) while
/// the cached body sequence has no entry for it, so a tick that does not
/// invalidate leaves the box unhittable. (Review of #881, R2-1.)
#[test]
fn an_opacity_tick_into_a_stacking_context_keeps_the_box_hittable() {
    let (mut doc, t0) = animated_doc(
        "@keyframes k { from { opacity: 1; } to { opacity: 0.5; } }
         .a { width: 100px; height: 100px; animation: k 1s linear 0.3s infinite; }",
        r#"<div style="width: 300px; height: 300px"><div class="a"></div></div>"#,
    );
    let a = id_of(&doc, "a");
    assert!(!doc.tree.nodes[a].creates_stacking_context());
    assert_agree(&doc, "opacity, before");
    tick_at(&mut doc, t0, 0.5);
    assert!(
        doc.tree.nodes[a].creates_stacking_context(),
        "positive control: the tick made the box a stacking context"
    );
    assert_eq!(hit_test(&doc.tree, 50.0, 50.0), Some(a));
    assert_agree(&doc, "opacity, after the tick");
}

/// The same flip through `transform`: from the identity to a translation.
#[test]
fn a_transform_tick_off_the_identity_keeps_the_box_hittable() {
    let (mut doc, t0) = animated_doc(
        "@keyframes k { from { transform: translateX(0px); } to { transform: translateX(300px); } }
         .a { width: 100px; height: 100px; animation: k 1s linear 0.3s infinite; }",
        r#"<div style="width: 300px; height: 300px"><div class="a"></div></div>"#,
    );
    let a = id_of(&doc, "a");
    assert!(
        !doc.tree.nodes[a].creates_stacking_context(),
        "the fixture needs the identity at t = 0"
    );
    let before = oracle_grid(&doc);
    assert_agree(&doc, "transform, before");
    tick_at(&mut doc, t0, 0.5);
    assert!(doc.tree.nodes[a].creates_stacking_context());
    assert_ne!(before, oracle_grid(&doc), "positive control: the box moved");
    assert_agree(&doc, "transform, after the tick");
}

/// A slide whose transform is never the identity keeps its answers right
/// while the cache stays warm: the value is read live by `descend`, and no
/// cached extent or sequence holds it. Three ticks, each checked against the
/// oracle after the previous one warmed the cache.
#[test]
fn a_transform_slide_is_hit_where_it_is_painted() {
    let (mut doc, t0) = animated_doc(
        "@keyframes k { from { transform: translateX(50px); } to { transform: translateX(450px); } }
         .a { width: 100px; height: 100px; animation: k 1s linear 0.3s infinite; }",
        r#"<div style="width: 300px; height: 300px"><div class="a"><div style="width: 20px; height: 20px; margin-left: 60px"></div></div></div>"#,
    );
    let mut previous = oracle_grid(&doc);
    assert_agree(&doc, "slide, before");
    for f in [0.25, 0.5, 0.75] {
        tick_at(&mut doc, t0, f);
        let now = oracle_grid(&doc);
        assert_ne!(
            previous, now,
            "positive control: the tick at {f} moved the box"
        );
        assert_agree(&doc, &format!("slide at {f}"));
        previous = now;
    }
}

/// An IFC root's left/top padding and border widths place its atomic inlines
/// (`ifc_content_box_offset`) — read into a cached flow extent. Animating one
/// moves the inline-block out of its root's stale box before the next layout,
/// so a tick that does not invalidate prunes it away.
fn ifc_offset_tick(property: &str, to: &str, extra: &str) {
    let css = format!(
        "@keyframes k {{ from {{ {property}: 0px; }} to {{ {property}: {to}; }} }}
         .r {{ width: 60px; height: 30px; {extra} animation: k 1s linear 0.3s infinite; }}
         .b {{ display: inline-block; width: 20px; height: 20px; }}"
    );
    let (mut doc, t0) = animated_doc(&css, r#"<div class="r">ab<span class="b"></span></div>"#);
    let before = oracle_grid(&doc);
    assert_agree(&doc, &format!("{property}, before"));
    tick_at(&mut doc, t0, 0.5);
    assert_ne!(
        before,
        oracle_grid(&doc),
        "positive control: the {property} tick moved the inline-block"
    );
    assert_agree(&doc, &format!("{property}, after the tick"));
}

#[test]
fn a_padding_left_tick_moves_an_inline_block_it_places() {
    ifc_offset_tick("padding-left", "300px", "");
}

#[test]
fn a_padding_top_tick_moves_an_inline_block_it_places() {
    ifc_offset_tick("padding-top", "300px", "");
}

#[test]
fn a_border_left_width_tick_moves_an_inline_block_it_places() {
    ifc_offset_tick("border-left-width", "300px", "border-style: solid;");
}

#[test]
fn a_border_top_width_tick_moves_an_inline_block_it_places() {
    ifc_offset_tick("border-top-width", "300px", "border-style: solid;");
}

// ── The oracle, verbatim from before the prune ──────────────────────────────

#[allow(clippy::too_many_arguments)]
fn reference_hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<usize> {
    tree.perf
        .bump(rinch_dom::perf::Counter::HitTestNodesVisited);
    let node = tree.get(node_id)?;

    // The `position: fixed` hoist, the IFC content-box offset, the transform
    // inverse and the body's viewport re-seed all live in `descend`, which every
    // probe-point walk in this file shares.
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;

    let nw = node.layout.width;
    let nh = node.layout.height;

    // Skip entire subtree when visibility: hidden — per CSS spec, hidden elements
    // and their descendants don't receive pointer events. This also guards against
    // Stylo not cascading visibility to children when a parent's class changes.
    if matches!(
        node.computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden
            | rinch_dom::computed_style::VisibilityValue::Collapse
    ) {
        return None;
    }

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;

    // Nodes with overflow clipping must restrict child hit testing to within
    // bounds — the same predicate paint clips pixels with (#324), so a box
    // cannot be drawn somewhere it cannot be tapped.
    let check_children = !node.clips_overflow() || point_in_bounds;

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    // `check_children` is deliberately not a gate around the whole block: a
    // `position: fixed` entry of this root's sequence is not clipped by this
    // root (#545), so it must be probed even when the point is outside these
    // bounds. It is applied per entry below instead.
    //
    // The cost was weighed rather than overlooked: a stacking-context root now
    // builds its `stacking_paint_order` and walks it even when the probe misses
    // its box, skipping every non-fixed entry. That is real work per probe on a
    // tree with many out-of-bounds clipping stacking contexts, and probes run
    // per pointer move. Nothing cheaper is correct — knowing whether a subtree
    // holds a fixed box is what building the sequence answers — and if it ever
    // shows up in a profile the fix is a per-node "has a fixed descendant" bit
    // maintained at style time, not moving this gate back.
    {
        // An IFC text node's layout is stretched to the whole container
        // (write_inline_positions, for scroll-height) — those artificial bounds
        // would shadow inline-block siblings laid out in the same text flow,
        // swallowing clicks meant for e.g. a button. Skip it: the IFC root
        // itself is returned for plain-text hits and text selection resolves by
        // walking up to it.
        let is_stretched_ifc_text =
            |child: &rinch_dom::Node| child.is_text() && child.ifc_root.is_some();

        if node_id == tree.body_id || node.creates_stacking_context() {
            // A stacking-context root probes exactly the sequence paint draws,
            // read backwards — the last box painted is the first one tapped.
            // Same function, same offsets, opposite direction: the two cannot
            // drift apart the way two hand-written phase walks did.
            let order =
                stacking_paint_order(tree, node_id, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            for entry in order.iter().rev() {
                let Some(child) = tree.get(entry.node_id) else {
                    continue;
                };
                // This root's own bounds gate. A `position: fixed` entry is
                // exempt: this root is not its containing block, so its clip
                // does not apply — paint lifts the same bracket around it
                // (`paint_children_with_stacking`, #545). Without the exemption
                // a fixed modal inside an `overflow: hidden; z-index: 1` panel
                // would be painted and never tappable.
                let escapes_root_clip = child.computed_style.position
                    == rinch_dom::computed_style::PositionValue::Fixed;
                if !check_children && !escapes_root_clip {
                    continue;
                }
                if is_stretched_ifc_text(child) {
                    continue;
                }
                // The clipping ancestors this entry was hoisted past. Since
                // #324 an `overflow` box is not a stacking context, so the
                // walk went straight through them and the `check_children`
                // gate below never saw them — the chain is what keeps a
                // hoisted box unreachable exactly where it is unpainted.
                //
                // The clips are in this root's own space, which is the space
                // `x`/`y` are in here, and at scale 1.0 because that is what
                // this walk asked `stacking_paint_order` for.
                if !order
                    .clips_for(entry)
                    .iter()
                    .all(|c| c.contains(x as f64, y as f64))
                {
                    continue;
                }
                if let Some(hit) = reference_hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x as f32,
                    entry.offset_y as f32,
                    x,
                    y,
                    vx,
                    vy,
                ) {
                    return Some(hit);
                }
            }
        } else if check_children {
            // Not a stacking-context root — test only the children that were
            // not hoisted to an ancestor's sequence, in reverse tree order.
            // Nothing here can be a fixed box: one creates a stacking context
            // unconditionally, so `paints_at_stacking_root` skips it and it is
            // an entry of some ancestor's sequence instead. So this branch keeps
            // the plain bounds gate.
            for &child_id in rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id)
                .iter()
                .rev()
            {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if paints_at_stacking_root(child) || is_stretched_ifc_text(child) {
                    continue;
                }
                if let Some(hit) =
                    reference_hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, vx, vy)
                {
                    return Some(hit);
                }
            }
        }
    }

    if !point_in_bounds {
        return None;
    }

    // Skip this element if pointer-events: none (children still checked above)
    if matches!(
        node.computed_style.pointer_events,
        rinch_dom::computed_style::PointerEventsValue::None
    ) {
        return None;
    }

    Some(node_id)
}
