//! #543 — a `display: none` element must not keep the box it had while visible.
//!
//! `mark_inline_descendants`' `InlineFlowRole::NoBox` arm detaches a hidden
//! child's Taffy node from its IFC root, on purpose: one attached child makes
//! that root's measure structurally unreachable under #466's leaf invariant and
//! the container collapses to `h = 0`. What the arm's comment claimed — *"the
//! only geometry this changes is the collapse itself"* — was false.
//! `read_layout_results` walks the **Taffy** tree, so a node that is not in it
//! never has `layout` written again and keeps the box it had when it was last
//! visible.
//!
//! That stale box is then read by everything that reads a box:
//!
//! - **paint** drew it. `paint_node`'s own comment said *"Skip hidden elements
//!   (display: none, style/script tags)"* while testing only the tag — the sole
//!   `DisplayValue::None` check anywhere under `paint/` was in `layer_bounds.rs`.
//! - **hit testing** answered for it. Measured in `ui-zoo-desktop`: after
//!   closing the Dropdown Menu, a click inside the ghost ran the *item's*
//!   handler and changed the app's selection. #543's own text says "the closed
//!   menu takes no clicks"; it took them.
//!
//! Two confident negative claims in two files, each harmless because it relied
//! on the other's guarantee — the stale box nothing draws, the missing test that
//! layout makes unnecessary — composing into a shipped, interactive ghost.
//!
//! **The axis, which is not obvious and is why this hid.** It needs the hidden
//! node's parent to be an **IFC root**. `.rinch-dropdown-menu` is
//! `display: inline-block`; inside a `Stack`/`Group` — i.e. any flex container,
//! which is how the component library lays out — CSS blockifies it to `block`,
//! and as a block whose first child is the `inline-block` target it becomes an
//! IFC root. Under a plain block parent it stays `inline-block`, is itself
//! IFC-owned, and none of this happens. Every fixture below therefore carries an
//! inline sibling, and the last one is the control that does not.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// **Every `y` below is a declared number, not a measured one.**
///
/// The panel is `top: 100%`, so its `y` is the height of the line box above it —
/// and a line box with no declared `line-height` is whatever the **font** makes
/// it. The first version of this file wrote `22.0`, which is what this laptop's
/// fonts give and is not what CI's give: the two preconditions failed there at
/// `19.0` with everything else identical. A literal like that pins the machine,
/// not the behaviour.
///
/// `line-height: 20px` makes the line box exactly 20px, so `y == 20` is a
/// consequence of the stylesheet a reader can check, the way
/// `anon_box_flattened_classification_tests` puts it: one line is *"a 20px
/// question rather than a font question"*. Deriving the expected `y` from the
/// measured line instead would also have gone green, and would have agreed with
/// itself however wrong the line was.
const LINE_BOX: &str = "position: relative; font-size: 16px; line-height: 20px;";

/// The line box `LINE_BOX` declares — and therefore the panel's `top: 100%`.
const LINE: f32 = 20.0;

/// The shape, in four nodes: a `position: relative` box, an inline sibling that
/// makes it an IFC root, and an absolutely positioned panel that starts visible.
///
/// The panel is **absolutely positioned** on purpose: while visible it is
/// `InlineFlowRole::OutOfFlow`, which neither joins a run nor ends the marking
/// pass, so the walk reaches the hidden node instead of `break`ing at a
/// block-level sibling.
fn shape(inline_sibling: bool) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let root = doc.create_element("div");
    doc.set_attribute(root, "style", LINE_BOX);
    doc.append_child(body, root);

    if inline_sibling {
        let sp = doc.create_element("span");
        doc.append_child(root, sp);
        let t = doc.create_text("x");
        doc.append_child(sp, t);
    } else {
        let d = doc.create_element("div");
        doc.set_attribute(d, "style", "height: 10px;");
        doc.append_child(root, d);
    }

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; top: 100%; left: 0; width: 160px; height: 126px; \
         display: block; background: red;",
    );
    doc.append_child(root, panel);

    doc.resolve_layout(VW, VH);
    (doc, root, panel)
}

fn rect(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

fn hide(doc: &mut RinchDocument, id: NodeId) {
    doc.set_style(id, "display", "none");
    doc.resolve_layout(VW, VH);
    // Twice: the ghost was stable across passes, so one pass could not tell a
    // fix from a transient.
    doc.resolve_layout(VW, VH);
}

/// The bug, stated as geometry. Broken: `(0, LINE, 160, 126)` — the box it had
/// while open, kept forever.
#[test]
fn a_hidden_box_under_an_ifc_root_keeps_no_box() {
    let (mut doc, _root, panel) = shape(true);
    assert_eq!(
        rect(&doc, panel),
        (0.0, LINE, 160.0, 126.0),
        "precondition: the panel is laid out below the text line while visible"
    );

    hide(&mut doc, panel);

    assert_eq!(
        rect(&doc, panel),
        (0.0, 0.0, 0.0, 0.0),
        "a `display: none` element generates no box, so it must carry none — \
         this is the box it had while visible, kept because the node was \
         detached from Taffy and `read_layout_results` never visits it again"
    );
}

/// The hidden node's **descendants** must lose their boxes too, and this is not
/// implied by the one above: hit testing descends into a non-clipping node
/// whatever its own bounds (`check_children = !clips_overflow() || in_bounds`),
/// so a zeroed panel over a stale child is still a clickable child. In
/// `ui-zoo-desktop` the click that proved this ran a menu *item*, not the panel.
#[test]
fn the_whole_hidden_subtree_loses_its_boxes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "style", LINE_BOX);
    doc.append_child(body, root);
    let sp = doc.create_element("span");
    doc.append_child(root, sp);
    let t = doc.create_text("x");
    doc.append_child(sp, t);

    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: absolute; top: 100%; left: 0; width: 160px; display: block;",
    );
    doc.append_child(root, panel);
    let item = doc.create_element("div");
    doc.set_attribute(item, "style", "height: 42px; background: red;");
    doc.append_child(panel, item);
    let deeper = doc.create_element("div");
    doc.set_attribute(deeper, "style", "height: 17px;");
    doc.append_child(item, deeper);

    doc.resolve_layout(VW, VH);
    assert_eq!(rect(&doc, item).3, 42.0, "precondition: the item has a box");
    assert_eq!(
        rect(&doc, deeper).3,
        17.0,
        "and so does its own child — 17, not 20, so it cannot be confused with \
         the line box the panel's `top: 100%` resolves against"
    );

    hide(&mut doc, panel);

    assert_eq!(rect(&doc, panel), (0.0, 0.0, 0.0, 0.0), "the panel");
    assert_eq!(
        rect(&doc, item),
        (0.0, 0.0, 0.0, 0.0),
        "the item inside it — a zeroed parent does not stop hit testing \
         descending into a child that still has a box"
    );
    assert_eq!(
        rect(&doc, deeper),
        (0.0, 0.0, 0.0, 0.0),
        "and a grandchild: the walk is the whole subtree, not one level"
    );
}

/// Showing it again must give the boxes back. The detach half of the `NoBox`
/// arm always re-attached correctly; zeroing must not cost that.
#[test]
fn showing_it_again_restores_the_box() {
    let (mut doc, _root, panel) = shape(true);
    hide(&mut doc, panel);
    assert_eq!(rect(&doc, panel), (0.0, 0.0, 0.0, 0.0));

    doc.set_style(panel, "display", "block");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        rect(&doc, panel),
        (0.0, LINE, 160.0, 126.0),
        "reopening must lay it out exactly where it was"
    );
}

/// The control: the identical toggle with a **block** sibling instead of an
/// inline one, so the parent is not an IFC root and the hidden node is never
/// detached. Taffy zeroes it, as it always has. This must stay green and is here
/// to say the fix did not merely paper over the general case — it is the
/// arrangement both of my first two headless mounts used, which is why they
/// missed the bug entirely.
#[test]
fn the_non_ifc_arrangement_was_always_correct() {
    let (mut doc, _root, panel) = shape(false);
    assert_eq!(rect(&doc, panel), (0.0, 10.0, 160.0, 126.0));
    hide(&mut doc, panel);
    assert_eq!(rect(&doc, panel), (0.0, 0.0, 0.0, 0.0));
}

// ── The local pixel oracle ─────────────────────────────────────────────────

/// The symptom is that it is **drawn**. A layout assertion cannot see that, and
/// a whole-screen comparison cannot see a defect this size (see the repo's
/// visual-regression note) — a region whose correct value is a known constant
/// can. The panel is the only red thing on the surface, so correct is provably
/// `0` and broken is provably `160 * 126`.
#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn red_pixels(doc: &mut RinchDocument) -> usize {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        painter
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| **p == [255, 0, 0, 255])
            .count()
    }

    /// **A checker test, and labelled as one.** It corrupts the state by hand
    /// instead of driving the sequence that produces it, which this repo rightly
    /// treats as the weak kind — it tests the checker ("does paint refuse a
    /// hidden node with a box?") and not the producer ("does the engine ever
    /// leave one?"). It is the right shape here anyway, because the property
    /// belongs to paint alone: *a `display: none` element draws nothing,
    /// whatever its `layout` says.*
    ///
    /// It exists because the producer-level fixtures do **not** discriminate
    /// `paint_node`'s `display: none` test. Measured: with that test removed and
    /// the zeroing in `read_layout_results` kept, every other fixture in this
    /// file passes, because there is no longer a stale box for paint to draw.
    /// The two repairs are independent — one keeps boxes honest for every reader
    /// (hit testing included, which paint cannot help), the other makes paint
    /// correct for any node that computes `none` however its box arose — and
    /// this is what stops the second from being unwitnessed code in a change
    /// that is *about* unwitnessed claims.
    ///
    /// Kills: the `DisplayValue::None` early return in `paint_node`.
    #[test]
    fn paint_refuses_a_hidden_node_that_still_carries_a_box() {
        let (mut doc, _root, panel) = shape(true);
        hide(&mut doc, panel);
        assert_eq!(red_pixels(&mut doc), 0, "precondition: gone");

        // Put the box back by hand — the state the engine used to leave behind.
        doc.tree.nodes[panel.0].layout = rinch_dom::LayoutResult {
            x: 0.0,
            y: LINE,
            width: 160.0,
            height: 126.0,
        };

        assert_eq!(
            red_pixels(&mut doc),
            0,
            "paint must refuse a `display: none` node whatever box it carries"
        );
    }

    #[test]
    fn a_hidden_panel_paints_nothing() {
        let (mut doc, _root, panel) = shape(true);
        assert_eq!(
            red_pixels(&mut doc),
            160 * 126,
            "precondition: while visible it fills its whole box"
        );

        hide(&mut doc, panel);

        assert_eq!(
            red_pixels(&mut doc),
            0,
            "a `display: none` element draws nothing"
        );
    }
}
