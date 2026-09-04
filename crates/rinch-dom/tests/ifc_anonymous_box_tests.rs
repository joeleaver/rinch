//! An anonymous block box carries no box model (#319).
//!
//! An IFC anonymous block box used to clone its parent's **entire** computed
//! style while its Taffy style stayed `Default::default()`. Layout saw zero
//! padding; paint saw the parent's — so every consumer of the clone's box
//! model double-counted it: paint's IFC arm displaced the whole inline layout
//! by one parent padding+border, `ifc_content_box_offset` displaced hit
//! testing and caret placement to match, `build_inline_layout`'s `max_width`
//! re-subtracted a padding Taffy had already taken out, and the cloned
//! `background`/`border` decorations were painted a second time at the
//! anonymous box's own rect.
//!
//! The fix is `ComputedStyle::for_anonymous_box`: only what CSS inherits
//! crosses over (CSS 2.1 §9.2.1.1). These tests each name the production
//! mutation they kill; the headline mutant — restoring the full clone in
//! `create_anonymous_block_boxes` — is what `main` shipped, so every test
//! marked with it fails on the pre-fix tree.
//!
//! **Every fixture uses asymmetric, non-zero padding and a border.** A
//! zero-padding container is the one value where the correct offset and the
//! double-counted one agree, and a symmetric padding cannot catch an axis
//! swap.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

/// Rasterise with the software painter — the same `paint_document` entry point
/// the desktop shell's `build_pixels` uses.
fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
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
    painter.pixels().to_vec()
}

fn color_count(px: &[u8], rgb: (u8, u8, u8)) -> u32 {
    let mut n = 0;
    for i in (0..px.len()).step_by(4) {
        if (px[i], px[i + 1], px[i + 2]) == rgb {
            n += 1;
        }
    }
    n
}

fn color_bbox(px: &[u8], rgb: (u8, u8, u8)) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (VW as u32, VH as u32);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (px[i], px[i + 1], px[i + 2]) == rgb {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

/// The container every test builds: asymmetric padding `(10, 20, 30, 40)` and
/// a 2px border, so the content-box offset is `(42, 12)` — different on each
/// axis. Mixed content (a block sibling after the inline run) is what makes
/// `create_anonymous_block_boxes` mint the anonymous root.
const CONTAINER: &str = "width: 400px; padding: 10px 20px 30px 40px; \
                         border: 2px solid rgb(0, 0, 255); font-size: 16px; \
                         line-height: 20px";

/// The anonymous IFC root that owns `child`, asserted to exist.
fn anon_root_of(doc: &RinchDocument, child: NodeId) -> usize {
    let root = doc
        .tree
        .get(child.0)
        .unwrap()
        .ifc_root
        .expect("the child is inline content of some IFC");
    assert!(
        doc.tree.get(root).unwrap().is_anonymous_block_box,
        "mixed content must put the inline run in an anonymous block box"
    );
    root
}

// ── the helper's answer for an anonymous root ───────────────────────────────

/// An anonymous root has no box model, so `ifc_content_box_offset` — the one
/// bridge every geometry consumer (paint, stacking, hit testing, caret
/// placement) reads — answers `(0, 0)` for a box it positions. The inherited
/// half is asserted too: the same clone still carries the parent's text
/// properties, or the fix would trade a phantom box model for lost fonts.
///
/// Kills: restoring the full style clone in `create_anonymous_block_boxes`
/// (the offset reads `(42, 12)` — the parent's padding+border — and paint
/// double-counts it), and dropping the inheritance entirely (font-size 16
/// becomes the default).
#[test]
fn an_anonymous_root_offsets_nothing_and_inherits_text_properties() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", CONTAINER);
    text_in(&mut doc, container, "Press ");
    let button = child_of(&mut doc, container, "button", "width: 60px; height: 24px");
    child_of(&mut doc, container, "div", "height: 20px");
    doc.resolve_layout(VW, VH);

    let anon = anon_root_of(&doc, button);
    let n = doc.tree.get(button.0).unwrap();
    assert_eq!(
        rinch_dom::paint::ifc_content_box_offset(&doc.tree, n),
        (0.0, 0.0),
        "an anonymous root has no padding or border to offset by — a nonzero \
         answer is the parent's box model double-counted (#319)"
    );

    let anon_style = &doc.tree.get(anon).unwrap().computed_style;
    assert_eq!(
        (
            anon_style.padding_left.to_px(),
            anon_style.padding_top.to_px(),
            anon_style.border_left_width.to_px(),
        ),
        (0.0, 0.0, 0.0),
        "no box-model property crosses the clone"
    );
    assert!(
        matches!(
            anon_style.margin_top,
            rinch_dom::computed_style::LengthPercentageAutoValue::Length(v) if v == 0.0
        ),
        "no margin crosses the clone"
    );
    assert_eq!(
        anon_style.background_color(),
        None,
        "no background decoration crosses the clone"
    );
    assert_eq!(
        (anon_style.font_size, anon_style.text_align),
        (
            16.0,
            doc.tree.get(container.0).unwrap().computed_style.text_align
        ),
        "the inherited text properties still cross"
    );
}

// ── paint's IFC arm: the draw lands where layout put it ─────────────────────

/// The inline-block leads its run, so Parley places it at `(0, 0)` of the
/// anonymous root's inline layout and its expected screen origin is the pure
/// layout sum — container origin + the anonymous box's Taffy position + the
/// box's own IFC position. Derived from `layout`, not hard-coded, so the test
/// fails exactly when paint stops agreeing with layout.
///
/// The fill is **translucent** (the #472 oracle): one 50% red over white is
/// `rgb(255,127,127)`, two stacked are `rgb(255,63,63)` — so a second copy is
/// caught even if it lands exactly on the first.
///
/// Kills: restoring the full clone (the draw shifts one padding+border down
/// and right of the layout sum — on the pre-#319 tree this is exactly where
/// it painted), and any regression of paint's IFC arm away from
/// `ifc_root_content_origin`.
#[test]
fn an_inline_block_in_an_anonymous_root_paints_at_its_laid_out_position() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // A white ground: the translucent oracle's arithmetic needs a known
    // backdrop, and the pixmap's own clear is not it.
    doc.set_attribute(body, "style", "background-color: rgb(255, 255, 255)");
    child_of(&mut doc, body, "div", "height: 100px");
    let container = child_of(&mut doc, body, "div", CONTAINER);
    let button = child_of(
        &mut doc,
        container,
        "button",
        "width: 60px; height: 24px; background-color: rgba(255, 0, 0, 0.5)",
    );
    child_of(&mut doc, container, "div", "height: 20px");
    doc.resolve_layout(VW, VH);

    let anon = anon_root_of(&doc, button);
    let (cx, cy) = {
        // The container's own layout origin — body sits at (0, 0), so the
        // body-relative origin is the screen origin. Pure layout, no helper.
        let l = &doc.tree.get(container.0).unwrap().layout;
        (l.x, l.y)
    };
    let (ax, ay) = {
        let l = &doc.tree.get(anon).unwrap().layout;
        (l.x, l.y)
    };
    assert_eq!(
        (ax, ay),
        (42.0, 12.0),
        "Taffy places the anonymous box one real padding+border into the \
         container — the padding is applied exactly once, here"
    );
    let (bx, by) = {
        let l = &doc.tree.get(button.0).unwrap().layout;
        (l.x, l.y)
    };
    let (ex, ey) = (cx + ax + bx, cy + ay + by);
    assert_eq!(
        (ex.fract(), ey.fract()),
        (0.0, 0.0),
        "the expected origin must be integral for an exact bbox comparison — \
         if this fires the fixture needs adjusting, not the tolerance"
    );

    let px = rasterize(&mut doc);
    assert_eq!(
        color_count(&px, (255, 63, 63)),
        0,
        "no pixel composites two 50% draws — the box is drawn once"
    );
    assert_eq!(
        color_bbox(&px, (255, 127, 127)),
        Some((ex as u32, ey as u32, ex as u32 + 60, ey as u32 + 24)),
        "the draw sits at the layout sum"
    );
    assert_ne!(
        color_bbox(&px, (255, 127, 127)),
        Some((
            ex as u32 + 42,
            ey as u32 + 12,
            ex as u32 + 102,
            ey as u32 + 36
        )),
        "one padding+border down-right is the double-counted origin the full \
         clone produced (#319)"
    );
}

// ── the same arm for a real root: one offset, on the right axes ────────────

/// The counterpart witness on a **real** padded root (no block sibling, so the
/// container itself is the IFC root): the draw sits exactly one padding+border
/// in from the layout sum — `(+42, +12)`, each axis its own value. An
/// anonymous root cannot see an axis swap in [`ifc_root_content_origin`] (its
/// offset is `(0, 0)`), and now that paint, stacking and hit testing all read
/// the one helper, a swap moves them **convergently** — the containment tests
/// in `one_draw_per_box_tests` stay green. Only an absolute pixel witness
/// with asymmetric axes can catch it, so this is that witness.
///
/// Kills: paint's IFC arm dropping the offset for real roots, and an axis
/// swap inside `ifc_root_content_origin` itself.
#[test]
fn an_inline_block_in_a_real_root_paints_one_offset_in() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "background-color: rgb(255, 255, 255)");
    child_of(&mut doc, body, "div", "height: 100px");
    let container = child_of(&mut doc, body, "div", CONTAINER);
    let button = child_of(
        &mut doc,
        container,
        "button",
        "width: 60px; height: 24px; background-color: rgba(255, 0, 0, 0.5)",
    );
    text_in(&mut doc, container, " OK");
    doc.resolve_layout(VW, VH);

    let c = doc.tree.get(container.0).unwrap();
    assert!(
        !c.is_anonymous_block_box && c.text_layout.is_some(),
        "all-inline content keeps the container itself the IFC root"
    );
    let (cx, cy) = (c.layout.x, c.layout.y);
    let (bx, by) = {
        let l = &doc.tree.get(button.0).unwrap().layout;
        (l.x, l.y)
    };
    // The layout sum plus the root's real content-box offset, each axis its
    // own value.
    let (ex, ey) = (cx + bx + 42.0, cy + by + 12.0);
    assert_eq!(
        (ex.fract(), ey.fract()),
        (0.0, 0.0),
        "the expected origin must be integral for an exact bbox comparison — \
         if this fires the fixture needs adjusting, not the tolerance"
    );

    let px = rasterize(&mut doc);
    assert_eq!(
        color_bbox(&px, (255, 127, 127)),
        Some((ex as u32, ey as u32, ex as u32 + 60, ey as u32 + 24)),
        "the draw sits one padding+border in from the layout sum"
    );
    assert_ne!(
        color_bbox(&px, (255, 127, 127)),
        Some((
            (cx + bx + 12.0) as u32,
            (cy + by + 42.0) as u32,
            (cx + bx + 72.0) as u32,
            (cy + by + 66.0) as u32
        )),
        "the axes swapped is a different, wrong place"
    );
}

// ── the cloned decorations: painted once, on the parent only ───────────────

/// The parent's `background` and `border` belong to the parent. The old clone
/// carried both onto the anonymous box, and nothing in paint skips an
/// anonymous box, so they were drawn a second time at its rect.
///
/// Background oracle: a 50% green fill composites to `rgb(127,255,127)` over
/// white once and `rgb(63,255,63)` twice — position-independent. Border
/// oracle: an exact pixel count of the opaque ring; the phantom inner ring
/// adds pure-blue pixels however it is placed (its top edge alone is 336
/// integral pixels wide).
///
/// Kills: keeping `background` (the twice-colour appears) or the border
/// widths/styles/colours (the count grows) in the clone.
#[test]
fn the_parents_decorations_are_not_repainted_on_the_anonymous_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "background-color: rgb(255, 255, 255)");
    child_of(&mut doc, body, "div", "height: 100px");
    let container = child_of(
        &mut doc,
        body,
        "div",
        "width: 400px; height: 100px; padding: 10px 20px 30px 40px; \
         border: 2px solid rgb(0, 0, 255); font-size: 16px; line-height: 20px; \
         color: rgb(0, 0, 0); background-color: rgba(0, 255, 0, 0.5)",
    );
    text_in(&mut doc, container, "label");
    child_of(&mut doc, container, "div", "height: 20px");
    doc.resolve_layout(VW, VH);

    let px = rasterize(&mut doc);
    // Non-vacuous: the single-composite colour is really on screen …
    assert!(
        color_count(&px, (127, 255, 127)) > 0,
        "the 50% green background composites over white at all"
    );
    // … and nothing composites it twice, wherever a second copy would land.
    assert_eq!(
        color_count(&px, (63, 255, 63)),
        0,
        "no pixel composites the background twice — the anonymous box must \
         not repaint its parent's background (#319)"
    );

    // The border: the ring is painted (positive control) …
    assert_eq!(
        color_bbox(&px, (0, 0, 255)),
        Some((0, 100, 400, 200)),
        "the parent's own 2px ring spans its border box"
    );
    // … and the interior — everything inside the 2px ring — is provably free
    // of it. The phantom ring the full clone produced sits at the anonymous
    // box's rect, strictly inside this region, wherever text metrics put it.
    let mut interior_blue = 0;
    for y in 103..197u32 {
        for x in 3..397u32 {
            let i = ((y * VW as u32 + x) * 4) as usize;
            if (px[i], px[i + 1], px[i + 2]) == (0, 0, 255) {
                interior_blue += 1;
            }
        }
    }
    assert_eq!(
        interior_blue, 0,
        "no border ink inside the ring — blue there is the anonymous box \
         repainting the parent's border at its own rect (#319)"
    );
}

// ── build_inline_layout's max_width: the phantom no longer re-shrinks it ────

/// Line-breaking width for an anonymous root used to subtract the cloned
/// horizontal padding+border from a width Taffy had **already** shrunk by the
/// real thing — so text in a padded mixed container wrapped one whole
/// horizontal box model early. The twin (same container, no block sibling, so
/// the container itself is the root) breaks lines at the true content width;
/// the anonymous root must agree with it.
///
/// The fixture proves it sits in the discriminating window at runtime: the
/// measured text is asserted **narrower than the true content width** (one
/// line when correct) and **wider than the phantom-shrunk width** (it must
/// wrap under the mutant) — so a font swap cannot quietly park it where
/// correct and broken code agree.
///
/// Kills: restoring the full clone (the mixed run wraps and lays out taller
/// than its twin).
#[test]
fn an_anonymous_root_breaks_lines_at_the_same_width_as_its_twin() {
    // Content width 400 - 100 - 60 - 2*2 = 236; the phantom subtraction is
    // another 164, leaving 72.
    let wide = "width: 400px; padding: 10px 60px 30px 100px; \
                border: 2px solid rgb(0, 0, 255); font-size: 16px; \
                line-height: 20px";
    let text = "wrap me here and now";

    let mut mixed = RinchDocument::new();
    let body = mixed.body();
    let m_container = child_of(&mut mixed, body, "div", wide);
    let m_text = text_in(&mut mixed, m_container, text);
    child_of(&mut mixed, m_container, "div", "height: 20px");
    mixed.resolve_layout(VW, VH);

    let mut twin = RinchDocument::new();
    let body = twin.body();
    let t_container = child_of(&mut twin, body, "div", wide);
    text_in(&mut twin, t_container, text);
    twin.resolve_layout(VW, VH);

    let anon = anon_root_of(&mixed, m_text);
    let anon_node = mixed.tree.get(anon).unwrap();
    let measured = anon_node
        .text_layout
        .as_ref()
        .expect("the anonymous root holds the inline layout")
        .layout
        .width();
    assert!(
        measured < 236.0,
        "the text ({measured}px) must fit the true content width on one line"
    );
    assert!(
        measured > 72.0,
        "the text ({measured}px) must overflow the phantom-shrunk width, or \
         correct and broken line breaking agree and this test is vacuous"
    );

    let mixed_h = anon_node.layout.height;
    let twin_h = {
        let c = twin.tree.get(t_container.0).unwrap();
        assert!(
            !c.is_anonymous_block_box && c.text_layout.is_some(),
            "the twin's container is itself the IFC root"
        );
        c.layout.height - 12.0 - 32.0 // minus (top, bottom) padding+border
    };
    assert_eq!(
        mixed_h, twin_h,
        "the anonymous root breaks lines at the true content width — taller \
         means its max_width re-subtracted the phantom padding (#319)"
    );
}

// ── layer_bounds' inline arms read the same origin ─────────────────────────

/// `opacity_layer_bounds` walks the tree paint walks and unions where each
/// node **draws**; its two inline arms used to hand-roll the content-box sum
/// from the root's style. For an anonymous root whose line fills the content
/// box, the phantom offset pushed the text extent `42px` past the container's
/// right edge — a bounds regression Vello would clip by (tiny-skia ignores
/// the shape, so this is a geometry witness, not an ink one).
///
/// Kills: restoring the full clone, and a `layer_bounds` regression away from
/// `ifc_root_content_origin` that re-adds an offset the anonymous root does
/// not have.
#[test]
fn layer_bounds_keeps_an_anonymous_roots_text_inside_the_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    child_of(&mut doc, body, "div", "height: 100px");
    // 300 wide, content box 300 - 40 - 20 - 2*2 = 236.
    let container = child_of(
        &mut doc,
        body,
        "div",
        "width: 300px; padding: 10px 20px 30px 40px; border: 2px solid black; \
         opacity: 0.5; font-size: 16px; line-height: 20px",
    );
    let block = child_of(
        &mut doc,
        container,
        "span",
        "display: inline-block; width: 236px; height: 30px",
    );
    child_of(&mut doc, container, "div", "height: 20px");
    doc.resolve_layout(VW, VH);
    let _ = anon_root_of(&doc, block);

    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, container.0, 1.0);
    let h = doc.tree.get(container.0).unwrap().layout.height as f64;
    let bounds = rinch_dom::paint::opacity_layer_bounds(&doc.tree, container.0, 1.0, x, y);
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (x, y, x + 300.0, y + h),
        "everything this layer draws sits inside the container's border box — \
         a wider answer is the phantom content-box offset pushing the inline \
         extent past the edge (#319)"
    );
    assert_ne!(
        bounds.x1,
        x + 342.0,
        "300 + the phantom 42 is the pre-#319 right edge"
    );
}

// ── the selection-highlight block reads the same origin ────────────────────

/// The read-only selection highlight is painted for a **real** root carrying
/// `data-text-sel` (an anonymous box never has attributes), so this pins the
/// routed site's behaviour where it is reachable: the highlight starts at the
/// content-box origin, inside the padding — never at the border box.
///
/// The highlight is a translucent blue over white; rather than exact-matching
/// a colour through the blend, the probe classifies "distinctly blue" pixels,
/// which glyph anti-aliasing (grey) never produces.
///
/// Kills: the selection block dropping the `ifc_root_content_origin` offset —
/// the highlight shifts into the padding strip, which is provably empty of
/// blue today.
#[test]
fn the_selection_highlight_starts_inside_the_padding() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "background-color: rgb(255, 255, 255)");
    child_of(&mut doc, body, "div", "height: 100px");
    let container = child_of(&mut doc, body, "div", CONTAINER);
    text_in(&mut doc, container, "select me");
    doc.set_attribute(container, "data-text-sel", "true");
    doc.set_attribute(container, "data-text-sel-start", "0");
    doc.set_attribute(container, "data-text-sel-end", "9");
    doc.resolve_layout(VW, VH);

    let px = rasterize(&mut doc);
    let is_blueish = |x: u32, y: u32| {
        let i = ((y * VW as u32 + x) * 4) as usize;
        let (r, _g, b) = (px[i], px[i + 1], px[i + 2]);
        b > 230 && r < 210 && (b as i16 - r as i16) > 40
    };
    // The line box spans y = 112..132 (container at y=100, offset (42, 12),
    // line-height 20). The left padding strip x = 3..41 sits between the
    // container's border and its content box; the border itself is blue, so
    // the probe starts inside it.
    let strip_hit = (3..41).any(|x| (112..132).any(|y| is_blueish(x, y)));
    assert!(
        !strip_hit,
        "no highlight ink in the padding strip — blue there means the \
         selection block dropped the content-box origin"
    );
    let content_hit = (42..120).any(|x| (112..132).any(|y| is_blueish(x, y)));
    assert!(
        content_hit,
        "the highlight is painted at all, starting at the content origin"
    );
}
