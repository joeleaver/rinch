//! #2741: a window's input region, computed from its document.
//!
//! Each fixture builds a document, lays it out at 400x300 with the body's
//! margin off, and asks [`input_region::compute`] (or the per-frame
//! [`InputRegionState`]) for the rects. Points are probed with
//! [`input_region::contains`], off the boxes' edges and corners so a fixture
//! cannot pass on a zero offset (an unscrolled box, an untransformed one).

use super::input_region::{self, InputRegionState, contains, to_pixels};
use peniko::kurbo::Rect;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_core::element::InputRegion;
use rinch_dom::RinchDocument;

const VIEW: (f32, f32) = (400.0, 300.0);

fn doc() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; width: 400px; height: 300px");
    (doc, body)
}

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

fn region(doc: &mut RinchDocument, mode: InputRegion) -> Vec<Rect> {
    doc.resolve_layout(VIEW.0, VIEW.1);
    input_region::compute(&doc.tree, mode)
}

/// The notification column: a transparent, content-sized column holding two
/// popups with a 20px gap. The popups are in the region, the gap and the
/// column's own transparent background are not — the whole point of #2741.
/// Mutant: count every element with a box (drop `paints_own_box`) and the gap
/// is in the column's box, so it is in.
#[test]
fn painted_takes_the_popups_and_leaves_the_gap_2741() {
    let (mut d, body) = doc();
    let column = el(
        &mut d,
        body,
        "position: absolute; left: 30px; top: 10px; width: 200px; \
         display: flex; flex-direction: column; gap: 20px",
    );
    el(&mut d, column, "height: 60px; background: #223");
    el(&mut d, column, "height: 40px; background: rgb(10, 20, 30)");
    let r = region(&mut d, InputRegion::Painted);
    assert!(contains(&r, 40.0, 20.0), "first popup: {r:?}");
    assert!(
        contains(&r, 225.0, 115.0),
        "second popup, near its far corner: {r:?}"
    );
    assert!(!contains(&r, 100.0, 80.0), "the gap is not in it: {r:?}");
    assert!(!contains(&r, 25.0, 20.0), "left of the column: {r:?}");
    assert!(!contains(&r, 100.0, 135.0), "below the last popup: {r:?}");
    assert_eq!(r.len(), 2, "one rect per popup: {r:?}");
}

/// An empty result is an empty region (the deskrinch session's requirement
/// 3): a column with nothing painted takes no input at all.
#[test]
fn nothing_painted_is_an_empty_region_2741() {
    let (mut d, body) = doc();
    let column = el(&mut d, body, "width: 200px; height: 100px");
    el(&mut d, column, "height: 40px; background: transparent");
    el(
        &mut d,
        column,
        "height: 40px; border: 3px solid transparent",
    );
    let r = region(&mut d, InputRegion::Painted);
    assert!(r.is_empty(), "{r:?}");
    assert!(region(&mut d, InputRegion::Marked).is_empty());
}

/// A visible border alone paints, as does text: a transparent paragraph puts
/// each line's extent in the region, not the paragraph's width. Mutant: take
/// an IFC root's whole box for its text and the point right of the short
/// label is in.
#[test]
fn borders_and_text_lines_paint_2741() {
    let (mut d, body) = doc();
    el(
        &mut d,
        body,
        "height: 30px; width: 100px; border-top: 2px solid red",
    );
    let p = el(
        &mut d,
        body,
        "width: 300px; font-size: 16px; line-height: 20px; font-family: sans-serif",
    );
    let t = d.create_text("Hi");
    d.append_child(p, t);
    let r = region(&mut d, InputRegion::Painted);
    assert!(contains(&r, 50.0, 20.0), "the bordered box: {r:?}");
    assert!(contains(&r, 4.0, 42.0), "the label's first glyph: {r:?}");
    assert!(
        !contains(&r, 200.0, 42.0),
        "right of a two-letter label is not in it: {r:?}"
    );
}

/// `Marked` takes only the marked elements, painted or not; `Painted` takes
/// a marked transparent box too, and `data-input-region="none"` takes a
/// painted subtree out (a marked element inside still counts). Mutant: read
/// the attribute's presence only, and `none` marks its box in.
#[test]
fn marked_and_the_attribute_in_painted_2741() {
    let (mut d, body) = doc();
    let a = el(&mut d, body, "height: 40px; width: 100px");
    d.set_attribute(a, "data-input-region", "");
    el(&mut d, body, "height: 40px; width: 100px; background: blue");
    let out = el(
        &mut d,
        body,
        "height: 60px; width: 100px; background: green",
    );
    d.set_attribute(out, "data-input-region", "none");
    let inner = el(&mut d, out, "height: 20px; width: 50px; background: red");
    let marked_inner = el(&mut d, out, "height: 20px; width: 50px");
    d.set_attribute(marked_inner, "data-input-region", "yes");
    let _ = inner;

    let m = region(&mut d, InputRegion::Marked);
    assert!(contains(&m, 50.0, 20.0), "marked box: {m:?}");
    assert!(!contains(&m, 50.0, 60.0), "painted but unmarked: {m:?}");
    assert!(contains(&m, 25.0, 110.0), "marked inside `none`: {m:?}");
    assert!(!contains(&m, 25.0, 90.0), "{m:?}");

    let p = region(&mut d, InputRegion::Painted);
    assert!(contains(&p, 50.0, 20.0), "a marked transparent box: {p:?}");
    assert!(contains(&p, 50.0, 60.0), "painted: {p:?}");
    assert!(
        !contains(&p, 75.0, 95.0),
        "`none` takes the green box out: {p:?}"
    );
    assert!(
        !contains(&p, 25.0, 90.0),
        "and the red one inside it: {p:?}"
    );
    assert!(
        contains(&p, 25.0, 110.0),
        "a marked box inside `none` counts: {p:?}"
    );
}

/// Nested boxes: a painted child of a transparent parent is in at its own
/// place (offsets summed), one inside a painted parent adds no rect, and one
/// that overflows its painted parent adds the part outside. Off zero: the
/// parent sits at (37, 23).
#[test]
fn nested_boxes_sum_offsets_and_overflow_2741() {
    let (mut d, body) = doc();
    let outer = el(
        &mut d,
        body,
        "position: absolute; left: 37px; top: 23px; width: 200px; height: 150px",
    );
    let card = el(
        &mut d,
        outer,
        "margin-left: 11px; width: 100px; height: 50px; background: #333",
    );
    el(&mut d, card, "width: 40px; height: 20px; background: #555");
    el(&mut d, card, "width: 160px; height: 20px; background: #777");
    let r = region(&mut d, InputRegion::Painted);
    assert!(
        contains(&r, 37.0 + 11.0 + 1.0, 23.0 + 1.0),
        "card's corner: {r:?}"
    );
    assert!(
        !contains(&r, 37.0 + 10.0, 23.0 + 1.0),
        "left of the card: {r:?}"
    );
    assert!(
        contains(&r, 37.0 + 11.0 + 150.0, 23.0 + 30.0),
        "the overflowing child past the card: {r:?}"
    );
    assert!(!contains(&r, 37.0 + 11.0 + 150.0, 23.0 + 45.0), "{r:?}");
    assert_eq!(
        r.len(),
        2,
        "the child inside the card is deduplicated: {r:?}"
    );
}

/// A transformed box is in at its painted place: the bounding box of the
/// transformed border box. Mutant: ignore transforms and the box is at its
/// layout place.
#[test]
fn a_transformed_box_is_in_where_it_is_painted_2741() {
    let (mut d, body) = doc();
    el(
        &mut d,
        body,
        "position: absolute; left: 20px; top: 20px; width: 50px; height: 40px; \
         background: red; transform: translate(100px, 50px) scale(2); transform-origin: 0 0",
    );
    let r = region(&mut d, InputRegion::Painted);
    assert!(!contains(&r, 30.0, 30.0), "not at the layout box: {r:?}");
    assert!(contains(&r, 125.0, 75.0), "the painted top-left: {r:?}");
    assert!(
        contains(&r, 215.0, 145.0),
        "the painted bottom-right (scale 2): {r:?}"
    );
    assert!(!contains(&r, 225.0, 155.0), "{r:?}");
}

/// A scrolled container moves its content and clips it to its padding box:
/// a row scrolled half out is in only where it shows. Mutant: drop the
/// scroll offset (the row is at its unscrolled place) or the clip (the part
/// scrolled out above the scroller is in).
#[test]
fn scrolled_content_moves_and_is_clipped_2741() {
    let (mut d, body) = doc();
    let scroller = el(
        &mut d,
        body,
        "position: absolute; left: 10px; top: 50px; width: 100px; height: 100px; overflow: hidden",
    );
    el(&mut d, scroller, "height: 70px");
    el(&mut d, scroller, "height: 40px; background: blue");
    el(&mut d, scroller, "height: 400px");
    d.resolve_layout(VIEW.0, VIEW.1);
    d.set_scroll_top(scroller, 90.0);
    let r = region(&mut d, InputRegion::Painted);
    // The blue row is at 70..110 in content, 50 + (70 - 90) = 30..70 on
    // screen, clipped to the scroller's 50..150.
    assert!(contains(&r, 20.0, 55.0), "the part that shows: {r:?}");
    assert!(contains(&r, 20.0, 69.0), "{r:?}");
    assert!(
        !contains(&r, 20.0, 40.0),
        "the part scrolled out is clipped: {r:?}"
    );
    assert!(
        !contains(&r, 20.0, 125.0),
        "not at its unscrolled place: {r:?}"
    );
}

/// Hidden things are not in the region: `display: none`, `visibility:
/// hidden` (a visible child of a hidden parent is), `pointer-events: none`.
#[test]
fn hidden_and_pointerless_boxes_are_out_2741() {
    let (mut d, body) = doc();
    el(&mut d, body, "height: 20px; background: red; display: none");
    let hidden = el(
        &mut d,
        body,
        "height: 40px; background: red; visibility: hidden",
    );
    el(
        &mut d,
        hidden,
        "height: 20px; width: 50px; background: blue; visibility: visible",
    );
    el(
        &mut d,
        body,
        "height: 20px; background: red; pointer-events: none",
    );
    let r = region(&mut d, InputRegion::Painted);
    assert!(!contains(&r, 100.0, 10.0), "hidden parent: {r:?}");
    assert!(contains(&r, 25.0, 10.0), "its visible child: {r:?}");
    assert!(
        !contains(&r, 100.0, 30.0),
        "the hidden parent below the child: {r:?}"
    );
    assert!(!contains(&r, 100.0, 50.0), "pointer-events: none: {r:?}");
}

/// Rounded corners are stepped in: the corner pixel of a 16px radius is out,
/// the curve's inside is in, and the straight edges are whole. Mutant: no
/// strips (the border box) and the corner pixel is in.
#[test]
fn rounded_corners_are_approximated_2741() {
    let (mut d, body) = doc();
    el(
        &mut d,
        body,
        "position: absolute; left: 40px; top: 30px; width: 120px; height: 80px; \
         background: #222; border-radius: 16px",
    );
    let r = region(&mut d, InputRegion::Painted);
    assert!(!contains(&r, 41.0, 31.0), "top-left corner pixel: {r:?}");
    assert!(
        !contains(&r, 159.0, 109.0),
        "bottom-right corner pixel: {r:?}"
    );
    assert!(contains(&r, 40.5, 70.0), "the straight left edge: {r:?}");
    // 45° into the corner, 3px inside the curve: (40, 30) + 16 * (1 - 0.707)
    // = 4.7px in on each axis, so (47, 37) is inside.
    assert!(contains(&r, 47.0, 37.0), "inside the curve: {r:?}");
    assert!(contains(&r, 100.0, 30.5), "the straight top edge: {r:?}");
    assert!(r.len() > 3, "strips, not one box: {r:?}");
}

/// The strips of an elliptical and an uneven set of radii stay inside the
/// box and cover its middle band whole.
#[test]
fn rounded_strips_stay_in_the_box() {
    use peniko::kurbo::Vec2;
    use rinch_dom::paint::radii::BorderRadii;
    let b = Rect::new(10.0, 20.0, 110.0, 70.0);
    let radii = BorderRadii {
        top_left: Vec2::new(20.0, 10.0),
        top_right: Vec2::ZERO,
        bottom_right: Vec2::new(5.0, 5.0),
        bottom_left: Vec2::ZERO,
    };
    let s = input_region::rounded_strips(b, &radii);
    for r in &s {
        assert!(
            r.x0 >= b.x0 && r.x1 <= b.x1 && r.y0 >= b.y0 && r.y1 <= b.y1,
            "{r:?}"
        );
    }
    assert!(
        contains(&s, 109.9, 20.1),
        "the square top-right corner: {s:?}"
    );
    assert!(
        !contains(&s, 10.5, 20.5),
        "the rounded top-left corner: {s:?}"
    );
    assert!(contains(&s, 10.5, 45.0), "the middle band: {s:?}");
    assert!(
        contains(&s, 10.1, 69.9),
        "the square bottom-left corner: {s:?}"
    );
}

/// The region follows layout: a popup added moves the hit cache's
/// generation and the next frame's region has it; a frame with nothing
/// changed computes nothing. Mutant: key on the viewport alone and the new
/// popup is never in.
#[test]
fn the_region_is_recomputed_after_a_layout_change_and_only_then_2741() {
    use rinch_dom::perf::Counter;
    let (mut d, body) = doc();
    let column = el(
        &mut d,
        body,
        "display: flex; flex-direction: column; gap: 10px; width: 100px",
    );
    el(&mut d, column, "height: 30px; background: red");
    d.resolve_layout(VIEW.0, VIEW.1);
    let mut state = InputRegionState::default();
    let view = (400, 300);
    let computes = |d: &RinchDocument| d.tree.perf.get(Counter::InputRegionComputes);

    let first = state
        .region(&d.tree, InputRegion::Painted, view)
        .unwrap()
        .to_vec();
    assert!(!contains(&first, 50.0, 50.0), "{first:?}");
    let base = computes(&d);
    let again = state
        .region(&d.tree, InputRegion::Painted, view)
        .unwrap()
        .to_vec();
    assert_eq!(first, again);
    assert_eq!(computes(&d), base, "nothing changed, nothing computed");

    el(&mut d, column, "height: 30px; background: blue");
    d.resolve_layout(VIEW.0, VIEW.1);
    let after = state
        .region(&d.tree, InputRegion::Painted, view)
        .unwrap()
        .to_vec();
    assert!(contains(&after, 50.0, 50.0), "the new popup: {after:?}");
    assert_eq!(computes(&d), base + 1);

    // A new viewport recomputes too; `All` is the whole window, `None` empty.
    state.region(&d.tree, InputRegion::Painted, (500, 300));
    assert_eq!(computes(&d), base + 2);
    assert!(state.region(&d.tree, InputRegion::All, view).is_none());
    assert_eq!(
        state.region(&d.tree, InputRegion::None, view),
        Some(&[][..])
    );
}

/// Pixel rects cover the logical ones: edges rounded outwards, at the scale
/// asked for.
#[test]
fn pixel_rects_round_outwards() {
    let r = [Rect::new(10.4, 20.6, 30.2, 40.0)];
    assert_eq!(to_pixels(&r, 1.0), vec![(10, 20, 21, 20)]);
    assert_eq!(to_pixels(&r, 2.0), vec![(20, 41, 41, 39)]);
    assert!(to_pixels(&[Rect::new(5.0, 5.0, 5.0, 9.0)], 1.0).is_empty());
}

/// rinch's own press filter agrees with the region the OS was given: a press
/// in the gap is not the window's, one on a popup is; `All` takes every
/// press and `None` none. Mutant: answer `true` for `Painted` and the gap
/// press is dispatched.
#[test]
fn a_press_outside_the_region_is_not_the_windows_2741() {
    let (mut d, body) = doc();
    let column = el(
        &mut d,
        body,
        "display: flex; flex-direction: column; gap: 20px; width: 100px",
    );
    el(&mut d, column, "height: 30px; background: red");
    el(&mut d, column, "height: 30px; background: red");
    d.resolve_layout(VIEW.0, VIEW.1);
    let mut state = InputRegionState::default();
    state.region(&d.tree, InputRegion::Painted, (400, 300));
    assert!(state.takes_press_at(InputRegion::Painted, 50.0, 15.0));
    assert!(
        !state.takes_press_at(InputRegion::Painted, 50.0, 40.0),
        "the gap"
    );
    assert!(state.takes_press_at(InputRegion::Painted, 50.0, 65.0));
    assert!(state.takes_press_at(InputRegion::All, 50.0, 40.0));
    assert!(!state.takes_press_at(InputRegion::None, 50.0, 15.0));
}

/// A document with the animation clock pinned and transitions on, `css`
/// loaded and one `div` of `class`; laid out once.
fn animated(css: &str, class: &str) -> (RinchDocument, NodeId) {
    let mut d = RinchDocument::new();
    d.set_animation_clock_ms(Some(1_000_000.0));
    d.load_css(css);
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0; width: 400px; height: 300px");
    let n = d.create_element("div");
    d.set_attribute(n, "class", class);
    d.append_child(body, n);
    d.tree.transitions_enabled = true;
    d.resolve_layout(VIEW.0, VIEW.1);
    (d, n)
}

/// The region the window keeps (`state`) against one computed afresh, at
/// rest after the animation: they must agree, and the popup must be where it
/// rests.
fn assert_cached_is_fresh(d: &RinchDocument, state: &mut InputRegionState, what: &str) {
    let cached = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap()
        .to_vec();
    let fresh = input_region::compute(&d.tree, InputRegion::Painted);
    assert_eq!(cached, fresh, "{what}: the kept region is stale");
}

const SLIDE_CSS: &str = "
    .pop { position: absolute; left: 0; top: 0; width: 100px; height: 40px;
           background: #345; transform: translateX(200px);
           transition: transform 200ms linear }
    .pop.in { transform: translateX(0px) }
    @keyframes slide { from { transform: translateX(200px) } to { transform: translateX(0px) } }
    .kf { position: absolute; left: 0; top: 0; width: 100px; height: 40px;
          background: #345; animation: slide 200ms linear forwards }
    .fade { position: absolute; left: 0; top: 0; width: 100px; height: 40px;
            background-color: rgba(0, 0, 0, 0); transition: background-color 200ms linear }
    .fade.on { background-color: rgb(0, 0, 0) }
";

/// A popup sliding in with a `transform` transition ends where it rests in
/// the region. The hit cache's generation keys only whether a transform is
/// `none`, so `translateX(200px)` → `translateX(0px)` never moved it, and the
/// region stayed at x 200..300 for good (#2741 review). Mutant: drop the
/// tick's `region_generation` bump.
#[test]
fn a_slide_in_transition_moves_the_region_2741() {
    let (mut d, n) = animated(SLIDE_CSS, "pop");
    let mut state = InputRegionState::default();
    let before = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap()
        .to_vec();
    assert!(
        contains(&before, 250.0, 20.0),
        "starts slid out: {before:?}"
    );
    d.set_attribute(n, "class", "pop in");
    d.resolve_layout(VIEW.0, VIEW.1);
    d.advance_animation_clock_ms(50.0);
    d.tick_transitions();
    d.resolve_layout(VIEW.0, VIEW.1);
    state.region(&d.tree, InputRegion::Painted, (400, 300));
    d.advance_animation_clock_ms(300.0);
    d.tick_transitions();
    d.resolve_layout(VIEW.0, VIEW.1);
    assert_cached_is_fresh(&d, &mut state, "transition");
    let r = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap();
    assert!(
        contains(r, 50.0, 20.0) && !contains(r, 250.0, 20.0),
        "at rest: {r:?}"
    );
}

/// The same with keyframes (`animation: slide … forwards`).
#[test]
fn a_slide_in_keyframe_animation_moves_the_region_2741() {
    let (mut d, _) = animated(SLIDE_CSS, "kf");
    let mut state = InputRegionState::default();
    d.advance_animation_clock_ms(50.0);
    d.tick_animations();
    d.resolve_layout(VIEW.0, VIEW.1);
    let mid = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap()
        .to_vec();
    assert!(contains(&mid, 160.0, 20.0), "mid-slide: {mid:?}");
    d.advance_animation_clock_ms(300.0);
    d.tick_animations();
    d.resolve_layout(VIEW.0, VIEW.1);
    assert_cached_is_fresh(&d, &mut state, "keyframes");
    let r = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap();
    assert!(
        contains(r, 50.0, 20.0) && !contains(r, 250.0, 20.0),
        "at rest: {r:?}"
    );
}

/// A popup fading in from `rgba(0, 0, 0, 0)` joins the region once its
/// colour has any alpha: no hit-test input changes, so only the colour's
/// crossing of alpha 0 can tell the region. Mutant: drop the bump.
#[test]
fn a_colour_fade_in_joins_the_region_2741() {
    let (mut d, n) = animated(SLIDE_CSS, "fade");
    let mut state = InputRegionState::default();
    assert!(
        state
            .region(&d.tree, InputRegion::Painted, (400, 300))
            .unwrap()
            .is_empty()
    );
    d.set_attribute(n, "class", "fade on");
    d.resolve_layout(VIEW.0, VIEW.1);
    // The first frame of the transition is still transparent.
    let start = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap()
        .to_vec();
    assert!(start.is_empty(), "transparent at the start: {start:?}");
    d.advance_animation_clock_ms(300.0);
    d.tick_transitions();
    d.resolve_layout(VIEW.0, VIEW.1);
    assert_cached_is_fresh(&d, &mut state, "fade");
    let r = state
        .region(&d.tree, InputRegion::Painted, (400, 300))
        .unwrap();
    assert!(contains(r, 50.0, 20.0), "faded in: {r:?}");
}

/// The press filter reads the rects as sent, edges rounded outwards: a
/// press at x 10.2 by a box from 10.5 is kept, as the compositor delivers it.
#[test]
fn the_press_filter_reads_the_rounded_rects() {
    let px = to_pixels(&[Rect::new(10.5, 10.5, 50.0, 30.0)], 1.0);
    assert!(input_region::contains_pixels(&px, 10.2, 20.0));
    assert!(!input_region::contains_pixels(&px, 9.9, 20.0));
    assert!(!contains(&[Rect::new(10.5, 10.5, 50.0, 30.0)], 10.2, 20.0));
}
