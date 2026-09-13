//! A near-opaque `opacity` layer must not destroy the clip it was pushed inside.
//!
//! `TinySkiaPainter::push_layer` has a fast path: at an `opacity` within
//! `f32::EPSILON` of `1.0` no layer is allocated, because compositing one back
//! at that alpha changes no pixel. The stack still has to stay balanced, so it
//! pushed a `LayerState::Clip { previous_mask: None }` — and *that* is the bug
//! (#560). The branch never takes the mask, so the `None` is not a saved value
//! at all; it is the assertion "nothing was clipping", which `pop_layer` then
//! installs over whatever really was.
//!
//! It is the **third** instance of one defect. #555 fixed two give-up branches
//! in `push_clip` that lost the enclosing clip the same way, and stated the rule
//! in the code:
//!
//! > A fast path that skips clipping work must still preserve the clip it
//! > inherited. Pushing `previous_mask: None` is a claim that nothing was
//! > clipping — not a way of saying "I did not need to change anything".
//!
//! # What actually escapes, which is not what the issue predicted
//!
//! #560 reports the *subject* escaping: "`opacity: 0.99999994` on a 200x200 red
//! box inside a 50x50 `overflow: hidden` container — the red escapes the
//! container and paints at full size". **Measured, it does not, and a fixture
//! written that way passes against the unfixed code.** The fast path leaves
//! `clip_mask` alone, so the enclosing clip is still in force for everything
//! drawn *inside* the layer; the damage is done at the `pop`, and it lands on
//! whatever paints next. So the escapee is the near-opaque box's **later
//! sibling**, and the fixture below is ordered accordingly — the subject first,
//! the victim after it.
//!
//! (One way to get the reported reading: children given `position: absolute`
//! inside a *static* container escape a static `overflow: hidden` ancestor
//! outright under #204, at every opacity. That was measured here too, and it is
//! why every test below asserts its controls rather than only its subject.)
//!
//! # Fixed points these tests deliberately step off
//!
//! - `opacity` itself. `1.0` never reaches `push_layer` (the caller guards on
//!   `opacity < 1.0`) and `0.5` takes the ordinary path, so both clip correctly
//!   with the bug in place. `0.99999994` is the single `f32` below `1.0` that is
//!   within `f32::EPSILON` of it, and it is the only *opacity* that reaches the
//!   branch at all. Both other values are asserted here, so the fixture cannot
//!   quietly become a test of nothing.
//! - The declaration that gets there. `opacity` is not the only caller, and it
//!   is the exotic one: `paint_node`'s `filter: grayscale(...)` arm pushes a
//!   `BlendMode::Saturation` layer at the filter's own amount, so
//!   **`filter: grayscale(1)` hits the same branch at exactly `1.0`** — an
//!   ordinary declaration, no unusual float needed. Measured, and the last two
//!   tests here are the ones to read first: two plain in-flow siblings of one
//!   `overflow: hidden` box, no stacking context anywhere, and the second one
//!   paints outside the box because the first is greyed.
//! - The subject's own pixels, which are clipped either way. Asserted anyway,
//!   with a comment, because "the red escapes" is the natural reading and it is
//!   wrong.
//! - The origin. The container carries a margin, so its clip rect is not the
//!   viewport rect and a clip dropped for the viewport's own would still show.
//! - One clip provenance. The clip reaches the sibling two different ways — as a
//!   link in the hoisted entries' `ClipSpan`, and as the collecting root's own
//!   bracket, which is in no chain (CLAUDE.md, "Stacking contexts and the clip
//!   chain") — and there is a test for each.
//!
//! **What none of this can see:** every assertion reads the tiny-skia pixmap.
//! `VelloPainter` has no fast path of this shape — its `push_layer` hands every
//! opacity straight to `vello::Scene::push_layer` and its `pop_layer` always
//! pops — so there is nothing on the GPU side for these tests to be silent
//! about. That is a statement about rinch's painter, not about what Vello does
//! with a near-1 layer internally, which no CPU-side test here can reach.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 300.0;

/// The one `f32` strictly below `1.0` that `push_layer`'s near-opaque test
/// accepts: `1.0 - 2f32.powi(-24)`, i.e. `5.96e-8` away from `1.0`, where
/// `f32::EPSILON` is `1.19e-7`. The next value down is exactly `f32::EPSILON`
/// away and takes the ordinary path.
const NEAR_OPAQUE: &str = "0.99999994";

fn paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * painter.width() + x) * 4) as usize;
    let d = painter.pixels();
    [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
}

fn rgb(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let p = pixel_at(painter, x, y);
    [p[0], p[1], p[2]]
}

/// A `50x50` `overflow: hidden` container at `(20, 10)` holding two stacking
/// contexts that share its clip: the **subject**, a near-opaque (or, for the
/// controls, ordinary) 200x20 red strip, and after it the **victim**, a plain
/// `z-index: 1` 200x200 green box.
///
/// `sc_container` chooses how the clip reaches the victim. With `false` the
/// container is static, so both children are hoisted into the *body's* sequence
/// and the container's clip is a link in their `ClipSpan`. With `true` the
/// container is itself a stacking context, so it collects them and its clip is
/// the bracket `paint_node` opened around the whole sequence — which is in no
/// entry's chain, and is therefore the more damaging one to lose: nothing in the
/// rest of the sequence puts it back.
fn document(opacity: &str, sc_container: bool) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let container = doc.create_element("div");
    let base = "width: 50px; height: 50px; overflow: hidden; margin: 10px 0 0 20px";
    doc.set_attribute(
        container,
        "style",
        &if sc_container {
            format!("{base}; position: relative; z-index: 0")
        } else {
            base.to_string()
        },
    );
    doc.append_child(body, container);

    let subject = doc.create_element("div");
    doc.set_attribute(
        subject,
        "style",
        &format!(
            "width: 200px; height: 20px; background-color: rgb(255, 0, 0); opacity: {opacity}"
        ),
    );
    doc.append_child(container, subject);

    let victim = doc.create_element("div");
    doc.set_attribute(
        victim,
        "style",
        "position: relative; z-index: 1; width: 200px; height: 200px; \
         background-color: rgb(0, 255, 0)",
    );
    doc.append_child(container, victim);

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);
    painter
}

/// Container box is `x in [20, 70)`, `y in [10, 60)`. The victim's own box is
/// `x in [20, 220)`, `y in [30, 230)`, so this point is inside the victim and a
/// long way outside the container — provably empty if the clip survived.
const OUTSIDE: (u32, u32) = (150, 150);
/// Inside both, so the victim is drawn at all rather than merely absent.
const INSIDE: (u32, u32) = (30, 45);
/// Inside the *subject's* box (`y in [10, 30)`) and outside the container.
const SUBJECT_OUTSIDE: (u32, u32) = (150, 20);

const GREEN: [u8; 3] = [0, 255, 0];

fn assert_victim_is_clipped(painter: &TinySkiaPainter, what: &str) {
    assert_eq!(
        pixel_at(painter, OUTSIDE.0, OUTSIDE.1),
        [0, 0, 0, 0],
        "{what}: the sibling painted at {OUTSIDE:?}, which is outside the 50x50 \
         overflow: hidden container it lives in"
    );
    assert_eq!(
        rgb(painter, INSIDE.0, INSIDE.1),
        GREEN,
        "{what}: the sibling is missing at {INSIDE:?}, inside the container — \
         this fixture only means something while it is drawn at all"
    );
}

/// The clip arrives as a link in the hoisted entries' `ClipSpan`. Consecutive
/// entries that share a span share one push, so the container's clip is pushed
/// once, before the subject, and popped once, after the victim — which is
/// exactly the window the subject's `pop_layer` used to blank.
#[test]
fn a_near_opaque_layer_keeps_the_clip_chain_its_later_sibling_shares() {
    let painter = document(NEAR_OPAQUE, false);
    assert_victim_is_clipped(&painter, "near-opaque subject, static container");
}

/// The clip arrives as the collecting root's own bracket. It is in no entry's
/// chain, so nothing later in the sequence re-pushes it: once `pop_layer` writes
/// a `None` over it, every remaining entry paints unclipped.
#[test]
fn a_near_opaque_layer_keeps_the_collecting_roots_own_bracket() {
    let painter = document(NEAR_OPAQUE, true);
    assert_victim_is_clipped(&painter, "near-opaque subject, stacking-context container");
}

/// The ordinary path, `opacity: 0.5`, saves the mask in `LayerState::Opacity`
/// and restores it. This is a control — it clips with the bug in place — and it
/// is also the pin on that path: mutating `parent_mask` to `None` there breaks
/// this and nothing else in the suite.
#[test]
fn an_ordinary_opacity_layer_keeps_the_clip_its_later_sibling_shares() {
    for sc in [false, true] {
        let painter = document("0.5", sc);
        assert_victim_is_clipped(&painter, "opacity: 0.5 subject");
    }
}

/// `opacity: 1` never reaches `push_layer` at all — `paint_node` guards on
/// `opacity < 1.0`. Asserted so that a change which stops the near-opaque value
/// from parsing, or moves the caller's guard, shows up as this pair going
/// identical rather than as the subject tests quietly testing nothing.
#[test]
fn a_fully_opaque_sibling_pushes_no_layer_and_keeps_the_clip() {
    for sc in [false, true] {
        let painter = document("1", sc);
        assert_victim_is_clipped(&painter, "opacity: 1 subject");
    }
}

/// The subject's **own** pixels are clipped whether or not the bug is present,
/// because the fast path leaves `clip_mask` alone and only the `pop` is wrong.
///
/// This is a fixed point, kept deliberately: #560 reports the subject escaping,
/// a fixture written to that description passes against the unfixed painter, and
/// the next person to read the issue will write it again unless the measurement
/// is here. It is not what protects anything.
#[test]
fn the_near_opaque_box_itself_was_never_the_one_that_escaped() {
    for sc in [false, true] {
        let painter = document(NEAR_OPAQUE, sc);
        assert_eq!(
            pixel_at(&painter, SUBJECT_OUTSIDE.0, SUBJECT_OUTSIDE.1),
            [0, 0, 0, 0],
            "the near-opaque box itself painted outside its container at \
             {SUBJECT_OUTSIDE:?}"
        );
    }
}

/// The same clipping container, with the subject greyed rather than faded, and
/// nothing in the document that is a stacking context: two plain in-flow
/// children, painted one after the other by the ordinary child walk.
///
/// This is the *reachable* form of #560. `filter: grayscale(1)` is a normal
/// thing to write, it resolves to exactly `1.0`, and `paint_node` hands that
/// straight to `push_layer` as the amount of a `BlendMode::Saturation` layer —
/// so the near-opaque branch fires on a declaration nobody would think of as a
/// near-opaque anything. Measured against the unfixed painter: the green box
/// paints at (150, 150), a hundred pixels outside the 50x50 box it lives in.
fn greyed_document(filter: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "width: 50px; height: 50px; overflow: hidden; margin: 10px 0 0 20px",
    );
    doc.append_child(body, container);

    let subject = doc.create_element("div");
    doc.set_attribute(
        subject,
        "style",
        &format!("width: 200px; height: 20px; background-color: rgb(255, 0, 0); filter: {filter}"),
    );
    doc.append_child(container, subject);

    let victim = doc.create_element("div");
    doc.set_attribute(
        victim,
        "style",
        "width: 200px; height: 200px; background-color: rgb(0, 255, 0)",
    );
    doc.append_child(container, victim);

    doc.resolve_layout(VW, VH);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint(&mut doc, &mut painter);
    painter
}

#[test]
fn a_full_grayscale_filter_keeps_the_clip_its_next_sibling_is_in() {
    let painter = greyed_document("grayscale(1)");
    assert_victim_is_clipped(&painter, "filter: grayscale(1) subject");
}

/// The controls for the pair above, and the second one is not decoration:
/// `grayscale(0.5)` takes the ordinary path and `none` pushes no layer at all,
/// so if either ever starts failing, the two-value spread that makes the test
/// above mean something has gone.
#[test]
fn a_partial_or_absent_filter_keeps_it_too() {
    for filter in ["none", "grayscale(0.5)"] {
        let painter = greyed_document(filter);
        assert_victim_is_clipped(&painter, &format!("filter: {filter} subject"));
    }
}

/// Straight into the painter, with no DOM: the same claim the document tests
/// make, stated about `TinySkiaPainter` alone so that a future change to
/// stacking, hoisting or the clip chain cannot make it stop being asked.
mod painter {
    use super::{TinySkiaPainter, pixel_at};
    use peniko::kurbo::{Affine, Rect};
    use peniko::{Color, Fill};
    use rinch_dom::paint::painter::{BlendMode, Painter};

    const RED: Color = Color::from_rgba8(255, 0, 0, 255);
    const BLUE: Color = Color::from_rgba8(0, 0, 255, 255);

    fn fill(p: &mut TinySkiaPainter, c: Color, r: Rect) {
        p.fill_color(Fill::NonZero, Affine::IDENTITY, c, &r.into());
    }

    /// Push a clip, open and close a near-opaque layer inside it, then draw.
    /// The draw is after the `pop_layer`, which is where the `None` used to
    /// land, and it reaches well outside the clip.
    #[test]
    fn the_near_opaque_fast_path_leaves_the_enclosing_clip_in_force_after_its_pop() {
        let mut p = TinySkiaPainter::new(200, 200);
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &Rect::new(20.0, 10.0, 70.0, 60.0).into(),
        );
        p.push_layer(
            BlendMode::Normal,
            0.99999994,
            Affine::IDENTITY,
            &Rect::new(0.0, 0.0, 200.0, 200.0).into(),
        );
        p.pop_layer();
        fill(&mut p, RED, Rect::new(0.0, 0.0, 200.0, 200.0));
        p.pop_layer();

        assert_eq!(
            pixel_at(&p, 150, 150),
            [0, 0, 0, 0],
            "a fill after a near-opaque layer's pop painted outside the clip \
             that layer was pushed inside"
        );
        assert_eq!(
            pixel_at(&p, 30, 45),
            [255, 0, 0, 255],
            "the fill is missing inside the clip, so this test is measuring the \
             wrong thing"
        );
    }

    /// The symmetric half, and the reason `pop_layer` cannot simply stop writing
    /// `clip_mask`: a real `push_clip`'s pop must restore what it replaced, or
    /// the clip outlives its bracket and eats everything drawn after it.
    #[test]
    fn popping_a_real_clip_restores_what_it_replaced() {
        let mut p = TinySkiaPainter::new(200, 200);
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &Rect::new(20.0, 10.0, 70.0, 60.0).into(),
        );
        fill(&mut p, RED, Rect::new(0.0, 0.0, 200.0, 200.0));
        p.pop_layer();
        fill(&mut p, BLUE, Rect::new(100.0, 100.0, 180.0, 180.0));

        assert_eq!(
            pixel_at(&p, 150, 150),
            [0, 0, 255, 255],
            "a fill after the clip was popped is still being clipped by it"
        );
    }
}
