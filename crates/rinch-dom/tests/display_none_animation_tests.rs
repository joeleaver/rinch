//! #747 — a `@keyframes` animation does not run on an element that is not
//! being rendered.
//!
//! css-animations-1 §3 is stricter than the transition rule #703 implements
//! next door, and the difference is the whole design. A transition on an
//! element that is not being rendered merely fails to *start*, because §3 of
//! css-transitions-1 gives it no before-change style. An animation on one has
//! **no animation effect at all**: it is not paused and it is not hidden, it
//! does not exist, and showing the element again starts a *new* animation from
//! the beginning rather than resuming the old one.
//!
//! rinch ran them. A `display: none` element's `ActiveAnimation` entries stayed
//! alive across the hide, interpolating into a `computed_style` nobody paints —
//! and, worse, the desktop frame clock schedules another frame whenever
//! `tree.active_animations` is non-empty (`rinch/src/app/event_dispatch.rs`),
//! so a `Loader` in a closed panel or an inactive tab kept an app rendering at
//! full rate with nothing on screen moving. `a_spinner_in_a_hidden_branch_stops_
//! asking_for_frames` and `a_detached_animation_stops_asking_for_frames` are
//! #699's pins for the *removed* twin; this file is the hidden one, and
//! `crates/rinch/src/app/hidden_animation_frames_tests.rs` asserts the frame
//! clock itself, on a real `Loader` in a real `Drawer`.
//!
//! # Three sites, and why the third has no counterpart on the transition side
//!
//! - **Nothing starts** on a node that is not rendered (`animation_is_rendered`).
//! - **A subtree that stops being rendered loses its animations**
//!   (`cancel_animations_in_subtree`), beside #703's transition cancel.
//! - **A subtree that starts being rendered again gets them back, from t=0**
//!   (`restart_animations_in_subtree`). A cancelled transition is never
//!   restarted, so #703 needed no such walk; a cancelled animation must be.
//!
//! The third one cannot be left to the ordinary cascade, and the reason is a
//! rinch detail rather than a CSS one: **a node shown by an ancestor need not be
//! re-cascaded at all.** `set_style` goes through `invalidate_inline_style`,
//! which drops the cached Stylo data of the node written to and of nothing else,
//! so a panel un-hidden with `set_style("display", "block")` re-cascades the
//! panel alone. `set_attribute` goes through `invalidate_descendant_styles` and
//! does re-cascade the subtree. Both routes have a fixture here, and only the
//! first discriminates.
//!
//! **Every resolve in this file therefore runs at one viewport size** — see
//! [`VP`]. That is not tidiness: a viewport change of more than half a pixel
//! invalidates every cached style, which re-cascades the whole document and
//! restarts a shown subtree through the per-node path, hiding the asymmetry the
//! walk exists for. Measured: with the restart walk stubbed out, hide-then-show
//! at a constant viewport leaves the animation gone and the same sequence with
//! the width bumped by 1px leaves it running. `display_none_transition_tests`
//! bumps the width freely because a transition has no second route.
//!
//! # Mutants, and what kills each
//!
//! Every attribution is **measured**: each mutant applied to the committed
//! source, `cargo test -p rinch-dom -p rinch --no-fail-fast` run against it, the
//! source reverted from the commit. That scope is the right one — the source is
//! `rinch-dom`, the fixtures are in both crates — and the unmutated control over
//! it is green, which is what says a zero below would mean something.
//!
//! | mutant | killed by |
//! |---|---|
//! | the gate never refuses (i.e. `main`'s starting behaviour) | 7: five here, two in `hidden_animation_frames_tests` |
//! | the gate reads the node's **own** display, no ancestor walk | the same 7 — see the note below |
//! | the gate checks the **immediate parent** instead of walking | 3: `an_animation_declared_two_levels_under_a_hidden_ancestor_never_starts` and both `display: none` fixtures in `hidden_animation_frames_tests` |
//! | the gate refuses but does not **remove** what was already running | `moving_a_spinner_into_a_hidden_panel_stops_it`, **alone** |
//! | the drop walk is a no-op | 7 |
//! | the drop walk does not descend (the node only) | the same 7 |
//! | the restart walk is a no-op | 4 |
//! | the restart walk never descends past the **direct children** | `showing_a_panel_restarts_a_spinner_three_levels_down`, **alone** |
//! | the restart walk does not stop at a box hidden in its own right | `showing_a_panel_leaves_a_box_hidden_in_its_own_right_alone`, **alone** |
//! | the restart site reuses the transition gate's `was_hidden` list | `showing_two_nested_wrappers_in_one_pass_restarts_the_spinner`, **alone** |
//! | `visibility: hidden` folded into "not rendered" | 4: `visibility_hidden_is_rendered_and_keeps_animating`, `a_paused_animation_on_a_rendered_box_is_untouched`, and both `Drawer` fixtures in `hidden_animation_frames_tests` |
//!
//! Five rows want reading twice.
//!
//! **Rows 1 and 2 are indistinguishable here, and that is a fact about the
//! change rather than a gap.** When a node's own `display` goes to `none` its
//! own cascade runs by construction, and the drop walk takes its entry away
//! regardless — so "own display only" and "no gate at all" differ only on a node
//! hidden by an ancestor, which is what both tables' five fixtures already
//! cover. Row 3 is the one that says the gate's walk *walks*, and it needed a fixture
//! built for it: before
//! `an_animation_declared_two_levels_under_a_hidden_ancestor_never_starts`
//! existed, a parent-only gate survived everything in this file, because every
//! other fixture puts the hidden box at the direct parent or reaches the answer
//! through the drop walk instead. That is #703's own lesson, repeated.
//!
//! **Row 4 is why the gate removes rather than merely declining.** A node
//! *moved* into a hidden panel changes nobody's `display`, so neither subtree
//! walk fires; the only thing that runs is the moved node's own re-cascade.
//!
//! **Rows 5 and 6 are indistinguishable for the same reason as 1 and 2**: the
//! node whose `display` changed is the wrapper, which carries no animation of
//! its own, so removing only its entry is removing nothing.
//!
//! **Row 8 was found by review, not by construction, and it is the cautionary
//! one.** A restart walk that never descended past the *direct children* passed
//! all 83 binaries in the scope: every other fixture exercising the walk put the
//! animated box at the direct child of the node being shown. The walk exists for
//! the inline route and every real overlay is several levels deep — `Drawer` is
//! root > overlay/panel > body > `Loader` > oval — so an app un-hiding a panel
//! that way would never have got its spinner back, silently.
//!
//! **Row 10 is the mistake this file exists to catch.** Reusing #703's
//! `was_hidden` list at the restart site is the natural thing to write, and it
//! is wrong: that list answers "was this rendered *before* the change", which is
//! the transition question. An animation asks "is it rendered *now*", and at the
//! show site the ancestor being un-hidden is on the list — so an inner wrapper
//! shown beneath an outer one in the same pass refuses its own restart and its
//! spinner never comes back.
//!
//! Two fixtures here kill nothing and say so in their own docs:
//! `a_rendered_spinner_asks_for_frames` is the positive control every zero rests
//! on, and `hiding_the_spinner_stops_it_asking_for_frames` is double-covered (the
//! gate's removal and the drop walk each do it alone), as is
//! `showing_the_spinner_itself_restarts_it_from_zero`, which the untouched
//! per-node path handles.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// Every box is given an explicit `font-size` and `line-height` so that nothing
/// here is derived from a font metric, and `width` is the animated property so
/// the animation is one a reader can check by value as well as by count.
///
/// `k747-grow` runs for 1000s so nothing below can end by expiring — every zero
/// in this file is something taking the animation away, never a timeout.
const CSS: &str = "
    @keyframes k747-grow { from { width: 10px; } to { width: 100px; } }
    .spin   { animation: k747-grow 1000s linear infinite;
              width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .held   { animation: k747-grow 1000s linear infinite paused;
              width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .wrap   { width: 200px; height: 200px; font-size: 16px; line-height: 20px; }
    .wrap--gone { display: none; }
    .wrap--here { display: block; }
    .wrap--veiled { visibility: hidden; }
";

/// **Every resolve in this file runs at this one size, and that is
/// load-bearing.**
///
/// A viewport change of more than half a pixel invalidates *every* cached Stylo
/// style (`resolve_layout`), so a fixture that bumps the width between steps
/// re-cascades the whole document — which restarts a shown subtree's animations
/// through the ordinary per-node path and hides the very asymmetry the subtree
/// walk exists for. Measured: with the restart walk stubbed out, hide-then-show
/// at a *constant* viewport leaves the animation gone, and the same sequence
/// with the width bumped by 1px leaves it running. The neighbouring
/// `display_none_transition_tests` bumps the width freely because a transition
/// has no such second route.
const VP: (f32, f32) = (800.0, 600.0);

/// How many animations are running on a node.
fn animations(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|v| v.len())
        .unwrap_or(0)
}

/// The wall-clock ms the node's one animation was started at.
fn start_time(doc: &RinchDocument, node: NodeId) -> f64 {
    let running = doc
        .tree
        .active_animations
        .get(&node.0)
        .expect("no animation is running on this node");
    assert_eq!(running.len(), 1, "this helper assumes exactly one");
    running[0].start_time_ms
}

fn display_of(doc: &RinchDocument, node: NodeId) -> rinch_dom::computed_style::DisplayValue {
    doc.tree.get(node.0).unwrap().computed_style.display
}

/// `body > div.wrap > div.spin`, laid out once with the clock armed.
///
/// Returns `(doc, wrap, spinner)`.
fn mounted_spinner() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(wrap, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, spinner),
        1,
        "precondition: the spinner is running"
    );
    (doc, wrap, spinner)
}

/// The control the whole file rests on: a rendered spinner runs, and asks the
/// shell for another frame.
///
/// Without it every "0 animations" assertion below would also pass against a
/// build where animations never start at all.
#[test]
fn a_rendered_spinner_asks_for_frames() {
    let (mut doc, _wrap, spinner) = mounted_spinner();
    assert_eq!(animations(&doc, spinner), 1, "it is running");
    assert!(
        doc.tick_animations(),
        "and the tick has something to advance"
    );
}

/// The issue's first half, at the element itself.
#[test]
fn hiding_the_spinner_stops_it_asking_for_frames() {
    let (mut doc, _wrap, spinner) = mounted_spinner();

    doc.set_style(spinner, "display", "none");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        0,
        "a `display: none` element is not being rendered, so it has no \
         animation effect (css-animations-1 §3)"
    );
    assert!(
        !doc.tick_animations(),
        "and the shell is not asked for another frame — `active_animations` \
         being non-empty is the whole of that decision"
    );
}

/// The issue's second half, and the one that is not derivable from the node's
/// own style: `display` does not inherit, so the spinner under a hidden wrapper
/// still computes `display: block`.
#[test]
fn hiding_an_ancestor_stops_a_descendant_animation() {
    let (mut doc, wrap, spinner) = mounted_spinner();

    doc.set_style(wrap, "display", "none");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        display_of(&doc, spinner),
        rinch_dom::computed_style::DisplayValue::Block,
        "the measurement this fixture rests on: the spinner's own display is \
         still `block`, so its own style cannot tell you it is hidden"
    );
    assert_eq!(animations(&doc, spinner), 0, "hidden by its ancestor");
    assert!(!doc.tick_animations(), "and the frames stop");
}

/// Two levels, because one level is what a check of the immediate parent also
/// passes.
///
/// `display_none_transition_tests` learned this the hard way for #703: a mutant
/// that replaced the ancestor walk with a parent check survived every fixture
/// in that file until one put the hidden ancestor a level further up.
#[test]
fn hiding_a_grandparent_stops_a_descendant_animation() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "class", "wrap");
    doc.append_child(body, outer);
    let mid = doc.create_element("div");
    doc.set_attribute(mid, "class", "wrap");
    doc.append_child(outer, mid);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(mid, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 1, "precondition: running");

    doc.set_style(outer, "display", "none");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        0,
        "two levels up is still not being rendered"
    );
}

/// A spinner mounted *inside* a panel that is already hidden never starts at
/// all — the commonest shape of the bug, since a closed panel is usually built
/// closed rather than built open and then shut.
#[test]
fn an_animation_declared_under_a_hidden_ancestor_never_starts() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap wrap--gone");
    doc.append_child(body, wrap);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(wrap, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        0,
        "it was never rendered, so it never had an animation effect"
    );
    assert!(
        !doc.tick_animations(),
        "an app whose closed panel holds a `Loader` must be allowed to go idle"
    );
}

/// The same question asked of the **gate** rather than of the drop walk, two
/// levels up.
///
/// The distinction matters because the two halves fail differently. Once a
/// subtree has been hidden, the drop walk has already taken its animations away
/// and the gate is never consulted for a descendant that is not re-cascaded —
/// so a gate that walked only to the immediate parent would survive
/// `hiding_a_grandparent_stops_a_descendant_animation`. A spinner *mounted*
/// two levels under a hidden box is the shape where only the gate can answer,
/// and it is the one a closed panel built closed actually has.
#[test]
fn an_animation_declared_two_levels_under_a_hidden_ancestor_never_starts() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "class", "wrap wrap--gone");
    doc.append_child(body, outer);
    let mid = doc.create_element("div");
    doc.set_attribute(mid, "class", "wrap");
    doc.append_child(outer, mid);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(mid, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        0,
        "the hidden box is the grandparent, so a gate that stopped at the \
         immediate parent would have started this"
    );
    assert!(!doc.tick_animations(), "and the frames never start");
}

/// Shown again, the animation comes back — and from t=0, not from where it was
/// when it was hidden.
///
/// The un-hide is written as an **inline** `display`, which is the route that
/// needs the subtree walk: `invalidate_inline_style` drops the cached Stylo
/// data of the node written to and of nothing else, so the wrapper is
/// re-cascaded and the spinner under it is not. Measured — see the mutant table
/// above; with the restart walk stubbed out this is the fixture that fails and
/// `showing_an_ancestor_by_class_restarts_a_descendant_animation` is not.
#[test]
fn showing_an_ancestor_restarts_a_descendant_animation_from_zero() {
    let (mut doc, wrap, spinner) = mounted_spinner();
    let first = start_time(&doc, spinner);

    doc.set_style(wrap, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 0, "precondition: dropped");

    // Long enough that a resumed animation and a restarted one cannot be
    // confused for one another by clock granularity.
    std::thread::sleep(std::time::Duration::from_millis(50));

    doc.set_style(wrap, "display", "block");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        1,
        "the spinner spins again once the panel is open"
    );
    let second = start_time(&doc, spinner);
    assert!(
        second >= first + 40.0,
        "a browser starts a *new* animation when the element is rendered again, \
         so its start time is now and its elapsed time is ~0; resumed it would \
         still read {first}, and this reads {second}"
    );
}

/// The spinner is a **great-grandchild** of the box whose `display` is toggled,
/// which is the shape every real component has.
///
/// The walk has to descend, and nothing else here says so: every other fixture
/// that exercises it puts the animated box at the *direct child* of the node
/// being shown, so a walk that restarted only direct children passed the entire
/// 83-binary suite. Found by review, not by construction.
///
/// It is not academic. The walk exists for the **inline** `set_style` route, and
/// a real overlay is several levels deep — `Drawer` is
/// root > overlay/panel > body > `Loader` > oval. An app un-hiding a panel that
/// way would never get its spinner back, and the suite would have said nothing.
#[test]
fn showing_a_panel_restarts_a_spinner_three_levels_down() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "wrap");
    doc.append_child(body, panel);

    // panel > a > b > spinner
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "wrap");
    doc.append_child(panel, a);
    let b = doc.create_element("div");
    doc.set_attribute(b, "class", "wrap");
    doc.append_child(a, b);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(b, spinner);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 1, "precondition: running");

    // The inline route, so only `panel` is re-cascaded — `a`, `b` and the
    // spinner are not, and the walk is the only thing that can reach them.
    doc.set_style(panel, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 0, "precondition: dropped");

    doc.set_style(panel, "display", "block");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, spinner),
        1,
        "the walk must DESCEND: a spinner three levels down comes back too"
    );
}

/// The other route to the same place, and it is **not** a discriminator: a
/// `class` write invalidates the subtree's cached styles
/// (`invalidate_descendant_styles`), so the spinner is re-cascaded on its own
/// and the ordinary per-node path restarts it.
///
/// It is here because the asymmetry between the two routes is the whole reason
/// the restart walk exists, and a reader who only saw the inline fixture could
/// reasonably conclude the walk is the only thing that ever restarts anything.
#[test]
fn showing_an_ancestor_by_class_restarts_a_descendant_animation() {
    let (mut doc, wrap, spinner) = mounted_spinner();
    let first = start_time(&doc, spinner);

    doc.set_attribute(wrap, "class", "wrap wrap--gone");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 0, "precondition: dropped");

    std::thread::sleep(std::time::Duration::from_millis(50));

    doc.set_attribute(wrap, "class", "wrap wrap--here");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, spinner), 1, "running again");
    assert!(
        start_time(&doc, spinner) >= first + 40.0,
        "and from t=0, by whichever route"
    );
}

/// Hidden and shown by its **own** `display`, which the per-node path handles
/// without any walk at all.
#[test]
fn showing_the_spinner_itself_restarts_it_from_zero() {
    let (mut doc, _wrap, spinner) = mounted_spinner();
    let first = start_time(&doc, spinner);

    doc.set_style(spinner, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 0, "precondition: dropped");

    std::thread::sleep(std::time::Duration::from_millis(50));

    doc.set_style(spinner, "display", "block");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, spinner), 1, "running again");
    assert!(
        start_time(&doc, spinner) >= first + 40.0,
        "restarted, not resumed"
    );
}

/// Showing a panel does not start an animation on a box inside it that is
/// hidden in its own right.
///
/// The restart walk descends, so it has to stop somewhere, and "somewhere" is
/// every box whose own `display` is `none` — that box is still not being
/// rendered and neither is anything under it. The sibling in the same panel is
/// the positive control that says the walk ran at all.
#[test]
fn showing_a_panel_leaves_a_box_hidden_in_its_own_right_alone() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);

    // A nested box hidden in its own right, with a spinner under it…
    let nested = doc.create_element("div");
    doc.set_attribute(nested, "class", "wrap wrap--gone");
    doc.append_child(wrap, nested);
    let buried = doc.create_element("div");
    doc.set_attribute(buried, "class", "spin");
    doc.append_child(nested, buried);

    // …and a sibling spinner that is only hidden by the panel.
    let visible = doc.create_element("div");
    doc.set_attribute(visible, "class", "spin");
    doc.append_child(wrap, visible);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, buried), 0, "precondition: never started");
    assert_eq!(animations(&doc, visible), 1, "precondition: running");

    doc.set_style(wrap, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, visible), 0, "precondition: panel closed");

    doc.set_style(wrap, "display", "block");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(
        animations(&doc, visible),
        1,
        "positive control: opening the panel did restart its spinner, so the \
         walk ran"
    );
    assert_eq!(
        animations(&doc, buried),
        0,
        "but the box under the nested `display: none` is still not rendered"
    );
}

/// `visibility: hidden` is the near neighbour that looks like the same thing
/// and is not.
///
/// Such a box is generated, laid out and **rendered** — merely invisible — so
/// its animations run, and a browser goes on painting them. It is the one shape
/// a component reaches for when it wants a hidden element that still animates.
///
/// Both halves of the question are asked, and the first one is the one that
/// discriminates. `visibility` is set on the **spinner itself**, so the
/// spinner's own cascade runs and the gate is actually consulted — a version
/// that veiled only the wrapper with an inline style would never re-cascade the
/// spinner at all (`invalidate_inline_style` invalidates one node), so the
/// animation would survive a gate that folds `visibility` in and the fixture
/// would discriminate nothing. Measured: that is exactly what the first draft
/// of this fixture did, and the `visibility`-folded mutant survived it. The
/// second half then covers the inherited case through a `class` write, which
/// does re-cascade the subtree.
#[test]
fn visibility_hidden_is_rendered_and_keeps_animating() {
    let (mut doc, wrap, spinner) = mounted_spinner();

    doc.set_style(spinner, "visibility", "hidden");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, spinner),
        1,
        "the spinner's own cascade ran and the gate let it through"
    );

    doc.set_attribute(wrap, "class", "wrap wrap--veiled");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, spinner),
        1,
        "and an inherited `visibility: hidden` is no different"
    );
    assert!(doc.tick_animations(), "still asking for frames");
}

/// `animation-play-state: paused` is not this change's business.
///
/// A paused animation on a rendered box keeps its entry across an unrelated
/// restyle — the gate asks about rendering, not about whether the clock is
/// moving. (It also keeps asking the shell for frames, which it arguably should
/// not; that is **#763**, not this.)
#[test]
fn a_paused_animation_on_a_rendered_box_is_untouched() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let held = doc.create_element("div");
    doc.set_attribute(held, "class", "held");
    doc.append_child(wrap, held);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, held), 1, "precondition: registered");

    // A `class` write, so the held node is re-cascaded and the gate is really
    // asked about it — an inline write to the wrapper would not reach it.
    doc.set_attribute(wrap, "class", "wrap wrap--veiled");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, held), 1, "a rendered box keeps it");
    assert_eq!(
        doc.tree.active_animations[&held.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Paused,
        "and it is still the paused one"
    );
}

/// …but a paused animation on a box that stops being rendered goes with the
/// rest.
///
/// "Paused" is a state of the animation, not a substitute for not having one:
/// a hidden element has no animation effect at all, and the shell's frame
/// decision cannot tell a paused entry from a running one.
#[test]
fn hiding_a_box_drops_its_paused_animation_too() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let held = doc.create_element("div");
    doc.set_attribute(held, "class", "held");
    doc.append_child(wrap, held);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, held), 1, "precondition: registered");

    doc.set_style(wrap, "display", "none");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, held), 0, "dropped with everything else");
    assert!(!doc.tick_animations(), "and the frames stop");
}

/// A spinner **moved** into a hidden panel stops, and the subtree walk is not
/// what stops it.
///
/// Nothing's `display` changed on this pass, so neither the drop walk nor the
/// restart walk fires at all. What the move does is re-cascade the moved node,
/// and the gate — asked about a node whose own `display` is `block` and whose
/// new ancestor's is `none` — is the only thing in the change that answers.
/// That is why the gate takes the entry away rather than merely declining to
/// start one.
///
/// It is not a detach, either: the node never leaves the document, so
/// `detach_subtree_styles` (#699) is not reached.
#[test]
fn moving_a_spinner_into_a_hidden_panel_stops_it() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let open = doc.create_element("div");
    doc.set_attribute(open, "class", "wrap");
    doc.append_child(body, open);
    let shut = doc.create_element("div");
    doc.set_attribute(shut, "class", "wrap wrap--gone");
    doc.append_child(body, shut);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(open, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 1, "precondition: running");

    doc.append_child(shut, spinner);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, spinner),
        0,
        "it is inside the closed panel now, so it is not being rendered"
    );
    assert!(!doc.tick_animations(), "and the frames stop");

    doc.append_child(open, spinner);
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, spinner),
        1,
        "and moving it back out starts it again"
    );
}

/// Two nested wrappers, both un-hidden in the **same** pass.
///
/// This is the fixture for a mistake that is very easy to make and that nothing
/// else here notices: reusing the transition gate's `was_hidden` list at the
/// restart site. That list names every node this cascade found hidden *before*
/// the change, which is the right question for a transition's before-change
/// style and the wrong one for an animation — an animation asks whether the box
/// is being rendered **now**. With the list passed in, the inner wrapper's own
/// restart is refused because its outer wrapper is on it, having been hidden
/// when the pass started, and the spinner two levels down never comes back.
///
/// The outer wrapper's own walk cannot cover for it: when that walk runs, the
/// inner wrapper has not been cascaded yet and is still carrying `display:
/// none`, so the walk correctly stops there.
#[test]
fn showing_two_nested_wrappers_in_one_pass_restarts_the_spinner() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "class", "wrap");
    doc.append_child(body, outer);
    let mid = doc.create_element("div");
    doc.set_attribute(mid, "class", "wrap");
    doc.append_child(outer, mid);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(mid, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    let first = start_time(&doc, spinner);

    doc.set_style(outer, "display", "none");
    doc.set_style(mid, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(animations(&doc, spinner), 0, "precondition: dropped");

    std::thread::sleep(std::time::Duration::from_millis(50));

    doc.set_style(outer, "display", "block");
    doc.set_style(mid, "display", "block");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, spinner), 1, "both open, so it spins");
    assert!(start_time(&doc, spinner) >= first + 40.0, "and from t=0");
}
