//! Ticks: transition and animation samples that write the `HitStyleKey`
//! fields with no layout around them.

use super::super::hit_test;
use super::{assert_agree, reference_hit_test};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

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
fn oracle_grid(doc: &RinchDocument) -> Vec<Option<u64>> {
    let mut out = Vec::new();
    for y in (0..600).step_by(7) {
        for x in (0..800).step_by(7) {
            out.push(reference_hit_test(&doc.tree, x as f32, y as f32));
        }
    }
    out
}

fn id_of(doc: &RinchDocument, class: &str) -> u64 {
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
