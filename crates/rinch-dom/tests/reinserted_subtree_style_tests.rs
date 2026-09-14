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
/// layout alone does not fix this one: `invalidate_ifc_for_node`'s plain
/// `taffy.mark_dirty(root)` is what makes the box re-measure, and deleting that
/// one line kills this fixture and nothing else in the crate. It is **not** the
/// `mark_ifc_measure_dirty` three lines below it — an atomic inline has no #466
/// measure leaf; the fixture that witnesses *that* call is
/// [`an_ifc_root_with_a_measure_leaf_is_remeasured_in_its_new_font`].
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
/// a property an `InlineLayout` is built from are not interchangeable, and two
/// that differ only outside that list are.
///
/// `apply_stylo_styles_to_taffy` invalidates **only** when this answers false,
/// so a field dropped from the predicate is a silent return of this bug for
/// that property, with nothing above it to notice. A predicate that always
/// answers false has nothing above it to notice either, and for the opposite
/// reason: it is *correct*, merely wasteful, so no behavioural fixture can see
/// it. This test is that mutant's only pin, and the waste it guards against is
/// argued mechanically rather than measured — see
/// `tests/restyle_invalidation_bench.rs`, where the gap an earlier revision of
/// #654 claimed did not reproduce.
#[test]
fn the_staleness_gate_lists_what_an_inline_layout_is_built_from() {
    use rinch_dom::computed_style::{ComputedStyle, DisplayValue};

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

    // `background-color` bakes into `InlineLayout::background_spans`, but only
    // an inline box ever contributes one — so the same change is a rebuild on
    // an inline and free on a block. Both halves are asserted: without the
    // second, comparing the six span inputs unconditionally would pass.
    let mut inline_base = base.clone();
    inline_base.display = DisplayValue::Inline;
    let mut inline_bg = inline_base.clone();
    inline_bg.background = rinch_dom::computed_style::BackgroundValue::Color(
        peniko::color::AlphaColor::from_rgba8(1, 2, 3, 255),
    );
    assert!(
        !inline_base.same_text_layout_inputs(&inline_bg),
        "an inline box's background-color bakes into an InlineBackgroundSpan"
    );

    let mut block_base = base.clone();
    block_base.display = DisplayValue::Block;
    let mut block_bg = block_base.clone();
    block_bg.background = rinch_dom::computed_style::BackgroundValue::Color(
        peniko::color::AlphaColor::from_rgba8(1, 2, 3, 255),
    );
    assert!(
        block_base.same_text_layout_inputs(&block_bg),
        "a block's background reaches no span; re-shaping its label on every \
         :hover is the cost the gate exists to avoid"
    );

    // And one property no producer reads at all, so the out-of-list side of the
    // rule has a witness that is not about `display`.
    let mut other_cursor = base.clone();
    other_cursor.cursor = rinch_dom::computed_style::CursorValue::Pointer;
    assert!(
        base.same_text_layout_inputs(&other_cursor),
        "cursor bakes into nothing"
    );
}

/// An inline box's `background-color`, padding and `border-radius` are baked
/// into `InlineLayout::background_spans`, so they go stale on a move exactly
/// the way the glyphs do.
///
/// Found by this PR's reviewer, and **pre-existing** rather than a regression:
/// it fails identically with `crates/rinch-dom/src` at `21fafff`. It is the
/// same defect as the fixtures above reached through a different producer, so
/// it is fixed and pinned here rather than filed.
///
/// Kills a `same_text_layout_inputs` that omits the background-span inputs, and
/// one that compares them but only when neither style is `display: inline`.
#[test]
fn a_moved_inline_boxs_background_span_is_rebuilt() {
    const SPAN_CSS: &str = "
        .a { font-family: sans-serif; font-size: 16px; line-height: 20px; }
        .b { font-family: sans-serif; font-size: 16px; line-height: 20px; }
        .a span.hl { background-color: rgb(255, 0, 0); }
        .b span.hl { background-color: rgb(0, 0, 255); }
    ";

    /// `panel > block > ("aa ", span.hl > "bb", " cc")`; returns the panel and
    /// the IFC root that holds the spans.
    fn panel_with_span(doc: &mut RinchDocument, parent: NodeId) -> (NodeId, NodeId) {
        let panel = doc.create_element("div");
        let block = doc.create_element("div");
        let before = doc.create_text("aa ");
        let span = doc.create_element("span");
        doc.set_attribute(span, "class", "hl");
        let inner = doc.create_text("bb");
        doc.append_child(span, inner);
        let after = doc.create_text(" cc");
        doc.append_child(block, before);
        doc.append_child(block, span);
        doc.append_child(block, after);
        doc.append_child(panel, block);
        doc.append_child(parent, panel);
        (panel, block)
    }

    fn span_colours(doc: &RinchDocument, ifc_root: NodeId) -> Vec<String> {
        doc.tree
            .get(ifc_root.0)
            .expect("node is live")
            .text_layout
            .as_ref()
            .expect("the IFC root holds a shaped layout")
            .background_spans
            .iter()
            .map(|s| format!("{:?}", s.color))
            .collect()
    }

    let mut doc = RinchDocument::new();
    doc.load_css(SPAN_CSS);
    let body = doc.body();

    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "a");
    doc.append_child(body, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "class", "b");
    doc.append_child(body, b);

    let (moved_panel, moved_block) = panel_with_span(&mut doc, a);
    doc.resolve_layout(800.0, 600.0);
    let under_a = span_colours(&doc, moved_block);
    assert!(!under_a.is_empty(), "the inline box must produce a span");

    doc.append_child(b, moved_panel);
    let (_, b_oracle) = panel_with_span(&mut doc, b);
    doc.resolve_layout(800.0, 600.0);

    let under_b = span_colours(&doc, b_oracle);
    assert_ne!(
        under_a, under_b,
        "counter-oracle: the two classes must paint different span colours"
    );
    assert_eq!(
        span_colours(&doc, moved_block),
        under_b,
        "the moved subtree's inline background span must be the new one"
    );
}

/// The witness for `invalidate_ifc_for_node`'s **second** Taffy mark, the one
/// that reaches the #466 measure leaf.
///
/// An IFC root only owns a measure leaf when it has an out-of-flow child, and
/// the leaf only survives a layout pass that does not rebuild the IFC structure
/// (a structural pass tears the leaves down and mints them fresh and dirty). So
/// the shape that needs the extra mark is: an IFC root with an absolutely
/// positioned child, whose inherited font changes on a pass that recomputes
/// Taffy but leaves `ifc_dirty` false. Deleting `mark_ifc_measure_dirty` from
/// `invalidate_ifc_for_node` leaves every other test in the crate green and
/// fails this one at 40 against 80; deleting the plain `taffy.mark_dirty` above
/// it leaves *this* one green and fails the inline-block fixture instead.
///
/// **The restyle deliberately also changes `padding`.** A font change alone
/// never sets `layout_dirty` — that flag is set when a *Taffy* style changes —
/// so `resolve_layout` takes its text-only branch, rebuilds the Parley layout
/// and never re-runs Taffy at all, leaving the box stale whatever this PR does.
/// That is pre-existing and separate (see the PR body); the padding is what
/// puts the pass on the branch where the measure leaf is the deciding factor.
#[test]
fn an_ifc_root_with_a_measure_leaf_is_remeasured_in_its_new_font() {
    // 100px is chosen so `hello world` fits on one line in sans-serif and not
    // in monospace, which is what makes the line count — and so the measured
    // height — differ by font with `line-height` still declared.
    const LEAF_CSS: &str = "
        .sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
        .mono { font-family: monospace;  font-size: 16px; line-height: 20px; padding: 1px; }
        .ifc  { width: 100px; }
        .oof  { position: absolute; top: 0; left: 0; width: 5px; height: 5px; }
    ";

    fn build(doc: &mut RinchDocument, root_class: &str) -> (NodeId, NodeId) {
        let body = doc.body();
        let root = doc.create_element("div");
        doc.set_attribute(root, "class", root_class);
        doc.append_child(body, root);
        let ifc = doc.create_element("div");
        doc.set_attribute(ifc, "class", "ifc");
        doc.append_child(root, ifc);
        // The out-of-flow child is what makes `setup_inline_formatting_contexts`
        // give this root a measure leaf instead of measuring it directly.
        let oof = doc.create_element("div");
        doc.set_attribute(oof, "class", "oof");
        doc.append_child(ifc, oof);
        let text = doc.create_text("hello world hello world");
        doc.append_child(ifc, text);
        (root, ifc)
    }

    let mut doc = RinchDocument::new();
    doc.load_css(LEAF_CSS);
    let (root, ifc) = build(&mut doc, "sans");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&ifc.0),
        "positive control: this shape must actually own a measure leaf, or the \
         fixture pins nothing"
    );
    let leaf_before = doc.tree.ifc_measure_leaves.get(&ifc.0).copied();
    let under_sans = doc.tree.get(ifc.0).unwrap().layout.height;

    // A restyle with no DOM mutation: `ifc_dirty` stays false, so the leaf is
    // the same Taffy node afterwards and nothing but the mark can dirty it.
    doc.set_attribute(root, "class", "mono");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.ifc_measure_leaves.get(&ifc.0).copied(),
        leaf_before,
        "positive control: the leaf must survive the pass, or a fresh dirty leaf \
         would pass this fixture for the wrong reason"
    );

    // Oracle: the same document built as monospace from the start.
    let mut oracle_doc = RinchDocument::new();
    oracle_doc.load_css(LEAF_CSS);
    let (_, oracle_ifc) = build(&mut oracle_doc, "mono");
    oracle_doc.resolve_layout(800.0, 600.0);
    let under_mono = oracle_doc.tree.get(oracle_ifc.0).unwrap().layout.height;

    assert_ne!(
        under_sans, under_mono,
        "counter-oracle: the two families must wrap to different line counts"
    );
    assert_eq!(
        doc.tree.get(ifc.0).unwrap().layout.height,
        under_mono,
        "the restyled IFC root must be re-measured in its new font; \
         {under_sans} would be the height it was measured at as sans-serif"
    );
}
