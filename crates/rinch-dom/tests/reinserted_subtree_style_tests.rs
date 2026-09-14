//! A subtree re-inserted under a different ancestor chain re-shapes its text
//! (issue #654).
//!
//! A `Node`'s `text_layout` is a **derived** value: the font family, size,
//! weight, line height and colour it was built from are baked into the shaped
//! glyphs and the brush, and nothing about the layout records which style
//! produced it. `build_ifc_layouts` re-serves an existing layout to any IFC
//! root whose `max_width` is unchanged, so a recascade that changes any of
//! those and does not *say* so leaves the old glyphs on screen — the computed
//! style reads `monospace` and the glyphs are the sans-serif ones the subtree
//! was shaped with under its old parent.
//!
//! Reported against a reactive `if`/`else` that swaps a panel out and back
//! (`show_dom`), which is the last fixture here. That path reaches the defect
//! by a longer route: while the branch sits **detached**, a stale `style_roots`
//! entry re-cascades it with no parent style, so it computes the *initial*
//! values — `font-family` computes to `serif`, which is where the report's
//! "falls back to serif" comes from — and `build_ifc_layouts` reshapes it in
//! serif before it comes back. Re-insertion then restored the computed styles
//! and not the glyphs. The first fixtures reproduce the same loss with no
//! reactivity, no detachment and no `show_dom` at all, which is what says the
//! defect is in the restyle and not in the conditional.
//!
//! **Every assertion is against an oracle built in the same document**, never
//! against a measured literal: a width measured from text pins the host's font
//! set, and the question here is only whether two identical subtrees under the
//! same parent shape identically. Each fixture also carries the **counter-
//! oracle** — the same content under the *other* parent — so a host whose two
//! generic families resolve to one face fails loudly instead of passing
//! vacuously.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

/// Two ancestors that differ **only** in an inherited typography property that
/// Taffy cannot see. `font-family` is the discriminator on purpose: a
/// `font-size` change rewrites the Taffy style and would be re-measured by
/// another route, so a fixture built on it would pass against the defect.
/// `font-size` and `line-height` are declared so no line box here is derived
/// from a font metric.
const CSS: &str = "
    .sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .mono { font-family: monospace;  font-size: 16px; line-height: 20px; }
    .red  { font-family: sans-serif; font-size: 16px; line-height: 20px; color: rgb(255, 0, 0); }
    .blue { font-family: sans-serif; font-size: 16px; line-height: 20px; color: rgb(0, 0, 255); }
";

/// `div.panel > div.mid > div.deep > "hello"`, appended to `parent`.
///
/// The IFC root that holds the shaped text (`.deep`) is deliberately a
/// **grandchild** of the moved node, not the moved node itself: the restyle
/// walks the whole subtree, so a fixture whose text lives on the moved node
/// alone does not distinguish "the subtree was re-shaped" from "the one node
/// that was handed to `append_child` was".
fn panel(doc: &mut RinchDocument, parent: NodeId) -> NodeId {
    let panel = doc.create_element("div");
    let mid = doc.create_element("div");
    let deep = doc.create_element("div");
    let text = doc.create_text("hello");
    doc.append_child(deep, text);
    doc.append_child(mid, deep);
    doc.append_child(panel, mid);
    doc.append_child(parent, panel);
    deep
}

/// The width of the Parley layout an IFC root actually holds — the shaped
/// glyphs, not the box Taffy gave them.
fn shaped_width(doc: &RinchDocument, ifc_root: NodeId) -> f32 {
    doc.tree
        .get(ifc_root.0)
        .expect("node is live")
        .text_layout
        .as_ref()
        .expect("the IFC root holds a shaped layout")
        .layout
        .width()
}

/// The brush the first glyph run was shaped with — `color` is baked into the
/// layout, so it is stale in exactly the same way the font is.
fn shaped_color(doc: &RinchDocument, ifc_root: NodeId) -> peniko::Brush {
    let layout = &doc
        .tree
        .get(ifc_root.0)
        .expect("node is live")
        .text_layout
        .as_ref()
        .expect("the IFC root holds a shaped layout")
        .layout;
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(run) = item {
                return run.style().brush.clone();
            }
        }
    }
    panic!("the layout has at least one glyph run");
}

fn computed_font_family(doc: &RinchDocument, id: NodeId) -> String {
    doc.tree
        .get(id.0)
        .expect("node is live")
        .computed_style
        .font_family
        .clone()
}

/// Moving a subtree from a `sans-serif` ancestor to a `monospace` one reshapes
/// its text in the new font.
///
/// This is #654 with the reactivity taken away. `append_child` re-cascades the
/// moved subtree — the computed `font-family` was always right — and used to
/// leave the Parley layout alone, so the glyphs stayed the ones the subtree was
/// shaped with under `.sans`.
///
/// Kills the mutant that drops `invalidate_ifc_for_node` from
/// `apply_stylo_styles_to_taffy` (verified: the moved width comes back equal to
/// the `.sans` oracle instead of the `.mono` one).
#[test]
fn a_subtree_moved_to_a_new_parent_reshapes_its_text() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();

    let sans = doc.create_element("div");
    doc.set_attribute(sans, "class", "sans");
    doc.append_child(body, sans);
    let mono = doc.create_element("div");
    doc.set_attribute(mono, "class", "mono");
    doc.append_child(body, mono);

    let moved = panel(&mut doc, sans);
    let sans_oracle = panel(&mut doc, sans);
    doc.resolve_layout(800.0, 600.0);

    let under_sans = shaped_width(&doc, moved);
    assert_eq!(
        under_sans,
        shaped_width(&doc, sans_oracle),
        "positive control: two identical subtrees under .sans shape identically"
    );

    // Move the whole panel under the monospace ancestor.
    doc.append_child(
        mono,
        doc.parent_node(doc.parent_node(moved).unwrap()).unwrap(),
    );
    let mono_oracle = panel(&mut doc, mono);
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        computed_font_family(&doc, moved),
        "monospace",
        "the recascade always got the computed style right — that was never the bug"
    );

    let under_mono = shaped_width(&doc, mono_oracle);
    assert_ne!(
        under_sans, under_mono,
        "counter-oracle: this host's sans-serif and monospace must differ, \
         or nothing below can discriminate"
    );
    assert_eq!(
        shaped_width(&doc, moved),
        under_mono,
        "the moved subtree must shape its text the way a subtree built under \
         .mono does; equal to {under_sans} would be the old sans-serif glyphs"
    );
}

/// The same loss through `color`, which reaches the layout as the glyph brush
/// and reaches Taffy not at all.
///
/// A second field pin: a `same_text_layout_inputs` that compared only the font
/// would pass the fixture above and fail this one.
#[test]
fn a_subtree_moved_to_a_new_parent_reshapes_its_brush() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();

    let red = doc.create_element("div");
    doc.set_attribute(red, "class", "red");
    doc.append_child(body, red);
    let blue = doc.create_element("div");
    doc.set_attribute(blue, "class", "blue");
    doc.append_child(body, blue);

    let moved = panel(&mut doc, red);
    doc.resolve_layout(800.0, 600.0);
    let red_brush = shaped_color(&doc, moved);

    doc.append_child(
        blue,
        doc.parent_node(doc.parent_node(moved).unwrap()).unwrap(),
    );
    let blue_oracle = panel(&mut doc, blue);
    doc.resolve_layout(800.0, 600.0);

    let blue_brush = shaped_color(&doc, blue_oracle);
    assert_ne!(
        format!("{red_brush:?}"),
        format!("{blue_brush:?}"),
        "counter-oracle: the two classes must paint different brushes"
    );
    assert_eq!(
        format!("{:?}", shaped_color(&doc, moved)),
        format!("{blue_brush:?}"),
        "the moved subtree must carry the brush its new ancestor gives it"
    );
}

/// The Taffy half: an atomic inline shrink-wraps to its **measured** text, and
/// that measurement is as derived from the typography as the glyphs are.
///
/// `append_child` marks only the node it is handed dirty, and Taffy dirt
/// propagates up rather than down, so an `inline-block` inside a moved subtree
/// kept the intrinsic width it was measured at under the old font. Clearing the
/// layout alone does not fix this one — `invalidate_ifc_for_node` also marks
/// the root's measure leaf (#466), which is what makes the box re-measure.
#[test]
fn a_moved_inline_block_is_remeasured_in_its_new_font() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();

    let sans = doc.create_element("div");
    doc.set_attribute(sans, "class", "sans");
    doc.append_child(body, sans);
    let mono = doc.create_element("div");
    doc.set_attribute(mono, "class", "mono");
    doc.append_child(body, mono);

    fn inline_block(doc: &mut RinchDocument, parent: NodeId) -> (NodeId, NodeId) {
        let wrapper = doc.create_element("div");
        let ib = doc.create_element("div");
        doc.set_attribute(ib, "style", "display: inline-block");
        let text = doc.create_text("hello");
        doc.append_child(ib, text);
        doc.append_child(wrapper, ib);
        doc.append_child(parent, wrapper);
        (wrapper, ib)
    }

    let (wrapper, moved) = inline_block(&mut doc, sans);
    doc.resolve_layout(800.0, 600.0);
    let under_sans = doc.tree.get(moved.0).unwrap().layout.width;

    doc.append_child(mono, wrapper);
    let (_, mono_oracle) = inline_block(&mut doc, mono);
    doc.resolve_layout(800.0, 600.0);

    let under_mono = doc.tree.get(mono_oracle.0).unwrap().layout.width;
    assert_ne!(
        under_sans, under_mono,
        "counter-oracle: the two families must measure to different widths"
    );
    assert_eq!(
        doc.tree.get(moved.0).unwrap().layout.width,
        under_mono,
        "the moved inline-block must shrink-wrap to its text in the NEW font; \
         {under_sans} would be the width it was measured at under .sans"
    );
}

/// #654 as reported: a reactive `if`/`else` swaps a panel out and back.
///
/// The branch body is a subtree built **once** outside the conditional and
/// handed back by the then-arm, which is the shape the report was filed
/// against. Nothing about the ancestor chain changes across the round trip —
/// the damage happens while the panel is detached, where a stale `style_roots`
/// entry re-cascades it against no parent at all and it computes the initial
/// `font-family: serif`, then gets re-shaped in serif before it comes back.
///
/// The assertion is that a round trip changes nothing, with the initial mount
/// as its own oracle.
///
/// **It is not the pin for any one field of `same_text_layout_inputs`**, and
/// that is worth saying: the detached cascade lands on the *initial* value of
/// every inherited property at once, so this fixture survives a predicate that
/// has dropped `font-family` (the `color` clause still catches the round trip)
/// and vice versa. The two `moved_to_a_new_parent` fixtures above are the
/// per-field pins, because there exactly one property differs between the old
/// ancestor and the new one.
#[test]
fn a_branch_swapped_out_and_back_keeps_its_font() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();

    let mut scope = RenderScope::new(dyn_doc, body);
    let root = scope.create_element("div");
    root.set_attribute("class", "mono");
    scope.parent().append_child(&root);

    // Built once, outside the conditional, and returned by the then-branch.
    let built: NodeHandle = {
        let panel = scope.create_element("div");
        let mid = scope.create_element("div");
        let deep = scope.create_element("div");
        let text = scope.create_text("hello");
        deep.append_child(&text);
        mid.append_child(&deep);
        panel.append_child(&mid);
        panel
    };
    let ifc_root = NodeId(built.children()[0].children()[0].node_id().0);

    let showing = Signal::new(true);
    let then_branch = built.clone();
    let _marker = rinch_core::show::show_dom(
        &mut scope,
        &root,
        move || showing.get(),
        move |_: &mut RenderScope| then_branch.clone(),
        Some(|s: &mut RenderScope| {
            let other = s.create_element("div");
            let text = s.create_text("search");
            other.append_child(&text);
            other
        }),
    );

    doc.borrow_mut().resolve_layout(800.0, 600.0);
    let at_mount = shaped_width(&doc.borrow(), ifc_root);

    showing.set(false);
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    showing.set(true);
    doc.borrow_mut().resolve_layout(800.0, 600.0);

    assert_eq!(
        computed_font_family(&doc.borrow(), ifc_root),
        "monospace",
        "the computed style survives the round trip — it always did"
    );
    assert_eq!(
        shaped_width(&doc.borrow(), ifc_root),
        at_mount,
        "and so must the glyphs it was shaped with"
    );
}

/// The gate itself, at the level it is written: two styles that differ only in
/// a property a Parley layout is built from are not interchangeable.
///
/// `apply_stylo_styles_to_taffy` invalidates **only** when this answers false,
/// and that is load-bearing rather than an optimisation — invalidating
/// unconditionally re-shapes every moved subtree's text on every DOM insertion,
/// measured 2-9x slower on a 500-row keyed reversal where no typography changes
/// at all. A field dropped from the predicate is therefore a silent return of
/// this bug for that property, with nothing above it to notice.
#[test]
fn the_staleness_gate_sees_a_font_family_and_a_colour_change() {
    use rinch_dom::computed_style::ComputedStyle;

    let base = ComputedStyle::default();

    assert!(
        base.same_text_layout_inputs(&base.clone()),
        "a style is interchangeable with itself, or nothing is ever reused"
    );

    let mut other_family = base.clone();
    other_family.font_family = "monospace".to_string();
    assert!(
        !base.same_text_layout_inputs(&other_family),
        "font-family reaches the shaped glyphs"
    );

    let mut other_colour = base.clone();
    other_colour.color = Some(peniko::color::AlphaColor::from_rgba8(1, 2, 3, 255));
    assert!(
        !base.same_text_layout_inputs(&other_colour),
        "colour reaches the layout as the glyph brush"
    );

    let mut other_size = base.clone();
    other_size.font_size = base.font_size + 1.0;
    assert!(
        !base.same_text_layout_inputs(&other_size),
        "font-size reaches the shaped glyphs"
    );

    // A property outside the list must NOT force a rebuild — without this the
    // predicate could be `false` unconditionally and every fixture above would
    // still pass, at the cost the gate exists to avoid.
    let mut other_background = base.clone();
    other_background.background = rinch_dom::computed_style::BackgroundValue::Color(
        peniko::color::AlphaColor::from_rgba8(1, 2, 3, 255),
    );
    assert!(
        base.same_text_layout_inputs(&other_background),
        "a background change bakes into no glyph and must not re-shape anything"
    );
}
