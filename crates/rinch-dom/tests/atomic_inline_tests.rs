//! #595 and #607 — `display: inline-flex` and `display: inline-grid` are
//! **atomic inlines**, not block-level boxes.
//!
//! An atomic inline (css-display-3 §2.6) is inline-level on the outside — it
//! joins the line around it and shrink-wraps — while its inside is an
//! independent formatting context the IFC only measures and places.
//! `inline-block`, `inline-flex` and `inline-grid` are all three one. rinch used
//! to map `DisplayValue::InlineFlex` onto `DisplayMode::Flex`, the same value
//! plain `display: flex` gets, so an `inline-flex` box **ended the run it should
//! have joined** and filled its container instead of shrink-wrapping (#595).
//!
//! `inline-grid` had the **same symptom through a different mechanism** (#607),
//! which is why it needed a second fix and a second set of fixtures: the value
//! did not reach `style_resolution` at all. `display_from_stylo` folded
//! `(Inline, Grid)` into `DisplayValue::Grid`, so `inline-grid` and `grid`
//! arrived as the same value and no `DisplayMode` arm could tell them apart.
//! The cure was a `DisplayValue::InlineGrid` variant first, then a
//! `DisplayMode::InlineGrid` that answers `is_atomic_inline`.
//!
//! **The two halves of an atomic inline come from different places, and the
//! `inline-grid` fixtures are the ones that pin that.** The *outside* —
//! inline-level, shrink-wrapping — is `DisplayMode`; the *inside* — which
//! formatting context the children get — is `DisplayValue::to_taffy` alone. So
//! routing `inline-grid` through the `inline-block` or `inline-flex` machinery
//! gets the outside right and the inside wrong, and
//! `an_inline_grid_box_lays_its_interior_out_as_a_grid` is what refuses it.
//!
//! # The oracle is the pair
//!
//! Every shape below is built twice, differing only in the wrapper's `display`
//! (`inline-flex` against `inline-block`), plus a **`display: flex` control**
//! that must *not* move. Chrome 150 (standards mode, served over HTTP) renders
//! the two spellings byte-identically in every shape here, so the assertion is
//! a property of the *input* that no candidate implementation controls:
//!
//! | shape (400px wide, `line-height: 20px`) | `inline-block` | `inline-flex` | `flex` |
//! |---|---|---|---|
//! | `before <w>mid</w> after <blk>`  | H=50, w\@x=45.38 25.78x20 | **identical** | H=90, w=400 |
//! | `<w>mid</w>` alone               | H=20, w=25.78x20 | **identical** | w=400 |
//! | two `<button>`s in a block box   | H=20, side by side at x=0 / 26.7 | **identical** | H=40, stacked |
//! | `<w>` around two sized divs      | H=25, w=30 (block inside) | H=20, **w=50** (flex inside) | w=400 |
//! | `<w style="width:50%">` on a line| H=20, w=200 | **identical** | — |
//!
//! `inline-grid`'s shapes are measured the same way, against the same Chrome
//! build, and each fixture's own doc carries its table. The first row above is
//! the one they share: Chrome gives `inline-grid` **H = 50** with the wrapper
//! 25.78x20 at x = 45.38, i.e. byte-identical to both spellings in the table,
//! and rinch gave H = 90 with a 400px wrapper until #607.
//!
//! The absolute numbers are Chrome's and are **not** asserted where they come
//! from a text measurement: rinch gives the `inline-block` twin 52px where
//! Chrome gives 50, which is that box's own line height and a separate question
//! (#595 excludes it explicitly). Where a number *is* asserted it is derived
//! from sized `<div>`s, not from glyphs — `line-height` is declared everywhere
//! rather than derived, so nothing here is a pin on the local font set.
//!
//! `INTER` is the one shape where Chrome tells the two spellings apart, and it
//! is worth its own note: rinch lays an `inline-block`'s interior out as a flex
//! row (#592), which is wrong for `inline-block` and exactly right for
//! `inline-flex` — so rinch's answer for `inline-flex` there matches Chrome to
//! the pixel while the twin does not. The pair assertion is dropped for that
//! one shape and Chrome's `inline-flex` number asserted directly.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;
/// Declared, never derived — see the module doc.
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px";
const BLK: &str = "width: 40px; height: 30px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn lay(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

/// Both validators, every time: an atomic inline is detached from its parent's
/// Taffy tree and computed as a root of its own, so a classification that only
/// half-lands shows up here as an orphan rather than as a wrong number.
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

/// `<div>` holding two `<button>`s, each given `display`.
fn two_buttons(display: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let b1 = el(&mut doc, c, "button", &format!("display: {display}"));
    txt(&mut doc, b1, "one");
    let b2 = el(&mut doc, c, "button", &format!("display: {display}"));
    txt(&mut doc, b2, "two");
    doc.resolve_layout(VW, VH);
    (doc, c, b1, b2)
}

/// **The component library's shape**, and the widest-reaching half of #595:
/// `display: inline-flex` is what `Button`, `Badge`, `Checkbox`, `Switch`,
/// `Avatar`, `ActionIcon`, `CloseButton`, `Loader`, `Pagination` and `Center`
/// all declare. Two of them in a plain `<div>` sat in a **column** before this
/// fix, where a browser puts them **side by side**.
///
/// Off the fixed point on purpose: one box alone is at `x = 0` whether it
/// joined a line or started a block, so the discriminating fact is the
/// *second* box's x. The `flex` control is what makes that a statement about
/// inline-level-ness rather than about buttons.
#[test]
fn two_atomic_inlines_share_a_line_where_two_flex_containers_stack() {
    let (fd, fc, f1, f2) = two_buttons("inline-flex");
    let (bd, bc, b1, b2) = two_buttons("inline-block");
    let (xd, xc, _, x2) = two_buttons("flex");

    // Control 1: the `inline-block` twin really is side by side, or "both
    // stack" would pass.
    assert!(
        lay(&bd, b2).0 >= lay(&bd, b1).0 + lay(&bd, b1).2 - 0.01,
        "control (inline-block): the second box starts after the first ends \
         (b1={:?} b2={:?})",
        lay(&bd, b1),
        lay(&bd, b2),
    );
    // Control 2: `display: flex` is block-level and still stacks, so this
    // fixture discriminates inline-level-ness and not merely "two boxes".
    assert_eq!(
        lay(&xd, x2).0,
        0.0,
        "control (flex): a block-level flex container starts its own band"
    );
    assert!(
        lay(&xd, xc).3 > lay(&bd, bc).3,
        "control (flex): stacking is taller than sharing a line ({} vs {})",
        lay(&xd, xc).3,
        lay(&bd, bc).3,
    );

    assert_eq!(
        (lay(&fd, f1), lay(&fd, f2), lay(&fd, fc).3),
        (lay(&bd, b1), lay(&bd, b2), lay(&bd, bc).3),
        "two inline-flex boxes must share a line exactly as two inline-blocks \
         do — Chrome renders these two identically (H=20, x=0 and x=26.7)",
    );
    assert_consistent(&fd, "two inline-flex buttons");
}

/// A lone `inline-flex` box in a block container: inline-level, so the
/// container gets a **line box** and the box shrink-wraps, where a `flex`
/// container fills the 400px.
///
/// The width is the discriminator, and it is not a text measurement on the
/// `flex` side: 400 is the container's declared width.
#[test]
fn a_lone_atomic_inline_shrink_wraps_where_a_flex_container_fills() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        txt(&mut doc, w, "mid");
        doc.resolve_layout(VW, VH);
        (doc, c, w)
    }
    let (fd, fc, fw) = build("inline-flex");
    let (bd, bc, bw) = build("inline-block");
    let (xd, _, xw) = build("flex");

    assert_eq!(
        lay(&xd, xw).2,
        400.0,
        "control (flex): a block-level flex container fills its containing block"
    );
    assert!(
        lay(&bd, bw).2 < 400.0,
        "control (inline-block): an atomic inline shrink-wraps (got {})",
        lay(&bd, bw).2,
    );
    assert_eq!(
        (lay(&fd, fw), lay(&fd, fc).3),
        (lay(&bd, bw), lay(&bd, bc).3),
        "a lone inline-flex box is placed and sized exactly as a lone \
         inline-block — Chrome gives both 25.78x20 on one 20px line",
    );
    assert_consistent(&fd, "lone inline-flex");
}

/// The **shrink-to-fit width**, asserted against Chrome's own number rather
/// than against the twin, because this is the one shape where Chrome tells the
/// two spellings apart: `inline-flex` lays its two sized children out in a row
/// (30 + 20 = 50 wide), `inline-block` stacks them (30 wide).
///
/// Font-independent by construction — the children are sized `<div>`s and there
/// is no text in the box at all. This is the sizing half of #595 (the issue
/// reports the wrapper 400px wide against Chrome's 25.8 for the text case).
#[test]
fn an_inline_flex_box_shrink_to_fits_to_its_flex_rows_width() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        let k1 = el(&mut doc, w, "div", "width: 20px; height: 10px");
        let k2 = el(&mut doc, w, "div", "width: 30px; height: 10px");
        doc.resolve_layout(VW, VH);
        (doc, w, k1, k2)
    }
    let (fd, fw, fk1, fk2) = build("inline-flex");
    let (xd, xw, _, _) = build("flex");

    assert_eq!(
        lay(&xd, xw).2,
        400.0,
        "control (flex): the block-level twin still fills its container, so 50 \
         is not what every spelling gives"
    );
    assert_eq!(
        lay(&fd, fw).2,
        50.0,
        "Chrome sizes this inline-flex box 50x10 — its flex row's max-content \
         width, not its container's 400 (got {:?})",
        lay(&fd, fw),
    );
    assert_eq!(
        lay(&fd, fk1),
        (0.0, 0.0, 20.0, 10.0),
        "first child, in a row"
    );
    assert_eq!(
        lay(&fd, fk2),
        (20.0, 0.0, 30.0, 10.0),
        "second child, beside it"
    );
    assert_consistent(&fd, "inline-flex shrink-to-fit");
}

/// A percentage inline size on an atomic inline resolves against its
/// **containing block**, which for a box an IFC places is the IFC root — the
/// `resolve_percentage_inline_blocks` second pass. Chrome: 200px, on the same
/// 20px line as the `x` before it.
///
/// `width: 50%` of 400 is 200 under **either** classification, so the width is
/// a fixed point here and the discriminator is the box's *position*: block-level
/// starts its own band (y = 20, container 30 tall), inline-level sits on the
/// line (container 20 tall in Chrome).
#[test]
fn a_percentage_inline_size_on_an_atomic_inline_joins_the_line() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        txt(&mut doc, c, "x");
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: {display}; width: 50%; height: 10px"),
        );
        doc.resolve_layout(VW, VH);
        (doc, c, w)
    }
    let (fd, fc, fw) = build("inline-flex");
    let (bd, bc, bw) = build("inline-block");

    assert_eq!(
        lay(&bd, bw).2,
        200.0,
        "control: 50% of the 400px root is 200"
    );
    assert_eq!(
        (lay(&fd, fw), lay(&fd, fc).3),
        (lay(&bd, bw), lay(&bd, bc).3),
        "a 50%-wide inline-flex box resolves and sits exactly where its \
         inline-block twin does — Chrome puts both 200x10 on the first line",
    );
    assert_consistent(&fd, "percentage inline-flex");
}

/// The **content-box bridge**: an atomic inline an IFC places stores its
/// `layout` relative to the root's *content* box, while a parent-chain sum adds
/// up *border*-box origins, so paint (and hit testing, and caret placement) add
/// [`rinch_dom::paint::ifc_content_box_offset`]. A box the IFC does not place
/// gets `(0, 0)`.
///
/// Off the fixed point twice over: the padding and border are **non-zero and
/// asymmetric**, so a swapped or dropped axis shows, and a `flex` control pins
/// that the offset is `(0, 0)` for a box that is not an atomic inline — without
/// which "always the root's padding" would pass.
#[test]
fn the_ifc_content_box_bridge_covers_an_inline_flex_box() {
    fn build(display: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; \
             padding: 7px 11px 3px 13px; border: 5px solid black",
        );
        txt(&mut doc, c, "before");
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: {display}; width: 20px; height: 10px"),
        );
        doc.resolve_layout(VW, VH);
        (doc, w)
    }
    fn offset(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
        let node = doc.tree.get(id.0).unwrap();
        rinch_dom::paint::ifc_content_box_offset(&doc.tree, node)
    }

    let (bd, bw) = build("inline-block");
    let (fd, fw) = build("inline-flex");
    let (xd, xw) = build("flex");

    // left border + left padding, top border + top padding — asymmetric, so an
    // axis swap fails.
    assert_eq!(
        offset(&bd, bw),
        (18.0, 12.0),
        "control: the inline-block is bridged"
    );
    assert_eq!(
        offset(&xd, xw),
        (0.0, 0.0),
        "control: a block-level flex container is placed by Taffy, so it is not \
         bridged — the bridge is not simply 'the root's padding'"
    );
    assert_eq!(
        offset(&fd, fw),
        (18.0, 12.0),
        "an inline-flex box is placed by the IFC, so paint owes it the same \
         content-box offset as an inline-block",
    );
    assert_consistent(&fd, "bridged inline-flex");
}

/// An atomic inline keeps the position the **IFC** gave it across a
/// re-layout that does not rebuild the IFC.
///
/// It is detached from its parent's Taffy tree and measured standalone, so
/// Taffy reports it at `(0, 0)`; `read_layout_results` takes only the *size*
/// from that measure and keeps the IFC's x/y. Without that, a non-structural
/// re-layout snaps every atomic inline back to the line origin — "collapsing a
/// row of buttons into a pile", as the comment there says.
///
/// **The second `resolve_layout` is the whole fixture, and finding a trigger
/// took a probe.** A repeat call at the same viewport early-returns, and a
/// restyle that dirties styles rebuilds the IFC, so neither reaches the code
/// this pins; a **viewport change** re-reads Taffy without an IFC rebuild and
/// does. Two boxes, not one: a lone box sits at `x = 0` whether its position
/// was kept or lost, so the discriminator is the *second* box's x. The
/// container is a fixed 400px, so the viewport change is required to move
/// nothing at all — and the `inline-block` twin is the control that says so.
#[test]
fn an_atomic_inline_keeps_its_ifc_position_across_a_non_structural_relayout() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let b1 = el(&mut doc, c, "button", &format!("display: {display}"));
        txt(&mut doc, b1, "one");
        let b2 = el(&mut doc, c, "button", &format!("display: {display}"));
        txt(&mut doc, b2, "two");
        doc.resolve_layout(VW, VH);
        (doc, b1, b2)
    }
    for display in ["inline-block", "inline-flex"] {
        let (mut doc, b1, b2) = build(display);
        let before = (lay(&doc, b1), lay(&doc, b2));
        assert!(
            before.1.0 > 0.0,
            "{display}: the second box must start off the line origin or this \
             fixture cannot tell a kept position from a lost one (got {before:?})",
        );

        // A wider viewport: nothing in the document depends on it (the container
        // is 400px), so every box must be exactly where it was.
        doc.resolve_layout(VW + 100.0, VH);

        assert_eq!(
            (lay(&doc, b1), lay(&doc, b2)),
            before,
            "{display}: a re-layout that does not rebuild the IFC must not move \
             an atomic inline back to the line origin",
        );
        assert_consistent(&doc, display);
    }
}

/// A `display: flex` container is **block-level** and must keep ending the run
/// — the guard rail for the whole change. If `Flex` were folded into the atomic
/// inlines, this is what would catch it.
///
/// The numbers are the issue's own: three bands (line, box, block) at 20 + 20 +
/// 30 = 90, against 50-ish for the inline-level spellings.
#[test]
fn a_block_level_flex_container_still_ends_the_inline_run() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "before");
    let w = el(&mut doc, c, "span", "display: flex");
    txt(&mut doc, w, "mid");
    txt(&mut doc, c, "after");
    let b = el(&mut doc, c, "div", BLK);
    txt(&mut doc, b, "block");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        lay(&doc, c).3,
        90.0,
        "Chrome: a block-level flex container splits the run into two lines \
         with its own band between them — 20 + 20 + 20 + 30",
    );
    assert_eq!(lay(&doc, w).2, 400.0, "and it fills its containing block");
    assert_eq!(lay(&doc, b).1, 60.0, "the block lands last");
    assert_consistent(&doc, "block-level flex");
}

/// `display: inline-grid` joins the line too (#607) — measured, not assumed, in
/// both directions.
///
/// Chrome 150 on this exact shape: `inline-grid` gives **H = 50**, the same as
/// `inline-block` and `inline-flex`, with the wrapper 25.78px wide at x = 45.38.
/// rinch gave **H = 90** and a 400px wrapper, i.e. #595's symptom unchanged by
/// #595's fix — because the value did not survive as far as `DisplayMode`:
/// `display_from_stylo` folded `(Inline, Grid)` into `DisplayValue::Grid`
/// (`computed_style/from_stylo/layout.rs`), so by the time `style_resolution`
/// classified it there was nothing left to tell it from `display: grid`. It now
/// converts to `DisplayValue::InlineGrid`, which `style_resolution` maps to
/// `DisplayMode::InlineGrid`, which answers `is_atomic_inline`.
///
/// A third spelling was differently wrong and is fixed with it:
/// `DisplayValue::parse` — the non-Stylo path in `computed_style/values.rs` —
/// had no `inline-grid` arm at all, so it fell through to `Self::default()`,
/// which is `Flex`. `computed_style/tests.rs` pins the arm and the fallback.
///
/// This fixture is the **flow** half and deliberately says nothing about the
/// box's interior: `an_inline_grid_box_lays_its_interior_out_as_a_grid` is what
/// keeps "inline-level" from being bought by turning the box into an
/// inline-block.
#[test]
fn an_inline_grid_box_joins_the_line_exactly_as_an_inline_block_does() {
    fn build(display: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        txt(&mut doc, c, "before");
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        txt(&mut doc, w, "mid");
        txt(&mut doc, c, "after");
        let b = el(&mut doc, c, "div", BLK);
        txt(&mut doc, b, "block");
        doc.resolve_layout(VW, VH);
        (doc, c)
    }
    let (gd, gc) = build("inline-grid");
    let (bd, bc) = build("inline-block");

    assert!(
        lay(&bd, bc).3 < 20.0 + 20.0 + 30.0,
        "control (inline-block): the run stays whole, so 'everything is three \
         bands' cannot pass (got {})",
        lay(&bd, bc).3,
    );
    assert_eq!(
        lay(&gd, gc).3,
        lay(&bd, bc).3,
        "an inline-grid box must join the line exactly as an inline-block does \
         — Chrome renders these two identically. inline-grid={} inline-block={}",
        lay(&gd, gc).3,
        lay(&bd, bc).3,
    );
}

/// The **interior** half of #607: an `inline-grid` box is inline-level on the
/// outside *and* a grid container on the inside, and the two halves come from
/// different places — the outside from [`rinch_dom::node::DisplayMode`], the
/// inside from `DisplayValue::to_taffy`.
///
/// This fixture exists because the cheapest wrong fix passes every other test
/// in this file: route `inline-grid` into the machinery `inline-block` or
/// `inline-flex` already has, and the box becomes inline-level — the run stays
/// whole, the box shrink-wraps — while its children stop being laid out in grid
/// tracks. Both of those spellings build a Taffy **flex** container, so two
/// width-less children collapse to nothing.
///
/// Font-independent by construction: the wrapper holds no text, the tracks are
/// declared, and the children are sized only in the axis the grid does not
/// place them along.
///
/// Chrome 150, standards mode, served over HTTP, on this exact markup:
///
/// | wrapper `display` | wrapper | first child | second child |
/// |---|---|---|---|
/// | `inline-grid` | **100x10** | x=0, 40x10 | x=40, 60x10 |
/// | `grid`        | 400x10     | x=0, 40x10 | x=40, 60x10 |
/// | `inline-block`| 0x20       | x=0, 0x10  | x=0, 0x10   |
/// | `inline-flex` | 0x10       | x=0, 0x10  | x=0, 0x10   |
///
/// rinch matches Chrome on every cell above except the two `0x20`/`0x10`
/// wrapper heights, which this fixture does not assert. (The containing block's
/// height is not asserted either: Chrome gives the `inline-grid` row 20px — its
/// declared `line-height`, since an atomic inline sits on a line box with a
/// strut — where rinch gives 10. That is the same line-box question #595
/// excluded from its own comparison, it predates this change, and it is
/// identical for the `inline-block` spelling, so it is not about `inline-grid`.)
#[test]
fn an_inline_grid_box_lays_its_interior_out_as_a_grid() {
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
        // No width of their own: the tracks are the only thing that can size
        // them, which is what makes a flex interior visible as a zero.
        let k1 = el(&mut doc, w, "div", "height: 10px");
        let k2 = el(&mut doc, w, "div", "height: 10px");
        doc.resolve_layout(VW, VH);
        (doc, w, k1, k2)
    }

    // Control 1: the same two children under a **block-level** grid really are
    // placed in the declared tracks, so the assertions below are about the
    // wrapper's outside and not about whether rinch supports the template at
    // all. Its 400 is also what the fix had to change: `inline-grid` used to be
    // this box.
    let (xd, xw, xk1, xk2) = build("grid");
    assert_eq!(lay(&xd, xw).2, 400.0, "control (grid): block-level, fills");
    assert_eq!(
        lay(&xd, xk1),
        (0.0, 0.0, 40.0, 10.0),
        "control: first track"
    );
    assert_eq!(
        lay(&xd, xk2),
        (40.0, 0.0, 60.0, 10.0),
        "control: second track"
    );

    // Control 2: the two *flex*-interior atomic inlines collapse here, in rinch
    // and in Chrome alike. This is the wrong fix, pinned: if `inline-grid` ever
    // answers these numbers instead, it has been folded into one of them.
    for flex_inside in ["inline-block", "inline-flex"] {
        let (d, w, k1, _) = build(flex_inside);
        assert_eq!(
            (lay(&d, w).2, lay(&d, k1).2),
            (0.0, 0.0),
            "control ({flex_inside}): a flex interior collapses width-less \
             children, so 100 is not what every atomic inline gives",
        );
    }

    let (gd, gw, gk1, gk2) = build("inline-grid");
    assert_eq!(
        lay(&gd, gw).2,
        100.0,
        "an inline-grid box shrink-to-fits to its grid tracks (40 + 60), not to \
         its container's 400 and not to a flex row's 0 — Chrome: 100x10 (got \
         {:?})",
        lay(&gd, gw),
    );
    assert_eq!(
        lay(&gd, gk1),
        (0.0, 0.0, 40.0, 10.0),
        "first child fills the first declared track",
    );
    assert_eq!(
        lay(&gd, gk2),
        (40.0, 0.0, 60.0, 10.0),
        "second child is placed in the second track, beside it",
    );
    assert_consistent(&gd, "inline-grid interior");
}

/// Two `inline-grid` boxes share a line, where two `grid` containers stack —
/// the flow half of #607 in the shape #595 found the component library in.
///
/// Off the fixed point: one box alone is at `x = 0` however it was classified,
/// so the discriminator is the **second** box's x. The `grid` control is what
/// makes this a statement about inline-level-ness rather than about two boxes.
#[test]
fn two_inline_grid_boxes_share_a_line_where_two_grid_containers_stack() {
    fn build(display: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let style = format!("display: {display}; grid-template-columns: 30px");
        let w1 = el(&mut doc, c, "span", &style);
        el(&mut doc, w1, "div", "height: 10px");
        let w2 = el(&mut doc, c, "span", &style);
        el(&mut doc, w2, "div", "height: 10px");
        doc.resolve_layout(VW, VH);
        (doc, c, w1, w2)
    }
    let (xd, xc, _, x2) = build("grid");
    let (gd, gc, g1, g2) = build("inline-grid");

    assert_eq!(
        lay(&xd, x2).0,
        0.0,
        "control (grid): a block-level grid container starts its own band"
    );
    assert!(
        lay(&xd, xc).3 > lay(&gd, gc).3,
        "control (grid): stacking is taller than sharing a line ({} vs {})",
        lay(&xd, xc).3,
        lay(&gd, gc).3,
    );
    assert_eq!(lay(&gd, g1).0, 0.0, "the first box opens the line");
    assert_eq!(
        lay(&gd, g2).0,
        30.0,
        "the second inline-grid box sits after the first's 30px track, on the \
         same line (got {:?} after {:?})",
        lay(&gd, g2),
        lay(&gd, g1),
    );
    assert_consistent(&gd, "two inline-grid boxes");
}

/// The **content-box bridge** reaches an `inline-grid` box too: a box the IFC
/// places stores its `layout` relative to the root's *content* box, so paint,
/// hit testing and caret placement add
/// [`rinch_dom::paint::ifc_content_box_offset`].
///
/// The sibling of `the_ifc_content_box_bridge_covers_an_inline_flex_box`, and it
/// pins a different statement: that one says `is_atomic_inline` is what the
/// bridge reads, this one says `inline-grid` answers it. Asymmetric padding and
/// border, so a swapped or dropped axis shows; a `grid` control pins that the
/// offset is `(0, 0)` for the block-level spelling, without which "always the
/// root's padding" would pass.
#[test]
fn the_ifc_content_box_bridge_covers_an_inline_grid_box() {
    fn build(display: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px; \
             padding: 7px 11px 3px 13px; border: 5px solid black",
        );
        txt(&mut doc, c, "before");
        let w = el(
            &mut doc,
            c,
            "span",
            &format!("display: {display}; width: 20px; height: 10px"),
        );
        doc.resolve_layout(VW, VH);
        (doc, w)
    }
    fn offset(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
        let node = doc.tree.get(id.0).unwrap();
        rinch_dom::paint::ifc_content_box_offset(&doc.tree, node)
    }

    let (xd, xw) = build("grid");
    let (gd, gw) = build("inline-grid");

    assert_eq!(
        offset(&xd, xw),
        (0.0, 0.0),
        "control: a block-level grid container is placed by Taffy, so it is not \
         bridged — the bridge is not simply 'the root's padding'"
    );
    // left border + left padding, top border + top padding.
    assert_eq!(
        offset(&gd, gw),
        (18.0, 12.0),
        "an inline-grid box is placed by the IFC, so paint owes it the same \
         content-box offset as the other two atomic inlines",
    );
    assert_consistent(&gd, "bridged inline-grid");
}

/// The MCP debug surface reports the `display` the node actually has (#607).
///
/// This is the one test of the *reason* `DisplayMode::InlineGrid` is a separate
/// variant rather than a reuse of `InlineBlock`. Every other fixture in this
/// file would pass with `inline-grid` folded into either of the other two atomic
/// inlines: the flow and the bridge read `is_atomic_inline`, which all three
/// answer, and the interior reads `DisplayValue::to_taffy`, which is untouched
/// by the fold. `dom_tree` is what would start lying — it dumps `display_mode`
/// verbatim (`rinch_dom::testing::get_node_detail`), and a debug surface that
/// names the wrong `display` costs an afternoon.
///
/// The three spellings together, so "it always says InlineGrid" cannot pass.
#[test]
fn the_debug_surface_reports_inline_grid_as_its_own_display_mode() {
    for (display, expected) in [
        ("inline-grid", "InlineGrid"),
        ("inline-flex", "InlineFlex"),
        ("inline-block", "InlineBlock"),
        ("grid", "Block"),
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", &format!("display: {display}"));
        doc.resolve_layout(VW, VH);

        let detail =
            rinch_dom::testing::get_node_detail(&doc.tree, w.0).expect("the node is in the tree");
        assert_eq!(
            detail["display_mode"].as_str(),
            Some(expected),
            "display: {display} must be reported as {expected}",
        );
    }
}

/// An `inline-grid` **flex item** is blockified, so nothing in this file applies
/// to it — the guard rail on the other side of #607.
///
/// CSS blockifies the `display` of a flex or grid item (css-display-3 §2.7), so
/// `inline-grid` computes to `grid` there and the box is block-level again.
/// Stylo does that for rinch, which is why the fix needed no flex-item case —
/// but "Stylo does it" is the kind of claim that is worth a fixture rather than
/// a sentence, and this is the fixture: **measured**, `computed_style.display`
/// comes back `Grid` (not `InlineGrid`), `display_mode` comes back `Block`, and
/// the box fills its 400px container instead of shrink-wrapping to its 40px
/// track.
///
/// It is also the answer to the blast-radius question #595 raised for the
/// component library: a component declaring `inline-grid` inside a `Stack`
/// would keep stacking, because a flex item's `inline-grid` is not an atomic
/// inline at all.
#[test]
fn an_inline_grid_flex_item_is_blockified_and_stays_block_level() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column; width: 400px",
    );
    let w = el(
        &mut doc,
        c,
        "span",
        "display: inline-grid; grid-template-columns: 40px",
    );
    el(&mut doc, w, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    let node = doc.tree.get(w.0).unwrap();
    assert_eq!(
        node.computed_style.display,
        rinch_dom::computed_style::DisplayValue::Grid,
        "Stylo blockifies a flex item, so `inline-grid` computes to `grid`",
    );
    assert!(
        !node.display_mode.is_atomic_inline(),
        "a blockified grid item is not an atomic inline (got {:?})",
        node.display_mode,
    );
    assert_eq!(
        lay(&doc, w).2,
        400.0,
        "so it fills its flex container rather than shrink-wrapping to its 40px \
         track (got {:?})",
        lay(&doc, w),
    );
    assert_consistent(&doc, "blockified inline-grid flex item");
}
