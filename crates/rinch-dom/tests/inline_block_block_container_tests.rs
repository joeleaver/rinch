//! #592 — `display: inline-block` is a **block container on the inside**.
//!
//! ```html
//! <div style="width: 400px; line-height: 20px">
//!   <span style="display: inline-block">text<div style="width:120px;height:30px">block</div>tail</span>
//! </div>
//! ```
//!
//! Only the *outside* of an `inline-block` is inline-level. Its inside is a
//! block container (css-display-3 §2.5), so CSS 2.1 §9.2.1.1 makes it generate
//! anonymous block boxes around its own inline runs exactly as a `<div>` would.
//! rinch laid the three pieces out in a **row**, 30px tall, because two separate
//! things said `inline-block` was not a block container: the predicate the
//! anonymous-box machinery asks (`DisplayMode::is_block_container`) and the
//! Taffy display the style conversion produces (`DisplayValue::to_taffy`).
//!
//! This file was written as the **falsifier**, before the fix, so it could not
//! be shaped by one. Every number in it was measured in **Chrome 150, standards
//! mode, served over HTTP** (`harnesses/session-2026-09-13/oracle-592/`), and
//! every fixture that pins the defect ran `#[ignore]`d against the unfixed
//! source first — 10 of the 16 below failed there; the other six are controls
//! and guard rails, which are supposed to pass on both sides.
//!
//! # The oracle: an inline-block's inside is a block container's inside
//!
//! The load-bearing comparison is a **twin**: the same content under
//! `display: inline-block` and under `display: block` at the same used width.
//! Chrome renders the two interiors identically — box `120x70`, the block child
//! at `(0, 20)`, `text` at `y = 1` and `tail` at `y = 51` on both sides. That is
//! a property of the *input* (which `display` the wrapper carries) that no
//! candidate implementation controls, so a fixture written on it bites under any
//! of them.
//!
//! # The controls, and why they are not the same shape
//!
//! A fix that simply blockified every atomic inline would pass most of the
//! fixtures below, so two of them exist to catch it:
//!
//! - **`inline-flex`** keeps a flex interior: Chrome lays the same three pieces
//!   out in a **row**, `166.25x30`. `an_inline_flex_lays_its_mixed_content_out_in_a_row`.
//! - **`inline-grid`** keeps a grid interior. Its *mixed-content* shape is a
//!   **fixed point** — Chrome gives it `120x70`, the same as the block twin,
//!   because a single-column grid stacks its items too — so the control here is
//!   #607's declared-track shape instead, where a grid interior (`100x10`, two
//!   items side by side in `40px 60px` tracks) is unmistakable.
//!
//! # Fixed points stepped off on purpose
//!
//! - **The block child's width.** 120px, not 40, so the inline-block's
//!   shrink-to-fit width is the child's declared width and not a text
//!   measurement: `text` and `tail` are ~29px and ~23px here, a quarter of it.
//! - **Two width-less children.** `0x20` with the second at `y = 10` is where a
//!   block interior and a flex row disagree (`0x10`, both at `y = 0`), while
//!   *one* child would look identical under either.
//! - **Padding and a border.** The block child's offset inside the inline-block
//!   is `(9, 27)`, not `(0, 20)`, so a fix that stacks the content but forgets
//!   the box model is caught.
//! - **A transition, both ways.** `inline ↔ inline-block` is inline-level on
//!   both sides *and* — since this fix — equal on every Taffy field, which is
//!   exactly the crossing #597's `is_inline_level` trigger could not see.
//!
//! # Two neighbours closed here, only one of them the same defect
//!
//! **#630** (an inline-block's inner `display: inline` element is not marked IFC
//! content) **is** this defect seen from another side, and the measurement says
//! so: reverting the `is_block_container` half alone brings it back. An
//! `inline-block` that is not a block container never becomes an IFC root, so
//! there is no IFC inside it to mark that inner inline.
//!
//! **#625** (an atomic inline measures its box with the *ancestor* IFC's text
//! style) is **not**, and that was worth measuring rather than assuming. Its
//! single cause is a pass order: `compute_inline_block_layouts` ran before
//! `sync_text_contexts`, so a flex- or grid-interior atomic inline measured its
//! text through a `NodeContext::Text` still holding its create-time
//! placeholders. Swapping the two closes it for all three atomic inlines **with
//! both of #592's causes reverted** — measured. It is fixed here because
//! #592 fixes the `inline-block` arm on its own (an IFC interior reads the
//! computed styles directly), and a fix that repaired one atomic inline and left
//! its two siblings wrong would break the `inline-flex`/`inline-grid` twin
//! fixtures in `atomic_inline_tests`, which Chrome says must agree.
//!
//! **#624** is *not* closed: a line box holding only an atomic inline still gets
//! no strut, so `a_nested_atomic_inline_is_measured_before_the_box_that_places_it`
//! records Chrome's `54x34` and asserts rinch's width only. See its doc.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;
/// One line is exactly this, so "one line versus three" is a 20px question
/// rather than a font question.
const LINE: f32 = 20.0;
const BLK_H: f32 = 30.0;
/// Four times the widest text run in this file, so the inline-block's
/// shrink-to-fit width is a declared number and not a glyph measurement.
const BLK_W: f32 = 120.0;
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px";
const BLK: &str = "width: 120px; height: 30px; background: rgb(255, 0, 255)";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn lay(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

/// On-screen position, summed through the box tree the way paint does. The
/// wrapped and unwrapped twins do not have the same ancestors, and an atomic
/// inline's own `layout.x/y` is IFC-relative rather than parent-relative, so
/// comparing the raw field would compare two different things.
fn abs(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let (x, y, _) = rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, id.0, 1.0);
    (x as f32, y as f32)
}

fn rel(doc: &RinchDocument, id: NodeId, base: NodeId) -> (f32, f32) {
    let (x, y) = abs(doc, id);
    let (bx, by) = abs(doc, base);
    (x - bx, y - by)
}

fn assert_consistent(doc: &RinchDocument, what: &str) {
    let t = doc.taffy_tree_violations();
    assert!(
        t.is_empty(),
        "{what}: Taffy tree inconsistent:\n  {}",
        t.join("\n  ")
    );
    let r = doc.run_bookkeeping_violations();
    assert!(
        r.is_empty(),
        "{what}: run bookkeeping inconsistent:\n  {}",
        r.join("\n  ")
    );
}

/// `text <120x30 block> tail` inside a wrapper with the given `display`, in a
/// 400px `line-height: 20px` container. Returns the container, the wrapper and
/// the block child.
fn mixed(display: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(&mut doc, c, "span", &format!("display: {display}"));
    txt(&mut doc, w, "text");
    let b = el(&mut doc, w, "div", BLK);
    txt(&mut doc, b, "block");
    txt(&mut doc, w, "tail");
    doc.resolve_layout(VW, VH);
    (doc, c, w, b)
}

/// The headline shape, against its block-container twin.
///
/// Chrome 150, both spellings at `width: 120px` so their used widths cannot
/// differ: wrapper `120x70`, block child at `(0, 20)`, `text` at `y = 1`,
/// `tail` at `y = 51`. rinch gave the `inline-block` side `172x30` with the
/// three pieces in a row at `x = 0, 29, 149`.
#[test]
fn an_inline_block_lays_its_mixed_content_out_as_a_block_container_does() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: {display}; width: 120px"),
        );
        txt(&mut doc, w, "text");
        let b = el(&mut doc, w, "div", BLK);
        txt(&mut doc, b, "block");
        txt(&mut doc, w, "tail");
        doc.resolve_layout(VW, VH);
        (doc, w, b)
    }

    let (bd, bw, bb) = build("block");
    let (id, iw, ib) = build("inline-block");

    // The twin first: whatever the numbers are, the two interiors must agree.
    assert_eq!(
        (lay(&id, iw).2, lay(&id, iw).3),
        (lay(&bd, bw).2, lay(&bd, bw).3),
        "an inline-block's box must be its block twin's box at the same used \
         width — Chrome gives both 120x70",
    );
    assert_eq!(
        rel(&id, ib, iw),
        rel(&bd, bb, bw),
        "and the block child must sit in the same place inside it — Chrome (0, 20)",
    );

    // Then the declared numbers, so a fix that breaks both twins identically is
    // caught too.
    assert_eq!(
        (lay(&id, iw).2, lay(&id, iw).3),
        (BLK_W, LINE + BLK_H + LINE),
        "Chrome: the inline-block is 120x70 — text, block and tail stacked",
    );
    assert_eq!(
        rel(&id, ib, iw),
        (0.0, LINE),
        "the block sits below the first line, not beside it",
    );
    assert_consistent(&id, "inline-block with mixed content");
}

/// The same shape with no declared width: the box shrink-to-fits to its widest
/// block child. Chrome: `120x70`.
///
/// Separate from the twin above because a `display: block` twin at `width: auto`
/// fills its container (400px), so this half of the behaviour has no twin and
/// has to be asserted against Chrome's number directly. 120 is the child's
/// declared width, four times the widest text run here.
#[test]
fn an_auto_width_inline_block_shrink_wraps_to_its_widest_block_child() {
    let (doc, _c, w, b) = mixed("inline-block");
    assert_eq!(
        (lay(&doc, w).2, lay(&doc, w).3),
        (BLK_W, LINE + BLK_H + LINE),
        "Chrome: 120x70",
    );
    assert_eq!(rel(&doc, b, w), (0.0, LINE));
    assert_consistent(&doc, "auto-width inline-block");
}

/// **Control.** `inline-flex` is inline-level on the outside and a *flex*
/// container on the inside, so the same three pieces stay in a **row**.
///
/// Chrome 150: `166.25x30`, the block child at `x = 25.8` (the width of `text`)
/// and `y = 0`. Neither x is asserted — they are glyph widths — but "the block
/// is beside the text, on one 30px row" is not.
///
/// This is the fixture that fails if the fix blockified every atomic inline
/// rather than `inline-block` alone — and **the structural half of it is the
/// half that bites**. Measured: admitting `InlineFlex` to
/// `DisplayMode::is_block_container` while leaving `DisplayValue::to_taffy`
/// alone mints two anonymous block boxes inside the `inline-flex` and marks its
/// text children as its IFC content, and changes **not one pixel** — the box is
/// `174x30` and the block child `(29, 0, 120x30)` either way, because Taffy's
/// flex algorithm lays an anonymous block box out exactly as it lays a text
/// leaf out. Every geometry assertion below passes under that mutant. So the
/// question has to be asked of the box tree: a flex container's text children
/// become anonymous **flex items**, not anonymous **block boxes**, and they are
/// not its IFC content.
#[test]
fn an_inline_flex_lays_its_mixed_content_out_in_a_row() {
    let (doc, _c, w, b) = mixed("inline-flex");
    assert_eq!(
        lay(&doc, w).3,
        BLK_H,
        "an inline-flex row is as tall as its tallest item (30), not 70",
    );
    let (bx, by) = rel(&doc, b, w);
    assert_eq!(
        by, 0.0,
        "the block child is a flex item on the row, not below it"
    );
    assert!(
        bx > 0.0,
        "…and it is beside `text`, not at the row's origin (got x = {bx})",
    );

    // The structural half. See the doc: nothing above discriminates.
    assert!(
        doc.tree.get(w.0).unwrap().run_boxes.is_empty(),
        "an inline-flex establishes no inline formatting context, so it mints \
         no anonymous block boxes — got {:?}",
        doc.tree.get(w.0).unwrap().run_boxes,
    );
    let claimed: Vec<usize> = doc
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.ifc_root == Some(w.0))
        .map(|(i, _)| i)
        .collect();
    assert!(
        claimed.is_empty(),
        "…and nothing inside it is its IFC content: {claimed:?}",
    );
    assert_consistent(&doc, "inline-flex with mixed content");
}

/// **Control.** `inline-grid` keeps a *grid* interior — #607's declared-track
/// shape, because the mixed-content shape is a fixed point (Chrome gives
/// `inline-grid` the same `120x70` a block container gets, a single-column grid
/// stacking its items).
///
/// Chrome 150 on this markup: `inline-grid` `100x10` with the children at
/// `(0,0,40,10)` and `(40,0,60,10)`; `inline-block` `0x20` with them stacked at
/// `(0,0,0,10)` and `(0,10,0,10)` — a grid interior sizes width-less children
/// from its tracks, a block interior shrink-to-fits them to nothing and stacks
/// them.
#[test]
fn an_inline_grid_keeps_its_tracks_where_an_inline_block_stacks() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: {display}; grid-template-columns: 40px 60px"),
        );
        let k1 = el(&mut doc, w, "div", "height: 10px");
        let k2 = el(&mut doc, w, "div", "height: 10px");
        doc.resolve_layout(VW, VH);
        (doc, w, k1, k2)
    }

    let (gd, gw, gk1, gk2) = build("inline-grid");
    assert_eq!(lay(&gd, gw).2, 100.0, "inline-grid: 40 + 60 tracks");
    assert_eq!(lay(&gd, gk1), (0.0, 0.0, 40.0, 10.0));
    assert_eq!(
        lay(&gd, gk2),
        (40.0, 0.0, 60.0, 10.0),
        "second track, beside"
    );

    let (bd, bw, bk1, bk2) = build("inline-block");
    assert_eq!(
        (lay(&bd, bw).2, lay(&bd, bw).3),
        (0.0, 20.0),
        "inline-block: a block interior ignores the tracks and stacks — Chrome 0x20",
    );
    assert_eq!(lay(&bd, bk1), (0.0, 0.0, 0.0, 10.0));
    assert_eq!(
        lay(&bd, bk2),
        (0.0, 10.0, 0.0, 10.0),
        "the second child is below the first, not beside it (a flex row puts \
         both at y = 0 and makes the box 10 tall)",
    );
    assert_consistent(&gd, "inline-grid tracks");
    assert_consistent(&bd, "inline-block width-less children");
}

/// Padding and a border wrap the stacked content, and the block child's offset
/// carries both.
///
/// Chrome 150 with `padding: 5px 7px; border: 2px solid black`: the box is
/// `138x84` (120 + 14 + 4 by 70 + 10 + 4) and the block child sits at `(9, 27)`.
/// Every number here is declared. rinch gave `190x44` with the child at `(38, 7)`.
#[test]
fn an_inline_blocks_padding_and_border_wrap_its_stacked_content() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(
        &mut doc,
        c,
        "span",
        "display: inline-block; padding: 5px 7px; border: 2px solid black",
    );
    txt(&mut doc, w, "text");
    let b = el(&mut doc, w, "div", BLK);
    txt(&mut doc, b, "block");
    txt(&mut doc, w, "tail");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        (lay(&doc, w).2, lay(&doc, w).3),
        (138.0, 84.0),
        "Chrome: 120 + 2*7 + 2*2 by 70 + 2*5 + 2*2",
    );
    assert_eq!(
        rel(&doc, b, w),
        (9.0, 27.0),
        "Chrome: the block child is inside the border and the padding, below \
         the first line",
    );
    assert_consistent(&doc, "padded inline-block");
}

/// A block two inlines deep inside an inline-block splits the inline it sits in
/// (#513) *within the inline-block's own formatting context*.
///
/// Chrome 150: the inline-block is `120x70` and the block child is at `(0, 20)`,
/// exactly as when it is a direct child. The `<b>` declares `font-weight: 400`
/// so the two sides measure the same text.
#[test]
fn a_block_two_inlines_deep_inside_an_inline_block_still_stacks() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(&mut doc, c, "span", "display: inline-block");
    txt(&mut doc, w, "out");
    let bb = el(&mut doc, w, "b", "font-weight: 400");
    txt(&mut doc, bb, "in");
    let blk = el(&mut doc, bb, "div", BLK);
    txt(&mut doc, blk, "block");
    txt(&mut doc, bb, "after");
    txt(&mut doc, w, "tailout");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        (lay(&doc, w).2, lay(&doc, w).3),
        (BLK_W, LINE + BLK_H + LINE),
        "Chrome: 120x70",
    );
    assert_eq!(rel(&doc, blk, w), (0.0, LINE));
    assert_consistent(&doc, "block two inlines deep inside an inline-block");
}

/// #630 — a `display: inline` element inside an inline-block is that
/// inline-block's IFC content, and deleting the wrapper changes nothing.
///
/// ```html
/// <span style="display:inline-block; position:relative">
///   <span>text<div style="position:absolute; width:40px; height:30px">block</div>tail</span>
/// </span>
/// ```
///
/// Chrome 150 renders this and its wrapper-free twin **identically**: the
/// inline-block is `46.25x20` (one line) and the absolute is at `(0, 20)`
/// relative to it. rinch made the inner span a real `29x44` box with
/// `ifc_root = None`, laid its text out by Taffy's block algorithm, and gave the
/// inline-block two line heights plus 4px.
///
/// The oracle is the twin, not the number: 46.25 is a glyph measurement. `y = 20`
/// is the declared `line-height`.
#[test]
fn an_inline_element_inside_an_inline_block_is_its_ifc_content() {
    fn build(wrapped: bool) -> (RinchDocument, NodeId, NodeId, Option<NodeId>) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("{CONTAINER}; position: relative"),
        );
        let ib = el(
            &mut doc,
            c,
            "span",
            "display: inline-block; position: relative",
        );
        let (host, inner) = if wrapped {
            let s = el(&mut doc, ib, "span", "");
            (s, Some(s))
        } else {
            (ib, None)
        };
        txt(&mut doc, host, "text");
        let a = el(
            &mut doc,
            host,
            "div",
            "width: 40px; height: 30px; position: absolute; background: rgb(255, 0, 255)",
        );
        txt(&mut doc, a, "block");
        txt(&mut doc, host, "tail");
        doc.resolve_layout(VW, VH);
        (doc, ib, a, inner)
    }

    let (wd, wib, wa, inner) = build(true);
    let (pd, pib, pa, _) = build(false);
    let inner = inner.unwrap();

    assert_eq!(
        (lay(&wd, wib).2, lay(&wd, wib).3),
        (lay(&pd, pib).2, lay(&pd, pib).3),
        "a bare inline wrapper inside an inline-block changes nothing — Chrome \
         gives both 46.25x20",
    );
    assert_eq!(
        lay(&wd, wib).3,
        LINE,
        "one line, not two plus a block (Chrome: 20)",
    );
    assert_eq!(
        rel(&wd, wa, wib),
        rel(&pd, pa, pib),
        "and the absolute lands in the same place in both",
    );
    assert_eq!(
        rel(&wd, wa, wib),
        (0.0, LINE),
        "Chrome puts the absolute below the line it follows",
    );
    assert_eq!(
        wd.tree.get(inner.0).unwrap().ifc_root,
        Some(wib.0),
        "the inner span is the inline-block's IFC content, not a block child of it",
    );
    assert_consistent(&wd, "inline inside an inline-block");
    assert_consistent(&pd, "the wrapper-free twin");
}

/// #625 — an inline-block measures its box with **its own** text style.
///
/// Chrome 150, `"Wavy milliliters WWW mmm"` with `line-height: 20px` declared on
/// the inline-block: `199.36x20` at `font-size: 16px` and `398.72x20` at 32px.
/// rinch gave `211x22` for **both**, and for `font-weight: 900` as well: the box
/// was measured with the ancestor IFC's root style (16px/400/undeclared
/// line-height, whose Parley default is 22), while paint used the declared one
/// — so 32px text painted 449px of ink into a 211px box.
///
/// The widths are glyph measurements, so what is asserted is the **ratio** and
/// the **declared** line-height. The ratio is the discriminator that survives a
/// different font set: doubling `font-size` cannot leave a live measurement
/// within 1.5x of where it started.
///
/// **The cause is a pass order, not #592's predicate**, and the two are pinned
/// apart: `compute_inline_block_layouts` ran before `sync_text_contexts`, whose
/// job is to replace a `NodeContext::Text`'s create-time placeholders
/// (`font_size: 16.0`, `font_weight: 400.0`, empty `line_height_css`) with the
/// real inherited values. Measured with both of #592's causes reverted and only
/// the swap kept: all three atomic inlines answer correctly. Measured with the
/// swap alone reverted: `inline-block` stays correct — its interior is an IFC
/// now and `build_inline_layout` reads computed styles directly — while
/// `inline-flex` and `inline-grid` go back to `211x22` at both font sizes.
/// `atomic_inline_tests` holds their side of it; the sibling twins there are why
/// this could not be left for a later PR.
#[test]
fn an_inline_block_measures_its_box_with_its_own_text_style() {
    fn width_of(extra: &str) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(
            &mut doc,
            c,
            "div",
            &format!("display: inline-block; line-height: 20px; {extra}"),
        );
        txt(&mut doc, w, "Wavy milliliters WWW mmm");
        doc.resolve_layout(VW, VH);
        let l = lay(&doc, w);
        (l.2, l.3)
    }

    let (w16, h16) = width_of("font-size: 16px");
    let (w32, h32) = width_of("font-size: 32px");

    assert!(
        w32 > w16 * 1.5,
        "doubling the inline-block's own font-size must widen its box \
         (Chrome: 199.36 → 398.72); got {w16} → {w32}",
    );
    assert_eq!(
        (h16, h32),
        (LINE, LINE),
        "the declared line-height reaches the measurement too — Chrome gives \
         both rows 20, and rinch gave 22, Parley's default for an *undeclared* \
         16px line",
    );
}

/// The same, for **all three** atomic inlines — the fixture that pins the pass
/// order rather than #592's predicate.
///
/// `inline-flex` and `inline-grid` have no inline formatting context inside, so
/// their interiors are measured through `NodeContext::Text` and the swap
/// described above is their whole fix. Chrome 150 gives all three the same box
/// for the same declared text (`199.36x20` at 16px, `398.72x20` at 32px), which
/// is the property this asserts: the three agree with each other, and each one
/// tracks its own `font-size`.
///
/// Off the fixed point twice over: the container declares `font-size: 16px`, so
/// the 16px row is where "reads its own style" and "reads the ancestor's" agree
/// and only the 32px row discriminates; and the container is 900px wide, so
/// neither row wraps and the height stays a `line-height` question.
#[test]
fn every_atomic_inline_measures_its_box_with_its_own_text_style() {
    fn box_of(display: &str, font_size: u32) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 900px; line-height: 20px; font-size: 16px",
        );
        let w = el(
            &mut doc,
            c,
            "div",
            &format!("display: {display}; line-height: 20px; font-size: {font_size}px"),
        );
        txt(&mut doc, w, "Wavy milliliters WWW mmm");
        doc.resolve_layout(VW, VH);
        let l = lay(&doc, w);
        (l.2, l.3)
    }

    let reference = box_of("inline-block", 32);
    assert!(
        reference.0 > box_of("inline-block", 16).0 * 1.5,
        "control: the inline-block arm tracks its own font-size",
    );
    for display in ["inline-flex", "inline-grid"] {
        assert_eq!(
            box_of(display, 32),
            reference,
            "an {display} box must be measured with its own text style, exactly \
             as an inline-block is — Chrome gives all three the same box",
        );
        assert_eq!(
            box_of(display, 16).1,
            LINE,
            "…including the declared line-height (22 is Parley's default for an \
             *undeclared* 16px line, which is what a stale measure context holds)",
        );
    }
}

/// The painted ink of that same inline-block fits inside the box the layout gave
/// it. The layout/paint disagreement is what made #625 user-visible rather than
/// cosmetic.
#[cfg(feature = "software-renderer")]
#[test]
fn an_inline_blocks_text_fits_the_box_it_was_measured_into() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(
        &mut doc,
        c,
        "div",
        "display: inline-block; line-height: 20px; font-size: 32px",
    );
    txt(&mut doc, w, "Wavy milliliters WWW mmm");
    doc.resolve_layout(VW, VH);
    let box_w = lay(&doc, w).2;

    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    let ink_right = px
        .iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
        .map(|(i, _)| i % VW as usize)
        .max()
        .expect("the text painted something");

    assert!(
        (ink_right as f32) <= box_w + 1.0,
        "32px text painted out to x = {ink_right} from a box only {box_w} wide \
         — the measurement and the paint used different styles",
    );
}

/// A percentage `min-width` that cannot bind must not change the box.
///
/// `min-width: 10%` of a 400px cell is 40px, far under the ~580px this text
/// wants, so the two spellings below are the same box by construction — a twin
/// whose oracle is the input, like the rest of this file. Chrome 150 gives both
/// `400x40`; rinch gives both `580x20` because an auto-width inline-block never
/// caps at its containing block (`min(max-content, available)` — the
/// min-content half of CSS 2.1 §10.3.5 is unimplemented, pre-existing and
/// unchanged here, which is precisely why the twin rather than the number is
/// asserted).
///
/// It is here because the percentage path is the only one that runs
/// `measure_inline_blocks`' shrink-to-fit pin, and the pin has to be taken from
/// **`unrounded_layout`**. Taffy rounds a final layout to whole pixels; pinning
/// the rounded 580 for a 580.37px max-content line re-broke the text into two
/// lines and left a 580-wide box holding an interior laid out for 400. Measured
/// both ways: `(580, 20)` with the unrounded pin, `(580, 40)` with the rounded
/// one, against a twin that is `(580, 20)` either way because it never reaches
/// the pin at all.
#[test]
fn a_percentage_min_width_that_cannot_bind_changes_nothing() {
    fn build(extra: &str) -> (f32, f32) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: inline-block{extra}"),
        );
        txt(
            &mut doc,
            w,
            "Wavy milliliters WWW mmm Wavy milliliters WWW mmm Wavy milliliters",
        );
        doc.resolve_layout(VW, VH);
        let l = lay(&doc, w);
        (l.2, l.3)
    }

    let bound = build("; min-width: 10%");
    let free = build("");
    assert_eq!(
        bound, free,
        "a min-width of 40px cannot change a box that wants ~580, so the \
         percentage re-measure must land where the plain one does",
    );
    assert_eq!(
        bound.1, LINE,
        "…on one line: the text was measured at the width the box has, not at \
         a whole-pixel rounding of it",
    );
}

/// An **empty** inline-block is zero-sized, and one with a border is exactly its
/// border.
///
/// Chrome 150 in a `line-height: 20px` container: `0x0`, and `2x2` with
/// `border: 1px solid black`. This is the pin on the one place the four
/// block-container sites deliberately disagree — `apply_empty_block_line_floor`
/// is a rinch divergence for blockified form controls and must not reach an
/// atomic inline, or every childless `inline-block` (an `<img>` with no `src`
/// included, since the UA sheet makes it one) becomes one line tall.
#[test]
fn an_empty_inline_block_is_zero_sized() {
    for (style, want) in [
        ("display: inline-block", (0.0, 0.0)),
        ("display: inline-block; border: 1px solid black", (2.0, 2.0)),
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", style);
        doc.resolve_layout(VW, VH);
        assert_eq!(
            (lay(&doc, w).2, lay(&doc, w).3),
            want,
            "[{style}] — Chrome gives an empty inline-block no box of its own",
        );
    }
}

/// A bare `<svg>` is an atomic inline, so it keeps its declared box and paints
/// wherever it is flowed — inside a `<button>`, inside an `inline-block`, and in
/// ordinary text.
///
/// Chrome 150 (`oracle-592/oracle4.html`): the icon is `16x16` in all three, at
/// `(38.39, 3)` inside the button, `(36.47, 0)` inside the inline-block, and
/// `(36.47, 0)` beside `Save` in a plain block. `getComputedStyle` reports
/// `display: inline` there, because in CSS an `<svg>` is an inline **replaced**
/// element; rinch models a replaced inline as `inline-block`, exactly as it
/// already does for `<img>`, and that is what the UA sheet rule says.
///
/// # Why this fixture is in #592's suite
///
/// Stylo gives an unknown element `display: inline`, and a `display: inline`
/// element that is IFC content owns no box (`is_flowed_inline_element`, #635) —
/// so a bare `<svg>` was `0x0` and unpainted in a plain block **before** this
/// PR too. What #592 changed is the blast radius: an `inline-block`'s interior
/// used to be a Taffy *flex* container, which blockified the svg into a flex
/// item and sized it from its declared `width`/`height` by accident, and a
/// block container flows it instead. Every raw `<button><svg/></button>` and
/// every icon-only Tooltip / Popover / HoverCard / DropdownMenu target — eight
/// `inline-block` declarations in `rinch-components` — would have gone blank on
/// merge. Found by the review of this PR, cured by one word in the UA sheet.
///
/// The ink assertion is what makes this bite: "does it have a box" and "did
/// anything reach the screen" are two questions, and the components lost the
/// second one.
#[cfg(feature = "software-renderer")]
#[test]
fn a_bare_svg_keeps_its_box_and_paints_wherever_it_is_flowed() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// `wrapper` is `tag|style` for the box between the container and
    /// `Save<svg/>`, or `None` to put them straight in the container.
    fn build(wrapper: Option<&str>) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let host = match wrapper {
            None => c,
            Some(spec) => {
                let (tag, style) = spec.split_once('|').unwrap();
                el(&mut doc, c, tag, style)
            }
        };
        txt(&mut doc, host, "Save");
        // The shape `render_tabler_icon` emits: a sized `<svg>` with a `viewBox`
        // and a filled `<path>`. An `<svg>`'s own CSS `background` is **not**
        // painted (`paint/svg.rs` owns the node), so a background would make
        // this fixture report zero ink under every spelling and discriminate
        // nothing — measured.
        let svg = el(&mut doc, host, "svg", "width: 16px; height: 16px");
        doc.set_attribute(svg, "viewBox", "0 0 16 16");
        let path = doc.create_element("path");
        doc.set_attribute(path, "d", "M0 0 H16 V16 H0 Z");
        doc.set_attribute(path, "fill", "rgb(255, 0, 255)");
        doc.append_child(svg, path);
        doc.resolve_layout(VW, VH);
        (doc, svg)
    }

    fn magenta(doc: &mut RinchDocument) -> usize {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        painter
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0 && p[0] > 200 && p[1] < 60 && p[2] > 200)
            .count()
    }

    for (what, wrapper) in [
        ("a plain block", None),
        ("a <button>", Some("button|")),
        ("an inline-block span", Some("span|display: inline-block")),
    ] {
        let (mut doc, svg) = build(wrapper);
        assert_eq!(
            (lay(&doc, svg).2, lay(&doc, svg).3),
            (16.0, 16.0),
            "inside {what}: the icon keeps its declared box (Chrome: 16x16)",
        );
        let ink = magenta(&mut doc);
        assert!(
            ink > 0,
            "inside {what}: the icon painted nothing — a flowed `display: inline` \
             element owns no box, and there is nothing for paint to fill",
        );
        assert_consistent(&doc, &format!("svg inside {what}"));
    }
}

/// **Control**: making `<svg>` an atomic inline must not blockify it. In
/// ordinary text it still joins the line it is in.
///
/// Chrome 150: `<div>Save<svg 16x16/>tail</div>` is one 21px line with the icon
/// at `(36.47, 0)` and `tail` after it. The widths either side of the icon are
/// glyph measurements and are not asserted; "one line, icon in the middle of
/// it" is.
#[test]
fn a_bare_svg_joins_the_line_it_is_in() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "Save");
    let svg = el(&mut doc, c, "svg", "width: 16px; height: 16px");
    txt(&mut doc, c, "tail");
    doc.resolve_layout(VW, VH);

    assert!(
        lay(&doc, c).3 < LINE * 2.0,
        "one line, not three: an `svg` blockified into a block-level box would \
         break the run around it (got height {})",
        lay(&doc, c).3,
    );
    assert!(
        rel(&doc, svg, c).0 > 0.0,
        "…and the icon sits after `Save` on that line, not at its origin",
    );
    assert_eq!(
        (lay(&doc, svg).2, lay(&doc, svg).3),
        (16.0, 16.0),
        "with its declared box intact",
    );
    assert_consistent(&doc, "svg flowed in text");
}

/// An icon-only atomic-inline target — the Tooltip / Popover / HoverCard /
/// DropdownMenu shape, whose whole hover region is one `<svg>`.
///
/// Chrome 150: the outer wrapper is `16x21`, the target `16x21`, the icon
/// `16x16`. Only the widths are asserted; the heights are the #624 line-box
/// question. Against this PR without the UA-sheet line all three are `0x0` and
/// the container is 0 tall — the hover target disappears.
#[test]
fn an_icon_only_atomic_inline_target_is_not_zero_sized() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let wrapper = el(
        &mut doc,
        c,
        "span",
        "display: inline-block; position: relative",
    );
    let target = el(&mut doc, wrapper, "span", "display: inline-block");
    let svg = el(&mut doc, target, "svg", "width: 16px; height: 16px");
    doc.resolve_layout(VW, VH);

    assert_eq!(lay(&doc, svg).2, 16.0, "the icon keeps its declared width");
    assert_eq!(
        lay(&doc, target).2,
        16.0,
        "the target shrink-wraps to it (Chrome: 16 wide)",
    );
    assert_eq!(lay(&doc, wrapper).2, 16.0, "and so does the wrapper");
    assert!(
        lay(&doc, c).3 > 0.0,
        "the container is not zero tall — a collapsed target is unhoverable",
    );
    assert_consistent(&doc, "icon-only atomic-inline target");
}

/// An `<img>` with no `src` has no box, and one with pixels has exactly its
/// pixels — the other half of the floor exclusion above, since the UA sheet
/// makes `<img>` an `inline-block`.
///
/// Chrome 150: `0x0` for a bare `<img>`.
#[test]
fn an_image_with_no_source_has_no_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let img = el(&mut doc, c, "img", "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        (lay(&doc, img).2, lay(&doc, img).3),
        (0.0, 0.0),
        "Chrome gives a source-less <img> no box; a one-line floor would make \
         it 19 tall",
    );
}

/// Restyling across `inline ↔ inline-block` reaches the same state as declaring
/// it, in both directions.
///
/// The oracle is the declared twin, which is a property of the input (whether a
/// transition happened). This crossing is the one #597's trigger could not see
/// once `inline-block` started mapping to `taffy::Display::Block`: both sides
/// are inline-level *and* equal on every Taffy field, so nothing re-ran the IFC
/// pass. Measured against the unfixed trigger: `inline → inline-block` left the
/// element with the split's parentless Taffy node (`C orphan`), and
/// `inline-block → inline` left its block child held by a node that is itself
/// detached (`D detached`).
///
/// The viewport changes between resolves, because `resolve_layout` early-returns
/// when nothing is dirty.
#[test]
fn restyling_between_inline_and_inline_block_reaches_the_declared_state() {
    fn declared(display: &str, vw: f32) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        txt(&mut doc, w, "text");
        let b = el(&mut doc, w, "div", BLK);
        txt(&mut doc, b, "block");
        txt(&mut doc, w, "tail");
        doc.resolve_layout(vw, VH);
        (doc, c, w, b)
    }

    for (from, to) in [("inline", "inline-block"), ("inline-block", "inline")] {
        let (mut rd, rc, rw, rb) = declared(from, VW);
        rd.set_attribute(rw, "style", &format!("display: {to}"));
        rd.resolve_layout(VW - 1.0, VH);

        let (dd, dc, dw, db) = declared(to, VW - 1.0);

        assert_consistent(&rd, &format!("restyled {from} -> {to}"));
        assert_consistent(&dd, &format!("declared {to}"));
        assert_eq!(
            (lay(&rd, rw).2, lay(&rd, rw).3),
            (lay(&dd, dw).2, lay(&dd, dw).3),
            "{from} -> {to}: the restyled wrapper's box is not the declared one",
        );
        assert_eq!(
            lay(&rd, rc).3,
            lay(&dd, dc).3,
            "{from} -> {to}: the container's height disagrees",
        );
        assert_eq!(
            rel(&rd, rb, rc),
            rel(&dd, db, dc),
            "{from} -> {to}: the block child is somewhere else",
        );
    }
}

/// A container that **stops** being a block container takes its detached text
/// back.
///
/// `mark_inline_descendants` removes inline content from an IFC root's Taffy
/// child list; nothing put a text node back when the root stopped being one,
/// because the heal's gate asked only about the node's own role and a text
/// node's role is `Inline` whatever happens around it. `<div>text</div>`
/// restyled to `display: flex` was a `C orphan` on `main` before this change —
/// #592 only widened the shape to every `inline-block`, a bare `<button>`
/// included.
///
/// The oracle is the declared twin again. Both spellings are asserted so the
/// `<div>` case, which is the pre-existing one, is pinned beside the new one.
#[test]
fn a_container_that_stops_being_one_takes_its_text_back() {
    fn build(tag: &str, start: &str, end: Option<&str>) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let p = el(&mut doc, body, "div", "width: 160px; line-height: 20px");
        let w = el(&mut doc, p, tag, start);
        let t = txt(&mut doc, w, "Edit");
        doc.resolve_layout(VW, VH);
        if let Some(e) = end {
            doc.set_attribute(w, "style", e);
            doc.resolve_layout(VW - 1.0, VH);
        }
        (doc, w, t)
    }

    for (tag, start) in [
        ("div", ""),
        ("button", ""),
        ("span", "display: inline-block"),
    ] {
        let (rd, _rw, rt) = build(tag, start, Some("display: flex"));
        let (dd, _dw, dt) = build(tag, "display: flex", None);

        assert_consistent(&rd, &format!("<{tag}> restyled to flex"));
        assert!(
            rd.tree
                .get(rt.0)
                .unwrap()
                .taffy_id
                .is_some_and(|t| rd.tree.taffy.parent(t).is_some()),
            "<{tag}>: the text has no Taffy parent after its container stopped \
             being a block container",
        );
        assert_eq!(
            dd.tree.get(dt.0).unwrap().ifc_root,
            rd.tree.get(rt.0).unwrap().ifc_root,
            "<{tag}>: the restyled text is claimed differently from the declared twin",
        );
    }
}

/// An atomic inline nested inside another is measured **before** the box that
/// has to place it.
///
/// ```html
/// <div style="padding: 20px">
///   <span style="display: inline-block; padding: 7px">
///     <i style="display: inline-block; width: 40px; height: 12px"></i>
///   </span>
/// </div>
/// ```
///
/// Both boxes are measured as Taffy compute roots, and the outer one's Parley
/// layout sizes the `InlineBox` it pushes for the inner from `Node::layout` — so
/// the order matters. Slab order is creation order, which for one
/// `set_inner_html` is outer-first: the host came out `14x14`, its padding and
/// nothing else.
///
/// Chrome 150: host `54x34`, inner at `(7, 10)` inside it. **rinch gives the
/// host `54x26` and the inner `(7, 7)`, and that 8px is #624**, not this fix: a
/// line box holding only an atomic inline gets no strut, so the line is the
/// inner's own 12px rather than the container's declared 20, and there is no
/// room above it to baseline-align into. The width and the inner's own box are
/// asserted; the height is recorded here and left to #624.
///
/// **This one passed on `main` too**, so it is a regression pin rather than a
/// kill: before the fix the inner `<i>` was not IFC content at all and the host
/// laid it out as an ordinary Taffy child. It fails against the *intermediate*
/// state — the fix with `inline_block_measure_roots` left in slab order — which
/// is the mutant it exists for (`host 14x14`, measured).
#[test]
fn a_nested_atomic_inline_is_measured_before_the_box_that_places_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.append_child(body, container);
    doc.set_inner_html(
        container,
        "<div style=\"padding: 20px\">\
           <span id=\"host\" style=\"display: inline-block; padding: 7px\">\
             <i id=\"inner\" style=\"display: inline-block; width: 40px; height: 12px\"></i>\
           </span>\
         </div>",
    );
    doc.resolve_layout(VW, VH);

    let host = NodeId(rinch_dom::testing::query_selector(&doc.tree, "[id=host]")[0]);
    let inner = NodeId(rinch_dom::testing::query_selector(&doc.tree, "[id=inner]")[0]);

    assert_eq!(
        lay(&doc, host).2,
        54.0,
        "Chrome: 40 + 2*7 — the inner box has to be measured before the host's \
         own measure reads it (14 is the padding alone)",
    );
    assert_eq!(
        (lay(&doc, inner).2, lay(&doc, inner).3),
        (40.0, 12.0),
        "the inner box keeps its declared size",
    );
    assert_eq!(
        rel(&doc, inner, host),
        (7.0, 7.0),
        "inside the host's padding. Chrome says (7, 10): its line box is the \
         container's declared 20px and the 12px box baseline-aligns into it, \
         where rinch's line box is the box's own 12px — that is #624",
    );
    assert_consistent(&doc, "nested atomic inlines");
}
