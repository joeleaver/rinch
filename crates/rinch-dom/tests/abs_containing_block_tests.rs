//! `position: absolute` resolves against its **containing block**, which is
//! the padding box of the nearest ancestor that establishes one — not the
//! direct parent Taffy resolves an out-of-flow box against (issue #386, the
//! other half of #204's initial-containing-block correction).
//!
//! Every number here is Chrome 153's (`--headless=new`, standards mode,
//! `* { box-sizing: border-box }`, 800x600 window), measured with
//! `getBoundingClientRect` and reported **relative to the containing block's
//! border box** (`CB` below), from the page these fixtures were transcribed
//! from. The scaffold is:
//!
//! ```html
//! <div CB = "position: relative; width: 400px; height: 300px; margin: 13px 0 0 17px">
//!   <div SP = "height: 20px"></div>
//!   <div MID = "width: 300px; height: 200px; margin: 0 0 0 30px">   <!-- static -->
//!     <div style="position: absolute; …"></div>
//! ```
//!
//! so the direct parent is a 300x200 box at `(30, 20)` inside a 400x300
//! containing block, and every expectation is **off the fixed point** where the
//! two coincide: a parent-resolved `inset: 0` is `(30, 20, 300, 200)` here, the
//! containing-block answer `(0, 0, 400, 300)`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const CB: &str = "position: relative; width: 400px; height: 300px; margin: 13px 0 0 17px;";
const SP: &str = r#"<div style="height: 20px"></div>"#;
const MID: &str = "width: 300px; height: 200px; margin: 0 0 0 30px;";
const ABS: &str = "position: absolute;";

struct Case {
    doc: RinchDocument,
    cb: usize,
}

impl Case {
    /// `html` under a plain wrapper under `<body>`. The reference box is the
    /// element carrying `data-cb`.
    fn new(html: &str) -> Self {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(wrap, html);
        doc.append_child(body, wrap);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let cb = one(&doc, "[data-cb]");
        Case { doc, cb }
    }

    /// The scaffold in the module doc: `cb_extra` and `mid_extra` are appended
    /// to the containing block's and the static parent's styles, `inner` is
    /// the parent's content.
    fn scaffold(cb_extra: &str, mid_extra: &str, inner: &str) -> Self {
        Self::new(&format!(
            r#"<div data-cb style="{CB}{cb_extra}">{SP}<div data-mid style="{MID}{mid_extra}">{inner}</div></div>"#
        ))
    }

    /// One absolute box, `data-m="abs"`, as the static parent's only child.
    fn single(abs_style: &str) -> Self {
        Self::scaffold("", "", &abs("abs", abs_style, ""))
    }

    fn id(&self, m: &str) -> usize {
        one(&self.doc, &format!("[data-m={m}]"))
    }

    /// `m`'s on-screen box relative to the reference box's border box.
    fn rect(&self, m: &str) -> [f32; 4] {
        let id = self.id(m);
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, id, 1.0);
        let (cx, cy) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, self.cb, 1.0);
        let l = self.doc.tree.get(id).unwrap().layout;
        [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
    }

    /// The same box in page coordinates (the initial-containing-block cases).
    fn page_rect(&self, m: &str) -> [f32; 4] {
        let id = self.id(m);
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, id, 1.0);
        let l = self.doc.tree.get(id).unwrap().layout;
        [x as f32, y as f32, l.width, l.height]
    }

    /// Lay out again at a different viewport height, so the pass is not
    /// skipped (`resolve_layout` returns early when nothing is dirty) while
    /// no width in the fixture moves.
    fn relayout(&mut self) {
        self.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
    }
}

fn one(doc: &RinchDocument, selector: &str) -> usize {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

fn abs(m: &str, style: &str, inner: &str) -> String {
    format!(r#"<div data-m="{m}" style="{ABS}{style}">{inner}</div>"#)
}

#[track_caller]
fn assert_rect(got: [f32; 4], want: [f32; 4], what: &str) {
    let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01);
    assert!(close, "{what}: got {got:?}, Chrome 153 gives {want:?}");
}

// ── The issue as filed ──────────────────────────────────────────────────────

/// `inset: 0` fills the positioned grandparent, not the static parent
/// (parent-resolved: `(30, 20, 300, 200)`).
#[test]
fn inset_zero_fills_the_positioned_grandparent() {
    let c = Case::single("inset: 0");
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "inset: 0");
}

/// A percentage size is a fraction of the containing block (parent-resolved:
/// 150x100).
#[test]
fn a_percentage_size_is_a_fraction_of_the_containing_block() {
    let c = Case::single("left: 0; top: 0; width: 50%; height: 50%");
    assert_rect(c.rect("abs"), [0.0, 0.0, 200.0, 150.0], "50% x 50%");
}

/// Percentage insets likewise (parent-resolved: `(60, 40)`).
#[test]
fn percentage_insets_resolve_against_the_containing_block() {
    let c = Case::single("left: 10%; top: 10%; width: 40px; height: 30px");
    assert_rect(c.rect("abs"), [40.0, 30.0, 40.0, 30.0], "left/top 10%");
}

/// `right`/`bottom` measure from the containing block's far edges
/// (parent-resolved: `(270, 170)`).
#[test]
fn right_and_bottom_anchor_to_the_containing_block() {
    let c = Case::single("right: 10px; bottom: 20px; width: 50px; height: 30px");
    assert_rect(c.rect("abs"), [340.0, 250.0, 50.0, 30.0], "right/bottom px");

    let c = Case::single("right: 10%; bottom: 10%; width: 25%; height: 30px");
    assert_rect(c.rect("abs"), [260.0, 240.0, 100.0, 30.0], "right/bottom %");
}

/// With both insets `auto` the box keeps its **static** position, which does
/// come from where it would have sat in its parent — so that axis is Taffy's.
#[test]
fn auto_insets_keep_the_static_position() {
    let c = Case::single("width: 50px; height: 30px");
    assert_rect(c.rect("abs"), [30.0, 20.0, 50.0, 30.0], "both axes auto");

    let c = Case::single("left: 12px; width: 50px; height: 30px");
    assert_rect(c.rect("abs"), [12.0, 20.0, 50.0, 30.0], "x inset, y static");
}

// ── The containing block is the PADDING box ─────────────────────────────────

const BOXED: &str = "padding: 15px 25px 35px 45px; border-style: solid; border-color: #000; \
                     border-width: 5px 8px 15px 12px;";

/// Insets and percentages measure from the padding edge: inside the border,
/// across the padding. The padding box here is 380x280 at `(12, 5)`.
#[test]
fn the_containing_block_is_the_padding_box() {
    let c = Case::scaffold(BOXED, "", &abs("abs", "inset: 0", ""));
    assert_rect(c.rect("abs"), [12.0, 5.0, 380.0, 280.0], "inset: 0");

    let c = Case::scaffold(
        BOXED,
        "",
        &abs("abs", "left: 10%; top: 10%; width: 50%; height: 50%", ""),
    );
    assert_rect(c.rect("abs"), [50.0, 33.0, 190.0, 140.0], "percentages");

    let c = Case::scaffold(
        BOXED,
        "",
        &abs(
            "abs",
            "right: 7px; bottom: 9px; width: 50px; height: 30px",
            "",
        ),
    );
    assert_rect(c.rect("abs"), [335.0, 246.0, 50.0, 30.0], "right/bottom");
}

/// A containing block whose height comes from its content (20 + 200 + 38)
/// still has a used height for the box to fill and take a percentage of —
/// which is why the size cannot be known before a first compute.
#[test]
fn an_auto_height_containing_block_is_measured_first() {
    let c = Case::new(&format!(
        r#"<div data-cb style="{CB}height: auto;">{SP}<div style="{MID}">{}{}</div><div style="height: 38px"></div></div>"#,
        abs("abs", "inset: 0", ""),
        abs("half", "left: 0; top: 0; width: 25%; height: 50%", ""),
    ));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 258.0], "inset: 0");
    assert_rect(c.rect("half"), [0.0, 0.0, 100.0, 129.0], "25% x 50%");
}

// ── What sits between the box and its containing block ──────────────────────

/// The parent's own formatting context does not matter once an inset is set.
#[test]
fn a_flex_or_grid_parent_changes_nothing() {
    let c = Case::scaffold("", "display: flex;", &abs("abs", "inset: 0", ""));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "flex parent");

    let c = Case::scaffold(
        "",
        "display: grid;",
        &abs("abs", "left: 10%; top: 10%; right: 10%; bottom: 10%", ""),
    );
    assert_rect(c.rect("abs"), [40.0, 30.0, 320.0, 240.0], "grid parent");
}

/// A static `inline-block` parent is laid out by its own detached compute and
/// positioned by the inline formatting context around it; the box still
/// resolves against the containing block above.
#[test]
fn an_inline_block_parent_changes_nothing() {
    let c = Case::scaffold(
        "",
        "display: inline-block;",
        &format!(
            "{}{}",
            abs("abs", "inset: 0", ""),
            abs(
                "rb",
                "right: 5px; bottom: 6px; width: 20px; height: 10px",
                ""
            )
        ),
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "inset: 0");
    assert_rect(c.rect("rb"), [375.0, 284.0, 20.0, 10.0], "right/bottom");
}

/// A `display: contents` wrapper generates no box, so it is neither the
/// parent nor a containing block.
#[test]
fn a_contents_wrapper_under_the_static_parent_changes_nothing() {
    let c = Case::scaffold(
        "",
        "",
        &format!(
            r#"<div style="display: contents">{}</div>"#,
            abs("abs", "inset: 0", "")
        ),
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "inset: 0");
}

/// The regression guard: a parent that *does* establish a containing block —
/// here by an identity transform (#415) — keeps the box. Taffy's answer.
#[test]
fn a_parent_that_establishes_one_keeps_the_box() {
    let c = Case::scaffold("", "transform: rotate(0deg);", &abs("abs", "inset: 0", ""));
    assert_rect(c.rect("abs"), [30.0, 20.0, 300.0, 200.0], "inset: 0");
}

/// Two static levels, each with its own offset, padding and border: the box
/// is measured from the containing block whatever lies between.
#[test]
fn several_static_levels_between() {
    let c = Case::new(&format!(
        r#"<div data-cb style="{CB}">{SP}<div style="padding: 10px; border: 3px solid #000; width: 320px"><div style="margin: 7px 0 0 9px; height: 50px">{}</div></div></div>"#,
        abs("abs", "left: 5px; top: 6px; width: 50%; height: 20px", ""),
    ));
    assert_rect(c.rect("abs"), [5.0, 6.0, 200.0, 20.0], "two levels");
}

/// A transformed (and an identity-transformed, #415) grandparent is a
/// containing block exactly as a positioned one is.
#[test]
fn a_transformed_grandparent_is_the_containing_block() {
    let c = Case::scaffold(
        "position: static; transform: translate(10px, 5px);",
        "",
        &abs("abs", "inset: 0", ""),
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "translate");

    let c = Case::scaffold(
        "position: static; transform: translate(0);",
        "",
        &abs("abs", "left: 10%; top: 10%; width: 50%; height: 50%", ""),
    );
    assert_rect(c.rect("abs"), [40.0, 30.0, 200.0, 150.0], "identity");
}

/// The containing block may itself be an atomic inline.
#[test]
fn an_inline_block_containing_block() {
    let c = Case::new(&format!(
        r#"<div style="margin: 13px 0 0 17px"><div data-cb style="{CB}display: inline-block; margin: 0;">{SP}<div style="{MID}">{}</div></div></div>"#,
        abs("abs", "inset: 0", ""),
    ));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "inset: 0");
}

// ── Scrolling ───────────────────────────────────────────────────────────────

/// A scroller **between** the box and its containing block does not carry the
/// box: the box is the containing block's content, not the scroller's.
/// Chrome: the same `(5, 6)` and `(375, 274)` at `scrollTop = 40` as at 0.
#[test]
fn a_scroller_between_does_not_carry_the_box() {
    let mut c = Case::scaffold(
        "",
        "overflow: auto; height: 100px;",
        &format!(
            r#"<div style="height: 500px"></div>{}{}"#,
            abs("abs", "left: 5px; top: 6px; width: 20px; height: 20px", ""),
            abs(
                "rb",
                "right: 5px; bottom: 6px; width: 20px; height: 20px",
                ""
            ),
        ),
    );
    assert_rect(c.rect("abs"), [5.0, 6.0, 20.0, 20.0], "unscrolled");
    assert_rect(c.rect("rb"), [375.0, 274.0, 20.0, 20.0], "unscrolled");

    let mid = one(&c.doc, "[data-mid]");
    c.doc.set_scroll_top(NodeId(mid), 40.0);
    // No layout has run since the scroll.
    assert_rect(c.rect("abs"), [5.0, 6.0, 20.0, 20.0], "scrolled, no layout");
    assert_rect(
        c.rect("rb"),
        [375.0, 274.0, 20.0, 20.0],
        "scrolled, no layout",
    );

    c.relayout();
    assert_eq!(c.doc.tree.get(mid).unwrap().scroll_offset.1, 40.0);
    assert_rect(c.rect("abs"), [5.0, 6.0, 20.0, 20.0], "scrolled, laid out");
    assert_rect(
        c.rect("rb"),
        [375.0, 274.0, 20.0, 20.0],
        "scrolled, laid out",
    );
}

/// The containing block's **own** scroll does carry it: the box is part of
/// what that box scrolls. Chrome: `top: 6px` at `scrollTop = 30` is `-24`.
#[test]
fn the_containing_blocks_own_scroll_carries_the_box() {
    let mut c = Case::new(&format!(
        r#"<div data-cb style="{CB}overflow: auto;">{SP}<div style="{MID}">{}</div><div style="height: 500px"></div></div>"#,
        abs("abs", "left: 5px; top: 6px; width: 20px; height: 20px", ""),
    ));
    assert_rect(c.rect("abs"), [5.0, 6.0, 20.0, 20.0], "unscrolled");
    c.doc.set_scroll_top(NodeId(c.cb), 30.0);
    assert_rect(
        c.rect("abs"),
        [5.0, -24.0, 20.0, 20.0],
        "scrolled, no layout",
    );
    c.relayout();
    assert_rect(
        c.rect("abs"),
        [5.0, -24.0, 20.0, 20.0],
        "scrolled, laid out",
    );
}

// ── The box's own content: why the size is known BEFORE its subtree lays out ─

/// A percentage child sees the corrected box, not the parent-resolved one.
#[test]
fn a_percentage_child_sees_the_corrected_box() {
    let c = Case::single_with(
        "inset: 0",
        r#"<div data-m="kid" style="width: 100%; height: 100%"></div>"#,
    );
    assert_rect(c.rect("kid"), [0.0, 0.0, 400.0, 300.0], "100% child");
}

/// Content that wraps: five 130px items in a wrapping row are two rows in a
/// 400px box (3 + 2) and three in a 300px one (2 + 2 + 1), so the box's
/// content-sized height says which width its subtree was laid out at.
#[test]
fn wrapping_content_is_laid_out_at_the_corrected_width() {
    let item = r#"<div style="width: 130px; height: 20px"></div>"#;
    let c = Case::single_with(
        "left: 0; right: 0; top: 0",
        &format!(
            r#"<div data-m="kid" style="display: flex; flex-wrap: wrap">{}</div>"#,
            item.repeat(5)
        ),
    );
    assert_rect(c.rect("kid"), [0.0, 0.0, 400.0, 40.0], "wrapping row");
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 40.0], "the box");
}

/// The containing block is itself an absolute box resolved against a
/// non-parent: the inner one needs the outer one's corrected size.
#[test]
fn a_nested_absolute_resolves_against_the_corrected_outer_one() {
    let inner = format!(
        r#"<div style="width: 60px; height: 40px; margin: 5px 0 0 5px">{}{}</div>"#,
        abs("b", "inset: 0", ""),
        abs("b2", "right: 0; bottom: 0; width: 50%; height: 10%", ""),
    );
    let c = Case::single_with_m(
        "a",
        "left: 10px; top: 10px; width: 50%; height: 50%",
        &inner,
    );
    assert_rect(c.rect("a"), [10.0, 10.0, 200.0, 150.0], "outer");
    assert_rect(c.rect("b"), [10.0, 10.0, 200.0, 150.0], "inner, inset: 0");
    assert_rect(
        c.rect("b2"),
        [110.0, 145.0, 100.0, 15.0],
        "inner, right/bottom",
    );
}

impl Case {
    fn single_with(abs_style: &str, inner: &str) -> Self {
        Self::single_with_m("abs", abs_style, inner)
    }
    fn single_with_m(m: &str, abs_style: &str, inner: &str) -> Self {
        Self::scaffold("", "", &abs(m, abs_style, inner))
    }
}

// ── Margins, padding, min/max on the box itself ─────────────────────────────

/// Margins come out of the space between the insets.
#[test]
fn margins_come_out_of_the_inset_box() {
    let c = Case::single("inset: 0; margin: 10px 20px 30px 40px");
    assert_rect(c.rect("abs"), [40.0, 10.0, 340.0, 260.0], "px margins");
}

/// A percentage margin is a fraction of the containing block's **width**, on
/// both axes (parent-resolved: 30).
#[test]
fn a_percentage_margin_is_of_the_containing_blocks_width() {
    let c = Case::single(
        "left: 0; top: 0; width: 50px; height: 30px; margin-left: 10%; margin-top: 10%",
    );
    assert_rect(c.rect("abs"), [40.0, 40.0, 50.0, 30.0], "10% margins");
}

/// `margin: auto` between two insets centres a sized box in the containing
/// block; one auto margin takes all the free space.
#[test]
fn auto_margins_take_the_free_space_of_the_containing_block() {
    let c = Case::single("inset: 0; width: 100px; height: 50px; margin: auto");
    assert_rect(c.rect("abs"), [150.0, 125.0, 100.0, 50.0], "centred");

    let c =
        Case::single("inset: 0; width: 100px; height: 50px; margin-left: auto; margin-top: auto");
    assert_rect(
        c.rect("abs"),
        [300.0, 250.0, 100.0, 50.0],
        "pushed to the end",
    );
}

/// Percentage padding is of the containing block's width (parent-resolved:
/// 30px a side, a 70x70 box).
#[test]
fn percentage_padding_is_of_the_containing_blocks_width() {
    let c = Case::single_with(
        "left: 0; top: 0; padding: 10%",
        r#"<div style="width: 10px; height: 10px"></div>"#,
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 90.0, 90.0], "10% padding");
}

/// Percentage `max-width` / `min-height` likewise (parent-resolved: 150x100).
#[test]
fn percentage_min_and_max_sizes_are_of_the_containing_block() {
    let c = Case::single(
        "left: 0; top: 0; width: 500px; max-width: 50%; height: 10px; min-height: 50%",
    );
    assert_rect(
        c.rect("abs"),
        [0.0, 0.0, 200.0, 150.0],
        "max-width / min-height",
    );
}

/// A content-sized box anchored by `right` alone.
#[test]
fn a_content_sized_box_anchored_by_right() {
    let c = Case::single_with(
        "right: 10px; top: 0",
        r#"<div style="width: 50px; height: 20px"></div>"#,
    );
    assert_rect(c.rect("abs"), [340.0, 0.0, 50.0, 20.0], "right only");
}

/// `stretch` fills the containing block less the insets.
#[test]
fn stretch_fills_the_containing_block() {
    let c = Case::single("left: 10px; top: 5px; width: stretch; height: stretch");
    assert_rect(c.rect("abs"), [10.0, 5.0, 390.0, 295.0], "stretch");
}

// ── A static inline-block between: the box is in a detached compute ─────────

/// The box's parent is a static 100x50 `inline-block`, which Taffy computes
/// detached from the tree the containing block is in.
#[test]
fn a_box_inside_a_static_inline_block() {
    let c = Case::new(&format!(
        r#"<div data-cb style="{CB}">{SP}<div style="display: inline-block; width: 100px; height: 50px; margin: 0 0 0 30px">{}{}</div></div>"#,
        abs("abs", "inset: 0", ""),
        abs(
            "rb",
            "right: 5px; bottom: 6px; width: 20px; height: 10px",
            ""
        ),
    ));
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "inset: 0");
    assert_rect(c.rect("rb"), [375.0, 284.0, 20.0, 10.0], "right/bottom");
}

// ── The containing block changes size or identity after the first layout ────

/// The containing block is resized by a later style change: the box follows,
/// though nothing about the box itself was restyled.
#[test]
fn the_box_follows_a_resized_containing_block() {
    let mut c = Case::single_with(
        "inset: 0",
        r#"<div data-m="kid" style="width: 50%; height: 50%"></div>"#,
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "before");
    c.doc.set_attribute(
        NodeId(c.cb),
        "style",
        &format!("{CB}width: 500px; height: 260px"),
    );
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [0.0, 0.0, 500.0, 260.0], "after");
    assert_rect(c.rect("kid"), [0.0, 0.0, 250.0, 130.0], "its child");
}

/// The static parent becomes positioned: it is the containing block now, and
/// the box goes back to Taffy's direct-parent answer. And back again.
#[test]
fn the_box_follows_a_change_of_containing_block() {
    let mut c = Case::single("inset: 0");
    let mid = one(&c.doc, "[data-mid]");
    c.doc
        .set_attribute(NodeId(mid), "style", &format!("{MID}position: relative"));
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(
        c.rect("abs"),
        [30.0, 20.0, 300.0, 200.0],
        "parent positioned",
    );

    c.doc.set_attribute(NodeId(mid), "style", MID);
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(
        c.rect("abs"),
        [0.0, 0.0, 400.0, 300.0],
        "parent static again",
    );
}

/// A restyle of the box itself (which rebuilds its Taffy style from the
/// computed values) keeps the correction.
#[test]
fn a_restyle_of_the_box_keeps_the_correction() {
    let mut c = Case::single("inset: 0");
    let id = c.id("abs");
    c.doc.set_attribute(
        NodeId(id),
        "style",
        &format!("{ABS}inset: 0; background: red"),
    );
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "restyled");

    c.doc.set_attribute(
        NodeId(id),
        "style",
        &format!("{ABS}left: 10%; top: 10%; width: 25%; height: 25%"),
    );
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [40.0, 30.0, 100.0, 75.0], "re-inset");
}

/// The same box under a containing block that is the direct parent: nothing
/// to correct, and nothing corrected.
#[test]
fn a_direct_parent_containing_block_is_untouched() {
    let c = Case::new(&format!(
        r#"<div data-cb style="{CB}">{}</div>"#,
        abs("abs", "left: 10%; top: 10%; width: 50%; height: 50%", ""),
    ));
    assert_rect(c.rect("abs"), [40.0, 30.0, 200.0, 150.0], "direct parent");
}

// ── The initial containing block (#204) gets the same margin/padding rules ──

/// Chrome 153, no positioned ancestor, 800px-wide viewport: margins come out
/// of the inset box (`740` wide at `x = 40`), `margin: 0 auto` centres
/// (`x = 350`), `padding: 10%` is 80px a side, `max-width: 50%` is 400.
#[test]
fn the_initial_containing_block_gets_the_same_rules() {
    let icb = |style: &str, inner: &str| {
        Case::new(&format!(
            r#"<div data-cb style="{MID}">{}</div>"#,
            abs("abs", style, inner)
        ))
    };
    let c = icb(
        "left: 0; right: 0; top: 0; height: 40px; margin: 10px 20px 30px 40px",
        "",
    );
    assert_rect(c.page_rect("abs"), [40.0, 10.0, 740.0, 40.0], "margins");

    let c = icb(
        "left: 0; right: 0; top: 0; width: 100px; height: 50px; margin: 0 auto",
        "",
    );
    assert_rect(
        c.page_rect("abs"),
        [350.0, 0.0, 100.0, 50.0],
        "auto margins",
    );

    let c = icb(
        "left: 0; top: 0; padding: 10%",
        r#"<div style="width: 10px; height: 10px"></div>"#,
    );
    assert_rect(c.page_rect("abs"), [0.0, 0.0, 170.0, 170.0], "10% padding");

    let c = icb(
        "left: 0; top: 0; width: 900px; max-width: 50%; height: 10px",
        "",
    );
    assert_rect(c.page_rect("abs"), [0.0, 0.0, 400.0, 10.0], "max-width");
}

// ── More of the scroll rule ─────────────────────────────────────────────────

/// A **static-position** box under a scroller that is not its containing
/// block does not ride the scroller either. Chrome: 70px of content above it
/// in a scroller at `(30, 20)`, `scrollTop = 40` — the box is at `(30, 90)`,
/// not `(30, 50)`.
#[test]
fn a_static_position_box_does_not_ride_a_scroller_between() {
    let mut c = Case::scaffold(
        "",
        "overflow: auto; height: 100px;",
        &format!(
            r#"<div style="height: 70px"></div>{}<div style="height: 500px"></div>"#,
            abs("abs", "width: 20px; height: 20px", ""),
        ),
    );
    assert_rect(c.rect("abs"), [30.0, 90.0, 20.0, 20.0], "unscrolled");
    let mid = one(&c.doc, "[data-mid]");
    c.doc.set_scroll_top(NodeId(mid), 40.0);
    assert_rect(
        c.rect("abs"),
        [30.0, 90.0, 20.0, 20.0],
        "scrolled, no layout",
    );
    c.relayout();
    assert_rect(
        c.rect("abs"),
        [30.0, 90.0, 20.0, 20.0],
        "scrolled, laid out",
    );
}

/// The initial containing block scrolls with the **page**, which in rinch is
/// the `<body>`'s scroll: Chrome moves a `top: 6px` box from 6 to -94 at
/// `scrollY = 100`. A scroller between the box and the page does not carry it.
#[test]
fn an_icb_box_rides_the_page_and_no_scroller_between() {
    let mut c = Case::new(&format!(
        r#"{SP}<div data-cb style="{MID}overflow: auto; height: 100px">{}{}<div style="height: 500px"></div></div><div style="height: 2000px"></div>"#,
        abs("top", "left: 7px; top: 6px; width: 20px; height: 20px", ""),
        abs("static", "width: 20px; height: 20px", ""),
    ));
    assert_rect(c.page_rect("top"), [7.0, 6.0, 20.0, 20.0], "unscrolled");
    assert_rect(
        c.page_rect("static"),
        [30.0, 20.0, 20.0, 20.0],
        "unscrolled",
    );

    c.doc.set_scroll_top(NodeId(c.cb), 40.0);
    assert_rect(
        c.page_rect("top"),
        [7.0, 6.0, 20.0, 20.0],
        "scroller, no layout",
    );
    assert_rect(
        c.page_rect("static"),
        [30.0, 20.0, 20.0, 20.0],
        "scroller, no layout",
    );
    c.relayout();
    assert_rect(
        c.page_rect("top"),
        [7.0, 6.0, 20.0, 20.0],
        "scroller, laid out",
    );
    assert_rect(
        c.page_rect("static"),
        [30.0, 20.0, 20.0, 20.0],
        "scroller, laid out",
    );

    let body = c.doc.body();
    c.doc.set_scroll_top(body, 100.0);
    assert_rect(
        c.page_rect("top"),
        [7.0, -94.0, 20.0, 20.0],
        "page, no layout",
    );
    c.relayout();
    assert_rect(
        c.page_rect("top"),
        [7.0, -94.0, 20.0, 20.0],
        "page, laid out",
    );
    assert_rect(
        c.page_rect("static"),
        [30.0, -80.0, 20.0, 20.0],
        "page, laid out",
    );
}

// ── More of the margin rule, and calc() ─────────────────────────────────────

/// `stretch` leaves room for the margins; one `auto` margin beside a length
/// takes what is left; a box larger than the space is centred on the block
/// axis and starts at the inset on the inline axis. All Chrome 153.
#[test]
fn margins_between_insets() {
    let c = Case::single(
        "left: 10px; top: 5px; width: stretch; height: stretch; margin: 3px 4px 5px 6px",
    );
    assert_rect(c.rect("abs"), [16.0, 8.0, 380.0, 287.0], "stretch");

    let c = Case::single(
        "inset: 0; width: 100px; height: 50px; margin-left: auto; margin-right: 30px; \
         margin-top: 20px; margin-bottom: auto",
    );
    assert_rect(
        c.rect("abs"),
        [270.0, 20.0, 100.0, 50.0],
        "one auto, one length",
    );

    let c = Case::single("inset: 0; width: 500px; height: 350px; margin: auto");
    assert_rect(
        c.rect("abs"),
        [0.0, -25.0, 500.0, 350.0],
        "larger than the space",
    );
}

/// A mixed `calc()` resolves against the containing block wherever a plain
/// percentage does: insets, size, padding, `max-width`.
#[test]
fn calc_resolves_against_the_containing_block() {
    let c = Case::scaffold(
        "",
        "",
        &format!(
            "{}{}",
            abs(
                "abs",
                "left: calc(10% + 5px); top: calc(10% + 5px); width: calc(50% - 20px); \
                 height: calc(50% - 20px)",
                ""
            ),
            abs(
                "pad",
                "right: 0; top: 0; padding: calc(5% + 2px); max-width: calc(25% + 10px); \
                 width: 300px",
                r#"<div style="height: 10px"></div>"#
            ),
        ),
    );
    assert_rect(c.rect("abs"), [45.0, 35.0, 180.0, 130.0], "insets and size");
    assert_rect(
        c.rect("pad"),
        [290.0, 0.0, 110.0, 54.0],
        "padding and max-width",
    );
}

/// A box with no size of its own still has a place: `right: 10px; bottom:
/// 20px` on a 0x0 box is the containing block's corner less the insets.
#[test]
fn a_zero_size_box_is_still_placed() {
    let c = Case::single("right: 10px; bottom: 20px; width: 0; height: 0");
    assert_rect(c.rect("abs"), [390.0, 280.0, 0.0, 0.0], "0x0");
}

// ── Other ways the answer changes with no restyle of the box ────────────────

/// An inline `left`/`top` write takes the inset fast path (#280), which
/// re-cascades nothing: the box moves against its containing block, and a
/// size that depends on the insets follows.
#[test]
fn an_inline_inset_write_moves_the_box_in_its_containing_block() {
    let mut c = Case::single("left: 10px; top: 10px; right: 10px; height: 30px");
    assert_rect(c.rect("abs"), [10.0, 10.0, 380.0, 30.0], "before");
    let id = NodeId(c.id("abs"));
    c.doc.set_style(id, "left", "50px");
    c.doc.set_style(id, "top", "25%");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [50.0, 75.0, 340.0, 30.0], "after");
}

/// The static parent stops generating a box (`display: contents`): the
/// containing block is now the box's layout parent and Taffy's answer is the
/// right one again — and must keep following the containing block's size,
/// which a length baked while it was a grandparent would not.
#[test]
fn the_parent_becoming_display_contents_hands_the_box_back_to_taffy() {
    let mut c = Case::single("left: 0; top: 0; width: 50%; height: 50%");
    assert_rect(c.rect("abs"), [0.0, 0.0, 200.0, 150.0], "grandparent");
    let mid = NodeId(one(&c.doc, "[data-mid]"));
    c.doc.set_attribute(mid, "style", "display: contents");
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [0.0, 0.0, 200.0, 150.0], "now the parent");

    c.doc.set_attribute(
        NodeId(c.cb),
        "style",
        &format!("{CB}width: 300px; height: 100px"),
    );
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(c.rect("abs"), [0.0, 0.0, 150.0, 50.0], "and it follows it");

    c.doc.set_attribute(mid, "style", MID);
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_rect(
        c.rect("abs"),
        [0.0, 0.0, 150.0, 50.0],
        "a grandparent again",
    );
}

/// The box is moved under another containing block.
#[test]
fn a_box_moved_to_another_containing_block() {
    let mut c = Case::new(&format!(
        r#"<div data-cb style="{CB}">{SP}<div data-mid style="{MID}">{}</div></div><div data-other style="position: relative; width: 120px; height: 80px"><div data-inner style="width: 50px; height: 10px; margin-left: 9px"></div></div>"#,
        abs("abs", "right: 0; bottom: 0; width: 50%; height: 50%", ""),
    ));
    assert_rect(c.rect("abs"), [200.0, 150.0, 200.0, 150.0], "first");
    let inner = NodeId(one(&c.doc, "[data-inner]"));
    let other = one(&c.doc, "[data-other]");
    let id = NodeId(c.id("abs"));
    c.doc.append_child(inner, id);
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    c.cb = other;
    assert_rect(c.rect("abs"), [60.0, 40.0, 60.0, 40.0], "second");
}

// ── What it costs ───────────────────────────────────────────────────────────

/// Root computes a layout pass ran.
fn computes(doc: &mut RinchDocument, f: impl FnOnce(&mut RinchDocument)) -> u64 {
    let before = doc.tree.taffy_computes;
    f(doc);
    doc.tree.taffy_computes - before
}

/// The extra compute is paid only when a containing block comes out of the
/// compute at a size the box was not baked for:
///
/// * a box whose size does not depend on its containing block never pays;
/// * a size-dependent one pays once on its first layout, when the containing
///   block has no size yet;
/// * neither a later pass, nor a restyle of the box, nor a resize of something
///   else pays again;
/// * a resize of the containing block pays once.
#[test]
fn the_second_compute_is_paid_only_when_the_containing_block_resizes() {
    // Position only: one compute, ever.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(
            r#"<div data-cb style="{CB}">{SP}<div style="{MID}">{}</div></div>"#,
            abs(
                "abs",
                "right: 10px; bottom: 20px; width: 50px; height: 30px",
                ""
            )
        ),
    );
    doc.append_child(body, wrap);
    assert_eq!(
        computes(&mut doc, |d| d.resolve_layout(800.0, 600.0)),
        1,
        "a position-only box is placed, not re-sized"
    );

    // Size-dependent: two on the first layout, one after.
    let mut c = Case::scaffold("", "", "");
    let mid = NodeId(one(&c.doc, "[data-mid]"));
    let wrap = c.doc.create_element("div");
    c.doc.set_inner_html(
        wrap,
        &abs(
            "abs",
            "inset: 0",
            r#"<div style="width: 50%; height: 50%"></div>"#,
        ),
    );
    c.doc.set_attribute(wrap, "style", "display: contents");
    c.doc.append_child(mid, wrap);
    assert_eq!(
        computes(&mut c.doc, |d| d.resolve_layout(800.0, 600.0)),
        1,
        "added under a containing block that has a size: baked at the style site"
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 400.0, 300.0], "and right");

    let fresh = Case::single("inset: 0");
    assert_eq!(
        fresh.doc.tree.taffy_computes, 2,
        "first layout of the whole document: the containing block had no size to bake"
    );

    assert_eq!(
        computes(&mut c.doc, |d| d.resolve_layout(800.0, 640.0)),
        1,
        "a later pass"
    );
    let id = NodeId(c.id("abs"));
    c.doc
        .set_attribute(id, "style", &format!("{ABS}inset: 0; padding: 3px"));
    assert_eq!(
        computes(&mut c.doc, |d| d.resolve_layout(800.0, 640.0)),
        1,
        "a restyle of the box"
    );
    c.doc
        .set_attribute(NodeId(c.cb), "style", &format!("{CB}width: 420px"));
    assert_eq!(
        computes(&mut c.doc, |d| d.resolve_layout(800.0, 640.0)),
        2,
        "a resize of the containing block"
    );
    assert_rect(c.rect("abs"), [0.0, 0.0, 420.0, 300.0], "followed");
}
