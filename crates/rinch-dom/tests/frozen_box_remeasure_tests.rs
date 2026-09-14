//! A box re-measured when the text inside it is re-laid (issues #678, #661).
//!
//! Two faces of one gap. Layout has a cheap path — `resolve_layout` returns
//! early when `layout_dirty` is false, rebuilding dirty IFC text layouts and
//! never running Taffy — and an even cheaper one: `compute_inline_block_layouts`
//! runs only inside the `ifc_dirty` branch. Both exist for good reasons, and
//! both used to be taken on a change that moves a box:
//!
//! - **#678** — `font-family`, `font-weight`, `font-style`, `line-height` and
//!   friends are not Taffy properties, so a declaration change in any of them
//!   left `layout_dirty` false. The text re-wrapped in the new font and the
//!   block around it kept the height it was measured at in the old one:
//!   40 where it must be 80.
//! - **#661** — an atomic inline (`inline-block`, `inline-flex`, `inline-grid`)
//!   is detached from its parent's Taffy child list so the enclosing IFC can
//!   measure it as an `InlineBox`, which means the root compute never reaches
//!   it. A `font-size` change or a `set_text_content` sets neither flag that
//!   re-measures one, so the box stayed `225 x 20` while paint drew six lines.
//!
//! **Every assertion is against an oracle built in the same document**, never a
//! measured literal: a size measured from text pins the host's font set. Each
//! fixture also carries the counter-oracle — "the two states must differ at all"
//! — so a host where they happen to coincide fails loudly instead of passing
//! vacuously. `font-size` and `line-height` are declared everywhere so no line
//! box here is derived from a font metric.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

// ---------------------------------------------------------------- #678 -----

/// Two classes that differ **only** in typography Taffy cannot see.
/// 100px is chosen so `hello world` fits one line in sans-serif and not in
/// monospace, which is what makes the measured height differ by font with
/// `line-height` still declared.
const TYPOGRAPHY_CSS: &str = "
    .sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .mono { font-family: monospace;  font-size: 16px; line-height: 20px; }
    .red  { font-family: sans-serif; font-size: 16px; line-height: 20px; color: rgb(255,0,0); }
    .ifc  { width: 100px; }
";

fn typography_doc(class: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(TYPOGRAPHY_CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", class);
    doc.append_child(body, root);
    let ifc = doc.create_element("div");
    doc.set_attribute(ifc, "class", "ifc");
    doc.append_child(root, ifc);
    let text = doc.create_text("hello world hello world");
    doc.append_child(ifc, text);
    (doc, root, ifc)
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).expect("node is live").layout.height
}

/// #678: swapping an ancestor's class from `.sans` to `.mono` re-measures the
/// block the text wraps inside.
///
/// Kills the mutant that drops the `if measured_size_stale { layout_dirty }`
/// arm from `apply_stylo_styles_to_taffy` (measured: 40 against an 80 oracle).
/// The *other* direction — a gate widened to `text_layout_stale`, which is
/// correct and costs a compute per hover — is invisible here and to every other
/// fixture in this file; `a_colour_only_restyle_still_skips_taffy` below is the
/// only thing that catches it.
#[test]
fn a_typography_only_restyle_remeasures_its_box() {
    let (mut doc, root, ifc) = typography_doc("sans");
    doc.resolve_layout(800.0, 600.0);
    let under_sans = height(&doc, ifc);

    doc.set_attribute(root, "class", "mono");
    doc.resolve_layout(800.0, 600.0);

    let (mut oracle, _, oracle_ifc) = typography_doc("mono");
    oracle.resolve_layout(800.0, 600.0);
    let under_mono = height(&oracle, oracle_ifc);

    assert_ne!(
        under_sans, under_mono,
        "counter-oracle: the two families must wrap to different line counts"
    );
    assert_eq!(
        height(&doc, ifc),
        under_mono,
        "the restyled block must be re-measured in its new font; {under_sans} \
         is the height it was measured at as sans-serif"
    );
}

/// The cheap path survives: a restyle that changes only `color` re-shapes the
/// glyphs and runs **no** Taffy compute.
///
/// This is the pin on the narrowness of `same_measured_text_inputs`. Kills the
/// mutant that sets `layout_dirty` on the broader `text_layout_stale` — or on
/// every restyle — which passes every other fixture in this file and quietly
/// makes a `:hover { color }` cost a full relayout.
#[test]
fn a_colour_only_restyle_still_skips_taffy() {
    let (mut doc, root, _ifc) = typography_doc("sans");
    doc.resolve_layout(800.0, 600.0);
    let computes_after_first = doc.tree.taffy_computes;
    assert!(
        computes_after_first > 0,
        "positive control: the first layout must run at least one compute, or \
         this fixture cannot tell a skipped compute from a broken instrument"
    );

    doc.set_attribute(root, "class", "red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.taffy_computes, computes_after_first,
        "a colour-only restyle must take the early return, not a Taffy compute"
    );

    // Positive control on the other side: the same document, the same call,
    // a typography change — the compute must happen.
    doc.set_attribute(root, "class", "mono");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.taffy_computes > computes_after_first,
        "positive control: a typography change must run a compute, or the \
         assertion above passes because nothing ever computes"
    );
}

// ---------------------------------------------------------------- #661 -----

const ATOMIC_CSS: &str = "
    .small { font-size: 16px; line-height: 20px; font-family: sans-serif; }
    .large { font-size: 32px; line-height: 40px; font-family: sans-serif; }
    .inline-block { display: inline-block; }
    .inline-flex  { display: inline-flex; }
    .inline-grid  { display: inline-grid; }
    .block        { display: block; width: 200px; }
    .wrap { width: 900px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
";

const SHORT: &str = "hello world hello world";
const LONG: &str = "hello world hello world hello world hello world";

/// `div.wrap > span.<display>.<size> > text`. The `<span>` is the box under
/// test; `.wrap` is the IFC root that has to line-break around it.
fn atomic_doc(display: &str, size: &str, text: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(ATOMIC_CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let inner = doc.create_element("span");
    doc.set_attribute(inner, "class", &format!("{display} {size}"));
    doc.append_child(wrap, inner);
    let t = doc.create_text(text);
    doc.append_child(inner, t);
    (doc, inner, t)
}

fn size_of(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).expect("node is live");
    (n.layout.width, n.layout.height)
}

/// The `display` values this pair of fixtures runs over. `block` is the control
/// the issue reports as already tracking — it is in the table because a repair
/// that only moved the atomic-inline arms would leave it untested, and because
/// it is the arm that says the oracle is measuring anything at all: a mutant
/// that kills the atomic-inline re-measure leaves this one green, which is what
/// attributes the failure to the atomic-inline path rather than to #678's.
///
/// It is a **200px** block rather than the issue's `width: max-content`: that
/// keyword resolves to `auto` in this build, so the control filled its 900px
/// parent and neither change moved it — the counter-oracle caught it, which is
/// what a counter-oracle is for.
const DISPLAYS: [&str; 4] = ["inline-block", "inline-flex", "inline-grid", "block"];

/// #661 row 1: an atomic inline whose `font-size` changes is re-measured.
///
/// Kills the mutant that drops `remeasure_dirty_atomic_inlines` from
/// `resolve_layout`'s non-`ifc_dirty` branch (verified: every atomic arm comes
/// back `172 x 20` against a `344 x 40` oracle, while `block` stays green —
/// which is what says the mutant is attributable to the atomic-inline path and
/// not to #678's arm).
///
/// The `inline-flex` and `inline-grid` arms additionally kill the mutant that
/// drops the `dirty_text_contexts` marking: those two measure their interior
/// text through a cached `NodeContext::Text`, so a re-measure against a stale
/// context re-runs and re-answers `172 x 20`.
#[test]
fn an_atomic_inline_is_remeasured_when_its_font_size_changes() {
    for display in DISPLAYS {
        let (mut doc, inner, _t) = atomic_doc(display, "small", SHORT);
        doc.resolve_layout(900.0, 600.0);
        let before = size_of(&doc, inner);

        doc.set_attribute(inner, "class", &format!("{display} large"));
        doc.resolve_layout(900.0, 600.0);

        let (mut oracle, oracle_inner, _) = atomic_doc(display, "large", SHORT);
        oracle.resolve_layout(900.0, 600.0);
        let expected = size_of(&oracle, oracle_inner);

        assert_ne!(
            before, expected,
            "counter-oracle ({display}): doubling the font size must change the box"
        );
        assert_eq!(
            size_of(&doc, inner),
            expected,
            "{display}: the box must be re-measured at the new font size; \
             {before:?} is the box it was first measured at"
        );
    }
}

/// #661 row 2: an atomic inline whose text content changes is re-measured.
///
/// The text goes through `set_text_content`, which is the path the issue names:
/// it inserts into `dirty_ifc_text_roots`, so the text *is* re-laid — the box
/// that is supposed to contain it was not.
///
/// Kills the same mutant as the fixture above, and separately the mutant that
/// drops `mark_atomic_inline_dirty` from `set_text_content` (verified: the three
/// atomic arms freeze at `172 x 20`, `block` stays green).
#[test]
fn an_atomic_inline_is_remeasured_when_its_text_changes() {
    for display in DISPLAYS {
        let (mut doc, inner, t) = atomic_doc(display, "small", SHORT);
        doc.resolve_layout(900.0, 600.0);
        let before = size_of(&doc, inner);

        doc.set_text_content(t, LONG);
        doc.resolve_layout(900.0, 600.0);

        let (mut oracle, oracle_inner, _) = atomic_doc(display, "small", LONG);
        oracle.resolve_layout(900.0, 600.0);
        let expected = size_of(&oracle, oracle_inner);

        assert_ne!(
            before, expected,
            "counter-oracle ({display}): doubling the text must change the box"
        );
        assert_eq!(
            size_of(&doc, inner),
            expected,
            "{display}: the box must be re-measured around the new text; \
             {before:?} is the box it was first measured at"
        );
    }
}

/// Text that is a **flex item** — not inside an inline formatting context —
/// is measured through the cached `NodeContext::Text`, and a recascade is not a
/// DOM mutation, so nothing refreshed it.
///
/// This is the third face, and #678's repair is what exposed it: before that,
/// no compute ran at all on a typography-only restyle, so the stale context
/// never got the chance to answer. Kills the mutant that drops the
/// `dirty_text_contexts` insert, and separately the one that drops the
/// `taffy.mark_dirty` beside it (Taffy caches a leaf measure per available
/// space and serves the stale one back).
#[test]
fn text_that_is_a_flex_item_is_remeasured_in_its_new_font() {
    const CSS: &str = "
        .f  { display: flex; width: 800px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .fb { display: flex; width: 800px; font-size: 32px; line-height: 40px; font-family: sans-serif; }
    ";
    fn build(class: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let d = doc.create_element("div");
        doc.set_attribute(d, "class", class);
        doc.append_child(body, d);
        let t = doc.create_text("hello world");
        doc.append_child(d, t);
        (doc, d)
    }

    let (mut doc, d) = build("f");
    doc.resolve_layout(800.0, 600.0);
    let before = height(&doc, d);

    doc.set_attribute(d, "class", "fb");
    doc.resolve_layout(800.0, 600.0);

    let (mut oracle, od) = build("fb");
    oracle.resolve_layout(800.0, 600.0);
    let expected = height(&oracle, od);

    assert_ne!(
        before, expected,
        "counter-oracle: doubling the font size must change the row's height"
    );
    assert_eq!(
        height(&doc, d),
        expected,
        "the flex item's text must be re-measured at the new font size; \
         {before} is the height it was first measured at"
    );
}

/// A `transition: font-size` on a **flex row** re-measures the text inside it.
///
/// The witness for the `dirty_text_contexts` half of
/// `invalidate_text_measure_for_node`, and the one shape that needs it. A class
/// swap reaches a text node's measure context by a second route —
/// `invalidate_descendant_styles` puts every descendant into `dirty_nodes`,
/// which the incremental sync also reads — so the fixtures above survive
/// deleting the insert. A **transition** writes `computed_style` without going
/// through any of that: it marks only the transitioning element, and an element
/// is not a text node.
///
/// **`take_dirty_nodes` is called on purpose.** `RinchApp` drains that set every
/// frame, so a fixture that never drains it is measuring a state no running app
/// is ever in — the text node put there by the class swap would still be sitting
/// in it when the transition ticks, and would be refreshed for the wrong reason.
/// Without the drain this fixture passes against its own mutant.
#[test]
fn a_font_size_transition_on_a_flex_row_remeasures_its_text() {
    const CSS: &str = "
        .t { display: flex; width: 200px; font-size: 10px; line-height: 1.5;
             transition: font-size 150ms linear; }
        .t.big { font-size: 40px; }
    ";
    fn build(class: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let d = doc.create_element("div");
        doc.set_attribute(d, "class", class);
        doc.append_child(body, d);
        let t = doc.create_text("Hello world, wrap me please, several words here");
        doc.append_child(d, t);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(800.0, 600.0);
        (doc, d)
    }

    let (oracle, od) = build("t big");
    let expected = height(&oracle, od);

    let (mut doc, d) = build("t");
    let small = height(&doc, d);
    assert_ne!(
        small, expected,
        "counter-oracle: the 40px row must be taller than the 10px one"
    );

    doc.set_attribute(d, "class", "t big");
    doc.resolve_layout(800.0, 600.0);
    // The frame boundary a running app crosses here.
    let _ = doc.take_dirty_nodes();

    for t in doc
        .tree
        .active_transitions
        .get_mut(&d.0)
        .expect("the class change started a font-size transition")
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }
    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        height(&doc, d),
        expected,
        "the completed font-size transition must re-measure the flex row's text; \
         {small} is the height it was measured at as 10px"
    );
}

/// The animation twin of the fixture above: a `@keyframes` that animates
/// `font-size` on a flex row re-measures the text at each frame.
///
/// `tick_animations` writes `computed_style` the same way `tick_transitions`
/// does and reaches the cascade's invalidation exactly as little, so the repair
/// is duplicated there — and duplicated code with one fixture behind it is code
/// that rots. This is the second fixture.
#[test]
fn a_font_size_animation_on_a_flex_row_remeasures_its_text() {
    const CSS: &str = "
        @keyframes grow { from { font-size: 10px; } to { font-size: 40px; } }
        .t { display: flex; width: 200px; font-size: 10px; line-height: 1.5; }
        .anim { animation: grow 1000ms linear forwards; }
        .big { font-size: 40px; }
    ";
    fn build(extra: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let d = doc.create_element("div");
        doc.set_attribute(d, "class", &format!("t {extra}"));
        doc.append_child(body, d);
        let t = doc.create_text("Hello world, wrap me please, several words here");
        doc.append_child(d, t);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(800.0, 600.0);
        (doc, d)
    }

    let (oracle, od) = build("big");
    let expected = height(&oracle, od);

    let (mut doc, d) = build("anim");
    let small = height(&doc, d);
    assert_ne!(
        small, expected,
        "counter-oracle: the 40px row must be taller than the 10px one"
    );
    // The frame boundary a running app crosses here — see the transition twin.
    let _ = doc.take_dirty_nodes();

    for a in doc
        .tree
        .active_animations
        .get_mut(&d.0)
        .expect("the class should have started a font-size animation")
    {
        a.start_time_ms -= 10_000.0;
    }
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        height(&doc, d),
        expected,
        "the finished font-size animation must re-measure the flex row's text; \
         {small} is the height it was measured at as 10px"
    );
}
