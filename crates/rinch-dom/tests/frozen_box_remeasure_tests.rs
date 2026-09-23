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

// ------------------------------------------------- display: contents -----

/// `typography_doc` with the text wrapped in a `display: contents` `<span>` —
/// the shape `rsx!` emits for every `{|| …}` reactive text and every
/// if/for/match/component site. Returns the wrapper as well.
fn contents_doc(class: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(TYPOGRAPHY_CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", class);
    doc.append_child(body, root);
    let ifc = doc.create_element("div");
    doc.set_attribute(ifc, "class", "ifc");
    doc.append_child(root, ifc);
    let wrapper = doc.create_element("span");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(ifc, wrapper);
    let text = doc.create_text("hello world hello world");
    doc.append_child(wrapper, text);
    (doc, root, ifc, wrapper)
}

/// The cheap path survives a `display: contents` wrapper in the restyled
/// subtree.
///
/// `sync_display_contents` stores a wrapper's Taffy style as `Display::None`,
/// and the cascade used to rebuild it from the computed values as
/// `Display::Flex` — so the comparison in `apply_stylo_styles_to_taffy` saw a
/// display change on **every** re-cascade of a wrapper, set `ifc_dirty` and
/// `layout_dirty`, and ran the whole-document structural pass plus a compute.
/// A class change re-cascades the whole subtree (as a `:hover` does), so this
/// colour-only change reaches the wrapper. At 2000 rows that made a hover cost
/// 82ms where the same hover without wrappers costs 0.12ms.
///
/// `a_colour_only_restyle_still_skips_taffy` above could not see it: its
/// fixture has no wrapper.
#[test]
fn a_colour_only_restyle_over_a_contents_wrapper_still_skips_taffy() {
    let (mut doc, root, _ifc, _wrapper) = contents_doc("sans");
    doc.resolve_layout(800.0, 600.0);
    let computes = doc.tree.taffy_computes;
    let passes = doc.tree.ifc_setup_passes;
    assert!(
        computes > 0 && passes > 0,
        "positive control: the first layout must run a compute and a structural \
         pass, or this fixture cannot tell a skipped one from a broken counter"
    );

    // Twice: the first restyle after mount and a steady-state one must both
    // take the cheap path.
    for class in ["red", "sans"] {
        doc.set_attribute(root, "class", class);
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(
            doc.tree.ifc_setup_passes, passes,
            "a colour-only restyle over a contents wrapper (to `.{class}`) must \
             not re-run the structural IFC pass"
        );
        assert_eq!(
            doc.tree.taffy_computes, computes,
            "a colour-only restyle over a contents wrapper (to `.{class}`) must \
             take the early return, not a Taffy compute"
        );
    }

    // Positive control: the same document, a typography change — the compute
    // must happen, so the assertions above are not passing on a dead counter.
    doc.set_attribute(root, "class", "mono");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.taffy_computes > computes,
        "positive control: a typography change must run a compute"
    );
}

/// The other side of the same comparison: a **real** crossing into or out of
/// `display: contents` still re-runs the structural pass and lays the wrapper
/// out as what it now is.
///
/// The fix makes the cascade rebuild a wrapper's Taffy style as the
/// `Display::None` style `sync_display_contents` writes, keyed on the
/// **computed** display. Keying it on `contents_spliced` instead looks
/// equivalent and is not: on the way *out* of `contents` that flag is still set
/// when the cascade runs (the sync pass clears it later), so the wrapper keeps
/// `Display::None` and never gets its box back — this fixture's `200.0` check
/// fails on that mutant (measured: `0.0`). Also covered: both directions,
/// twice, and `flex -> contents`, whose Taffy displays used to compare equal
/// (#520).
#[test]
fn a_real_display_change_into_or_out_of_contents_still_invalidates() {
    let (mut doc, _root, ifc, wrapper) = contents_doc("sans");
    doc.resolve_layout(800.0, 600.0);
    let contents_height = height(&doc, ifc);

    for round in 0..2 {
        let passes = doc.tree.ifc_setup_passes;
        doc.set_attribute(wrapper, "style", "display: block; height: 200px");
        doc.resolve_layout(800.0, 600.0);
        assert!(
            doc.tree.ifc_setup_passes > passes,
            "round {round}: contents -> block must re-run the structural pass"
        );
        assert_eq!(
            height(&doc, wrapper),
            200.0,
            "round {round}: the wrapper must get its own box back"
        );
        assert_ne!(
            height(&doc, ifc),
            contents_height,
            "round {round}: counter-oracle: the container must grow around the \
             wrapper's own box"
        );

        let passes = doc.tree.ifc_setup_passes;
        doc.set_attribute(wrapper, "style", "display: contents; height: 200px");
        doc.resolve_layout(800.0, 600.0);
        assert!(
            doc.tree.ifc_setup_passes > passes,
            "round {round}: block -> contents must re-run the structural pass"
        );
        assert_eq!(
            height(&doc, ifc),
            contents_height,
            "round {round}: the wrapper must be spliced again, its height ignored"
        );
    }

    doc.set_attribute(wrapper, "style", "display: flex; height: 200px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, wrapper), 200.0, "flex: the wrapper has a box");
    let passes = doc.tree.ifc_setup_passes;
    doc.set_attribute(wrapper, "style", "display: contents; height: 200px");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.ifc_setup_passes > passes,
        "flex -> contents must re-run the structural pass"
    );
    assert_eq!(
        height(&doc, ifc),
        contents_height,
        "flex -> contents splices"
    );
}

/// Stylesheet for the two tick fixtures below: a `display: contents` wrapper
/// that carries a `width` transition or a `width` animation. `width` is a
/// layout property, so a tick marks the wrapper `LAYOUT`-dirty and it reaches
/// the tick passes' Taffy re-sync — which is the code under test.
const CONTENTS_TICK_CSS: &str = "
    @keyframes pulse { from { width: 10px; } to { width: 100px; } }
    .sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .red  { font-family: sans-serif; font-size: 16px; line-height: 20px; color: rgb(255,0,0); }
    .ifc  { width: 100px; }
    .w    { display: contents; width: 10px; transition: width 1000ms linear; }
    .w.wide { width: 100px; }
    .w.anim { animation: pulse 1000ms linear infinite; }
";

/// `div.<root_class> > div.ifc > span.w.<wrapper_extra> > text`, laid out once
/// with transitions enabled.
fn contents_tick_doc(root_class: &str, wrapper_extra: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CONTENTS_TICK_CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", root_class);
    doc.append_child(body, root);
    let ifc = doc.create_element("div");
    doc.set_attribute(ifc, "class", "ifc");
    doc.append_child(root, ifc);
    let wrapper = doc.create_element("span");
    doc.set_attribute(wrapper, "class", &format!("w {wrapper_extra}"));
    doc.append_child(ifc, wrapper);
    let text = doc.create_text("hello world hello world");
    doc.append_child(wrapper, text);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, root, wrapper)
}

/// The counters a tick fixture holds flat, plus the positive control that the
/// first layout moved them at all.
fn structural_counters(doc: &RinchDocument) -> (u64, u64) {
    let c = (doc.tree.ifc_setup_passes, doc.tree.taffy_computes);
    assert!(
        c.0 > 0 && c.1 > 0,
        "positive control: the first layout must run a structural pass and a \
         compute, or a flat counter proves nothing"
    );
    c
}

/// A transition ticking on a `display: contents` wrapper leaves its Taffy style
/// `sync_display_contents`' `Display::None`, so neither the tick nor the next
/// colour-only restyle re-runs the structural pass.
///
/// `tick_transitions` rebuilds a `LAYOUT`-dirty node's Taffy style from its
/// computed values. Without the contents substitution there it writes a
/// `Display::Flex` style onto the wrapper — a box of its own, mid-splice — and
/// the next re-cascade compares that with the `Display::None` it rebuilds and
/// runs the whole-document pass. The cascade fixture above cannot see this: it
/// never ticks.
#[test]
fn a_transition_ticking_on_a_contents_wrapper_keeps_the_cheap_path() {
    let (mut doc, root, wrapper) = contents_tick_doc("sans", "");
    doc.set_attribute(wrapper, "class", "w wide");
    doc.resolve_layout(800.0, 600.0);
    let _ = doc.take_dirty_nodes();
    let before = structural_counters(&doc);

    for t in doc
        .tree
        .active_transitions
        .get_mut(&wrapper.0)
        .expect("positive control: the class change started a width transition on the wrapper")
        .values_mut()
    {
        // Mid-transition, so the tick interpolates rather than finishing.
        t.start_time_ms -= 500.0;
    }
    doc.tick_transitions();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        (doc.tree.ifc_setup_passes, doc.tree.taffy_computes),
        before,
        "a width transition ticking on a contents wrapper must not give it a \
         Taffy box, re-run the structural pass or compute"
    );

    doc.set_attribute(root, "class", "red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.ifc_setup_passes, before.0,
        "a colour-only restyle after the tick must not re-run the structural pass"
    );
    assert_eq!(
        doc.tree.taffy_computes, before.1,
        "a colour-only restyle after the tick must not compute"
    );
}

/// The animation twin: `tick_animations` carries its own copy of the re-sync,
/// and a copy with no fixture behind it is free to drift.
#[test]
fn an_animation_ticking_on_a_contents_wrapper_keeps_the_cheap_path() {
    let (mut doc, root, wrapper) = contents_tick_doc("sans", "anim");
    let _ = doc.take_dirty_nodes();
    let before = structural_counters(&doc);

    for a in doc
        .tree
        .active_animations
        .get_mut(&wrapper.0)
        .expect("positive control: the wrapper's class started a width animation")
    {
        a.start_time_ms -= 500.0;
    }
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        (doc.tree.ifc_setup_passes, doc.tree.taffy_computes),
        before,
        "a width animation ticking on a contents wrapper must not give it a \
         Taffy box, re-run the structural pass or compute"
    );

    doc.set_attribute(root, "class", "red");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.ifc_setup_passes, before.0,
        "a colour-only restyle after the tick must not re-run the structural pass"
    );
    assert_eq!(
        doc.tree.taffy_computes, before.1,
        "a colour-only restyle after the tick must not compute"
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
/// One of the two witnesses for the `dirty_text_contexts` half of
/// `invalidate_text_measure_for_node`, and a **tick** is the shape that needs
/// it. A class swap reaches a text node's measure context by a second route —
/// `invalidate_descendant_styles` puts every descendant into `dirty_nodes`,
/// which the incremental sync also reads — so the fixtures above survive
/// deleting the insert. A transition writes `computed_style` without going
/// through any of that: it marks only the transitioning element, and an element
/// is not a text node. The animation twin below kills the same mutant for the
/// same reason; those two are the pair, and nothing else in the crate is.
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

// -------------------------------------------- inside an atomic inline -----

const NESTED_CSS: &str = "
    .wrap  { width: 900px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
    .ib    { display: inline-block; }
    .box   { display: block; height: 10px; width: 50px; }
    .box.wide { width: 300px; }
    .anim  { transition: width 150ms linear; }
";

/// `div.wrap > span.ib > div.box{extra}` — an ordinary block inside an atomic
/// inline, sized in px so nothing here is measured from a font.
fn block_in_atomic_inline(extra: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(NESTED_CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let ib = doc.create_element("span");
    doc.set_attribute(ib, "class", "ib");
    doc.append_child(wrap, ib);
    let b = doc.create_element("div");
    doc.set_attribute(b, "class", &format!("box {extra}"));
    doc.append_child(ib, b);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(900.0, 600.0);
    (doc, ib, b)
}

/// A **Taffy** style change on a box inside an atomic inline grows the atomic
/// inline.
///
/// The `font-size` and text fixtures above go through
/// `invalidate_text_measure_for_node`; this one goes through the other call
/// site, the `mark_atomic_inline_dirty` beside `set_style` in
/// `apply_stylo_styles_to_taffy`. Nothing else in the crate covers it — the
/// review of #694 measured that line surviving all 66 binaries of
/// `-p rinch-dom -p rinch`.
///
/// Kills the mutant that drops that call (measured: `50x10` against a `300x10`
/// oracle).
#[test]
fn a_taffy_restyle_inside_an_atomic_inline_regrows_it() {
    let (oracle, oracle_ib, _) = block_in_atomic_inline("wide");
    let expected = size_of(&oracle, oracle_ib);

    let (mut doc, ib, b) = block_in_atomic_inline("");
    let before = size_of(&doc, ib);
    assert_ne!(
        before, expected,
        "counter-oracle: 50px and 300px children must give different boxes"
    );
    let _ = doc.take_dirty_nodes();

    doc.set_attribute(b, "class", "box wide");
    doc.resolve_layout(900.0, 600.0);

    assert_eq!(
        size_of(&doc, ib),
        expected,
        "the atomic inline must contain its restyled child; {before:?} is the \
         box it was first measured at"
    );
}

/// The same change arriving as a **transition frame** rather than a restyle.
///
/// The tick's own Taffy re-sync is a second call site for the same invalidation,
/// and the tick pre-passes do not cover it: they fire only for
/// `changes_text_measure()` properties, and `width` is not one. Found by the
/// review of #694 as a fourth face of the same gap, live on this PR's own path.
///
/// Kills the mutant that drops `mark_atomic_inline_dirty` from
/// `tick_transitions`' re-sync (measured: `50x10` against a `300x10` oracle).
#[test]
fn a_width_transition_inside_an_atomic_inline_regrows_it() {
    let (oracle, oracle_ib, _) = block_in_atomic_inline("wide");
    let expected = size_of(&oracle, oracle_ib);

    let (mut doc, ib, b) = block_in_atomic_inline("anim");
    let before = size_of(&doc, ib);
    assert_ne!(before, expected, "counter-oracle");
    let _ = doc.take_dirty_nodes();

    doc.set_attribute(b, "class", "box anim wide");
    doc.resolve_layout(900.0, 600.0);
    // The frame boundary a running app crosses here.
    let _ = doc.take_dirty_nodes();

    for t in doc
        .tree
        .active_transitions
        .get_mut(&b.0)
        .expect("the class change started a width transition")
        .values_mut()
    {
        t.start_time_ms -= 10_000.0;
    }
    doc.tick_transitions();
    doc.resolve_layout(900.0, 600.0);

    assert_eq!(
        size_of(&doc, ib),
        expected,
        "the atomic inline must contain its transitioned child; {before:?} is \
         the box it was first measured at"
    );
}

/// The **animation** twin of the fixture above.
///
/// The Taffy re-sync is duplicated verbatim in `tick_transitions` and
/// `tick_animations`, so a fixture for one of them is a fixture for neither
/// copy in particular: the review of #694 measured the animation copy's mark
/// being killed by **zero** fixtures while the transition copy's was covered.
/// Duplicated code needs duplicated witnesses.
///
/// Kills the mutant that drops `mark_atomic_inline_dirty` from
/// `tick_animations`' re-sync.
#[test]
fn a_width_animation_inside_an_atomic_inline_regrows_it() {
    const CSS: &str = "
        @keyframes grow { from { width: 50px; } to { width: 300px; } }
        .wrap { width: 900px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .ib   { display: inline-block; }
        .box  { display: block; height: 10px; width: 50px; }
        .anim { animation: grow 1000ms linear forwards; }
        .wide { width: 300px; }
    ";
    fn build(extra: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_attribute(wrap, "class", "wrap");
        doc.append_child(body, wrap);
        let ib = doc.create_element("span");
        doc.set_attribute(ib, "class", "ib");
        doc.append_child(wrap, ib);
        let b = doc.create_element("div");
        doc.set_attribute(b, "class", &format!("box {extra}"));
        doc.append_child(ib, b);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(900.0, 600.0);
        (doc, ib, b)
    }

    let (oracle, oracle_ib, _) = build("wide");
    let expected = size_of(&oracle, oracle_ib);

    let (mut doc, ib, b) = build("anim");
    let before = size_of(&doc, ib);
    assert_ne!(before, expected, "counter-oracle");
    // The frame boundary a running app crosses here.
    let _ = doc.take_dirty_nodes();

    for a in doc
        .tree
        .active_animations
        .get_mut(&b.0)
        .expect("the class started a width animation")
    {
        a.start_time_ms -= 10_000.0;
    }
    doc.tick_animations();
    doc.resolve_layout(900.0, 600.0);

    assert_eq!(
        size_of(&doc, ib),
        expected,
        "the atomic inline must contain its animated child; {before:?} is the \
         box it was first measured at"
    );
}

/// An IFC root that owns a **#466 measure leaf** reflows when an atomic inline
/// inside it grows, at an unchanged available width.
///
/// The witness for `mark_ifc_measure_dirty(root_id)` in
/// `remeasure_dirty_atomic_inlines`, which round 1 of the #694 review could not
/// build and round 2 could. The shape needs three things at once: a root with
/// an out-of-flow child, so `setup_inline_formatting_contexts` gives it a
/// separate Taffy measure leaf instead of measuring it directly; a width that
/// does not change, so nothing else invalidates the measure; and an atomic
/// inline in the root's own inline content whose growth changes the root's line
/// count. Dirty propagates up, not down, so a `mark_dirty` on the root never
/// reaches that leaf.
///
/// Kills the mutant that drops the `mark_ifc_measure_dirty` call: the root
/// stays one line tall around two lines of content.
#[test]
fn a_measure_leaf_root_reflows_when_an_atomic_inline_inside_it_grows() {
    const CSS: &str = "
        .wrap { width: 200px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .abs  { position: absolute; width: 10px; height: 10px; }
        .ib   { display: inline-block; }
    ";
    fn build(label: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_attribute(wrap, "class", "wrap");
        doc.append_child(body, wrap);
        // The out-of-flow child is what makes this root own a measure leaf.
        let a = doc.create_element("div");
        doc.set_attribute(a, "class", "abs");
        doc.append_child(wrap, a);
        let lead = doc.create_text("lead text ");
        doc.append_child(wrap, lead);
        let ib = doc.create_element("span");
        doc.set_attribute(ib, "class", "ib");
        doc.append_child(wrap, ib);
        let t = doc.create_text(label);
        doc.append_child(ib, t);
        doc.resolve_layout(800.0, 600.0);
        (doc, wrap, ib, t)
    }

    let (oracle, oracle_wrap, oracle_ib, _) = build("wide wide wide wide");
    let expected_wrap = size_of(&oracle, oracle_wrap);
    let expected_ib = size_of(&oracle, oracle_ib);

    let (mut doc, wrap, ib, t) = build("x");
    let before_wrap = size_of(&doc, wrap);
    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&wrap.0),
        "positive control: this shape must actually own a measure leaf, or the \
         fixture pins nothing about `mark_ifc_measure_dirty`"
    );
    assert_ne!(
        before_wrap, expected_wrap,
        "counter-oracle: the root's height must differ between the two labels"
    );
    let _ = doc.take_dirty_nodes();

    doc.set_text_content(t, "wide wide wide wide");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        size_of(&doc, ib),
        expected_ib,
        "the atomic inline's own box"
    );
    assert_eq!(
        size_of(&doc, wrap),
        expected_wrap,
        "the IFC root must reflow around the grown atomic inline; \
         {before_wrap:?} is the box it was first measured at"
    );
}

/// Atomic inlines **nest**, and the outer one is sized from the inner one's
/// `Node::layout`.
///
/// `mark_atomic_inline_dirty` says in bold that its walk does not stop at the
/// first atomic inline it finds, and `remeasure_dirty_atomic_inlines` sorts its
/// targets deepest-first from the same premise. Both were documented behaviour
/// with no fixture until the review of #694 measured that deleting the sort
/// leaves the whole suite green.
///
/// Kills that mutant: without the sort the outer box is measured from the
/// inner one's *stale* layout and comes back one pass behind.
#[test]
fn nested_atomic_inlines_both_regrow() {
    const CSS: &str = "
        .wrap  { width: 900px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .outer { display: inline-block; }
        .inner { display: inline-block; }
    ";
    fn build(text: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_attribute(wrap, "class", "wrap");
        doc.append_child(body, wrap);
        let outer = doc.create_element("span");
        doc.set_attribute(outer, "class", "outer");
        doc.append_child(wrap, outer);
        let inner = doc.create_element("span");
        doc.set_attribute(inner, "class", "inner");
        doc.append_child(outer, inner);
        let t = doc.create_text(text);
        doc.append_child(inner, t);
        doc.resolve_layout(900.0, 600.0);
        (doc, outer, t)
    }

    let (oracle, oracle_outer, _) = build(LONG);
    let expected = size_of(&oracle, oracle_outer);

    let (mut doc, outer, t) = build("hello");
    let before = size_of(&doc, outer);
    assert_ne!(
        before, expected,
        "counter-oracle: the two texts must give different boxes"
    );
    let _ = doc.take_dirty_nodes();

    doc.set_text_content(t, LONG);
    doc.resolve_layout(900.0, 600.0);

    assert_eq!(
        size_of(&doc, outer),
        expected,
        "the outer atomic inline must grow with the inner one; {before:?} is \
         the box it was first measured at"
    );
}

/// The **third** pass that sizes an atomic inline, and the witness for saying so.
///
/// `mark_atomic_inline_dirty`'s doc used to claim that
/// `compute_inline_block_layouts` and `remeasure_dirty_atomic_inlines` were the
/// only two. `resolve_percentage_inline_blocks` is a third — it filters on the
/// same `display_mode.is_atomic_inline()` predicate and calls
/// `measure_inline_blocks` too — and it is the one #661 itself proposed as the
/// hook. The review of #694 caught the claim; this is what keeps it caught.
///
/// A viewport resize is the event that isolates it: nothing about it dirties the
/// IFC structure, and the re-cascade it forces produces identical computed
/// styles, so nothing marks anything and `dirty_atomic_inlines` is empty going
/// into the pass that moves this box.
///
/// **The attribution rests on a mutant, not on that emptiness.** Making
/// `resolve_percentage_inline_blocks` return early kills this fixture and
/// nothing else in the file — that is what says the third pass did the work.
/// An earlier revision asserted `is_empty()` *after* the resolve as a "positive
/// control"; the review of #694 showed it could not fail, because the pass
/// drains the set — seeding it with this very node beforehand still left it
/// empty afterwards. The check below is the same question asked where the
/// answer is not predetermined: **before** the pass, with nothing cleared by
/// hand.
#[test]
fn a_percentage_atomic_inline_tracks_a_viewport_resize() {
    const CSS: &str = "
        .wrap { font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .ib   { display: inline-block; width: 50%; }
    ";
    fn build() -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_attribute(wrap, "class", "wrap");
        doc.append_child(body, wrap);
        let ib = doc.create_element("span");
        doc.set_attribute(ib, "class", "ib");
        doc.append_child(wrap, ib);
        let t = doc.create_text(SHORT);
        doc.append_child(ib, t);
        (doc, ib)
    }

    let (mut oracle, oracle_ib) = build();
    oracle.resolve_layout(400.0, 600.0);
    let expected = size_of(&oracle, oracle_ib);

    let (mut doc, ib) = build();
    doc.resolve_layout(900.0, 600.0);
    let wide = size_of(&doc, ib);
    assert_ne!(
        wide, expected,
        "counter-oracle: a 50% box must differ between a 900px and a 400px viewport"
    );
    let _ = doc.take_dirty_nodes();
    assert!(
        doc.tree.dirty_atomic_inlines.is_empty(),
        "positive control: nothing may be marked going INTO the resize, or this \
         fixture is pinning `remeasure_dirty_atomic_inlines` and not the third \
         pass it is about. Asked before rather than after, and with nothing \
         cleared by hand: the pass drains the set, so an empty set afterwards \
         is a fixed point that cannot fail"
    );

    doc.resolve_layout(400.0, 600.0);

    assert_eq!(
        size_of(&doc, ib),
        expected,
        "the percentage atomic inline must track the new viewport; {wide:?} is \
         the box it had at 900px"
    );
}

/// The IFC root around a re-measured atomic inline keeps its **shaped text**,
/// not just its height.
///
/// The witness for `dirty_ifc_text_roots.insert(root_id)` in
/// `remeasure_dirty_atomic_inlines`, which the review of #694 measured as
/// surviving every test and every probe and reported as reading redundant. It
/// is not: the same block sets `root.text_layout = None`, and
/// `build_ifc_layouts` rebuilds a root only when it is rebuilding **all** of
/// them — which it does exactly when `dirty_ifc_text_roots` is *empty*. So the
/// insert is unnecessary until some **other** root is in that set, and then
/// dropping it leaves this root's Parley layout at `None` for good: the box is
/// the right size and nothing is drawn in it.
///
/// Hence the two roots. The second exists only to make the set non-empty, and
/// the atomic inline grows through a **Taffy** style change, which is the route
/// that does not dirty its own enclosing root on the way past. Measured against
/// the mutant: height 30 either way, `text_layout` `Some` with the insert and
/// `None` without it.
///
/// **Since the keep-text-layouts change the insert is belt-and-braces**:
/// `build_ifc_layouts` now rebuilds any root whose `text_layout` is `None`,
/// dirty or not, so dropping the insert no longer strands this root and the
/// mutant is equivalent on its own. The fixture stays as the pin on the
/// *outcome*: with **both** rules gone the root is left with nothing to paint.
#[test]
fn a_remeasured_atomic_inlines_root_keeps_its_shaped_text() {
    const CSS: &str = "
        .wrap { width: 400px; font-size: 16px; line-height: 20px; font-family: sans-serif; }
        .ib   { display: inline-block; }
        .box  { display: block; height: 10px; width: 50px; }
        .box.wide { width: 350px; }
    ";
    // `first` holds text plus an atomic inline; `second` holds text only.
    fn build(box_class: &str, second_text: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();

        let first = doc.create_element("div");
        doc.set_attribute(first, "class", "wrap");
        doc.append_child(body, first);
        let lead = doc.create_text("alpha alpha ");
        doc.append_child(first, lead);
        let ib = doc.create_element("span");
        doc.set_attribute(ib, "class", "ib");
        doc.append_child(first, ib);
        let inner = doc.create_element("div");
        doc.set_attribute(inner, "class", &format!("box {box_class}"));
        doc.append_child(ib, inner);

        let second = doc.create_element("div");
        doc.set_attribute(second, "class", "wrap");
        doc.append_child(body, second);
        let t2 = doc.create_text(second_text);
        doc.append_child(second, t2);

        doc.resolve_layout(800.0, 600.0);
        (doc, first, inner, t2)
    }

    let (oracle, oracle_first, _, _) = build("wide", "beta beta beta");
    let expected = size_of(&oracle, oracle_first);

    let (mut doc, first, inner, second_text) = build("", "beta");
    let before = size_of(&doc, first);
    assert_ne!(
        before, expected,
        "counter-oracle: the wider inline-block must push the root to a second line"
    );
    let _ = doc.take_dirty_nodes();

    // The *other* root goes into `dirty_ifc_text_roots`, which is what stops
    // `build_ifc_layouts` from rebuilding everything.
    doc.set_text_content(second_text, "beta beta beta");
    // …and the atomic inline grows through a Taffy style, the route that does
    // not dirty its own enclosing root.
    doc.set_attribute(inner, "class", "box wide");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        size_of(&doc, first),
        expected,
        "the root must reflow around the grown atomic inline; {before:?} is the \
         box it was first measured at"
    );
    assert!(
        doc.tree
            .get(first.0)
            .expect("the root is live")
            .text_layout
            .is_some(),
        "the root must still hold a shaped inline layout — this is the half the \
         height assertion above cannot see, and the only thing that fails when \
         the `dirty_ifc_text_roots` insert is dropped"
    );
}
