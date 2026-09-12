//! #595 — `display: inline-flex` is an **atomic inline**, not a block-level box.
//!
//! An atomic inline (css-display-3 §2.6) is inline-level on the outside — it
//! joins the line around it and shrink-wraps — while its inside is an
//! independent formatting context the IFC only measures and places.
//! `inline-block` and `inline-flex` are both one; rinch used to map
//! `DisplayValue::InlineFlex` onto `DisplayMode::Flex`, the same value plain
//! `display: flex` gets, so an `inline-flex` box **ended the run it should have
//! joined** and filled its container instead of shrink-wrapping.
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

/// `display: inline-grid` is the **same defect and is not fixed** — measured,
/// not assumed, in both directions.
///
/// Chrome 150 on this exact shape: `inline-grid` gives **H = 50**, the same as
/// `inline-block` and `inline-flex`, with the wrapper 25.78px wide at x = 45.38.
/// rinch gives **H = 90** and a 400px wrapper, i.e. the issue's symptom
/// unchanged by #595 — because the value does not survive as far as
/// `DisplayMode`: `display_from_stylo` folds `(Inline, Grid)` into
/// `DisplayValue::Grid` (`computed_style/from_stylo/layout.rs`, comment
/// `// inline-grid`), so by the time `style_resolution` classifies it there is
/// nothing left to tell it from `display: grid`. Fixing it needs a
/// `DisplayValue::InlineGrid` variant first, which is #607's job, not this
/// file's.
///
/// A third spelling is differently wrong and is #607's too: `DisplayValue::parse`
/// — the non-Stylo path in `computed_style/values.rs` — has no `inline-grid` arm
/// at all, so it falls through to `Self::default()`, which is `Flex`.
#[test]
#[ignore = "#607: inline-grid is folded into DisplayValue::Grid before it can be classified"]
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
