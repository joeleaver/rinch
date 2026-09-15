//! #763 — `animation-play-state: paused` stops the animation's clock, and it
//! must stop the app's too.
//!
//! A paused `@keyframes` animation keeps its `ActiveAnimation` entry, and has
//! to: its frozen sample is written into `computed_style` on every cascade, and
//! that is what makes it *look* paused rather than un-animated
//! (`display_none_animation_tests::a_paused_animation_on_a_rendered_box_is_untouched`
//! pins that the entry survives a re-cascade). But `tick_animations` used to
//! count that entry as running — `is_complete` answers `false` for anything
//! paused — and re-apply its frozen sample every tick, marking the node dirty
//! as though it had moved. The desktop frame clock schedules another frame on
//! either, so a paused spinner rendered at full rate, forever, with nothing on
//! screen moving. The same symptom #747 fixed for a hidden element, by a
//! different route.
//!
//! This file pins the rinch-dom half: what `tick_animations` answers and what
//! it leaves dirty. `crates/rinch/src/app/paused_animation_frames_tests.rs`
//! pins the frame clock itself, both of its readers (the redraw request and
//! K23's "was there anything to tick").
//!
//! Every animation whose tick answer is asserted `false` is paused before it
//! could end, so every `false` is the pause, never a timeout.
//!
//! The pre-pass narrowing has a price this file also pins: the tick used to be
//! the only thing that re-measured a paused typography animation's text, by
//! re-measuring it every frame. The cascade that writes a paused sample now does
//! it once (`a_class_that_adds_a_paused_font_size_animation_measures_its_text`,
//! `pausing_after_a_tick_that_sampled_the_base_size_measures_the_paused_size`,
//! both found by the review of #779).
//! Every resolve runs at one viewport size: a change of more than half a pixel
//! re-cascades the whole document, which is a second route to every answer
//! here (see `display_none_animation_tests::VP`).
//!
//! # Mutants, and what kills each
//!
//! Measured: each mutant applied to the committed source, `cargo test -p
//! rinch-dom -p rinch --no-fail-fast` run against it (85 result lines, the
//! unmutated control green), the source restored from the commit. "shell" names
//! fixtures in `crates/rinch/src/app/paused_animation_frames_tests.rs`.
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: a paused entry counts as running | 9: `a_paused_animation_asks_for_no_frames`, `a_paused_font_size_animation_does_not_invalidate_layout`, `resuming_a_paused_animation_asks_for_frames_again`, `a_resumed_animation_continues_from_where_it_was_paused`, `a_finished_transition_does_not_displace_a_paused_sample`, and 4 in the shell |
//! | a paused entry is **dropped** by the tick instead of kept quiet | 4: `a_paused_animation_asks_for_no_frames`, `two_animations_on_one_node_one_paused_still_ask_for_frames`, `a_resumed_animation_continues_from_where_it_was_paused`, shell `a_paused_animation_lets_the_app_go_idle` |
//! | the tick still re-applies a paused sample **and marks the node dirty** | 4: `a_paused_animation_leaves_the_tree_clean_after_a_tick` and 3 in the shell |
//! | the text-measure pre-pass is not narrowed to running animations | 2: `a_paused_font_size_animation_does_not_invalidate_layout`, shell `a_paused_animation_owes_the_android_loop_no_frame` |
//! | the tick answers `false` whenever anything is paused | 2: `a_running_animation_beside_a_paused_one_still_asks_for_frames`, `two_animations_on_one_node_one_paused_still_ask_for_frames` |
//! | a node's entries are judged by its **first** animation only | 2: `two_animations_on_one_node_one_paused_still_ask_for_frames`, `a_finished_transition_does_not_displace_a_paused_sample` |
//! | the tick **skips** a paused animation instead of re-applying its sample | `a_finished_transition_does_not_displace_a_paused_sample`, **alone** |
//! | resuming does not move the start time (the clock jumps the pause) | `a_resumed_animation_continues_from_where_it_was_paused`, **alone** |
//! | resuming restarts the animation from t=0 | `a_resumed_animation_continues_from_where_it_was_paused`, **alone** |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `width` and `height` are the animated properties because they affect layout,
/// which is the dirtying a re-applied sample does that is easiest to see.
const CSS: &str = "
    @keyframes k763-grow { from { width: 40px; } to { width: 100px; } }
    @keyframes k763-tall { from { height: 10px; } to { height: 100px; } }
    @keyframes k763-type { from { font-size: 10px; } to { font-size: 30px; } }
    @keyframes k763-slide { from { width: 0px; } to { width: 10000px; } }

    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .spin { animation: k763-grow 1000s linear infinite; }
    .held { animation: k763-grow 1000s linear infinite paused; }
    .both { animation: k763-grow 1000s linear infinite, k763-tall 1000s linear infinite;
            animation-play-state: paused, running; }
    .type-held { animation: k763-type 1000s linear infinite paused; }
    .slide { animation: k763-slide 10000ms linear infinite; }
    .slide--held { animation-play-state: paused; }
    .eased { transition: width 60ms linear; }
    .narrow { width: 20px; }
";

const VP: (f32, f32) = (800.0, 600.0);

fn animations(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|v| v.len())
        .unwrap_or(0)
}

/// The node's computed `width` in px, or `None` when it is not a length.
fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// A document with one `div.box` per class list, laid out once with the clock
/// armed. Returns the boxes in the order given.
fn mount(classes: &[&str]) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let boxes = classes
        .iter()
        .map(|class| {
            let node = doc.create_element("div");
            doc.set_attribute(node, "class", class);
            let text = doc.create_text("text");
            doc.append_child(node, text);
            doc.append_child(body, node);
            node
        })
        .collect();
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    (doc, boxes)
}

/// The control every `false` and every "clean" below rests on: a running
/// animation asks for frames, and its tick dirties the node it moved. Kills
/// nothing on its own and passes on `main`.
#[test]
fn a_running_animation_asks_for_frames() {
    let (mut doc, boxes) = mount(&["box spin"]);
    assert_eq!(animations(&doc, boxes[0]), 1, "precondition: registered");
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.get_mut(boxes[0].0).unwrap().dirty = rinch_dom::DirtyFlags::empty();

    assert!(doc.tick_animations(), "a moving clock needs another frame");
    assert!(
        doc.tree.dirty_nodes.contains(&boxes[0].0) && !doc.tree.paint_dirty_nodes.is_empty(),
        "and a running `width` animation dirties its node on a tick — without \
         this, `a_paused_animation_leaves_the_tree_clean_after_a_tick` would \
         also pass against a tick that never dirties anything"
    );
    assert!(
        doc.tree
            .get(boxes[0].0)
            .unwrap()
            .dirty
            .contains(rinch_dom::DirtyFlags::LAYOUT),
        "and flags the node itself"
    );
}

/// The issue's probe, verbatim: `.held { animation: … infinite paused }` —
/// after resolve there is one entry, and the tick has nothing to advance.
///
/// The entry and its frozen sample must both survive the tick: the fix is to
/// stop *counting* a paused animation, not to stop *having* one.
#[test]
fn a_paused_animation_asks_for_no_frames() {
    let (mut doc, boxes) = mount(&["box held"]);
    let held = boxes[0];
    assert_eq!(animations(&doc, held), 1, "precondition: registered");
    assert_eq!(
        width_px(&doc, held),
        Some(40.0),
        "precondition: the cascade wrote the frozen sample (t=0 of 40px→100px), \
         which is off the box's own 10px"
    );

    for round in 0..3 {
        assert!(
            !doc.tick_animations(),
            "round {round}: a paused animation has nothing to advance, so it \
             must not ask the shell for another frame"
        );
    }

    assert_eq!(
        animations(&doc, held),
        1,
        "the entry stays — it is what keeps the box looking paused"
    );
    assert_eq!(
        doc.tree.active_animations[&held.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Paused,
        "and it is still the paused one"
    );
    assert_eq!(width_px(&doc, held), Some(40.0), "still showing its sample");
}

/// A paused sample is already in `computed_style` — the cascade put it there —
/// so a tick that re-applies it has changed nothing and must not say it did.
///
/// `width` affects layout, so a tick that marked the node dirty would put it in
/// `dirty_nodes`, which the desktop frame clock resolves and repaints: a
/// redraw per frame by the second route, even with the tick answering `false`.
#[test]
fn a_paused_animation_leaves_the_tree_clean_after_a_tick() {
    let (mut doc, boxes) = mount(&["box held"]);
    let held = boxes[0];
    // What the shell does after a resolve: it takes the mutated-node set, and a
    // paint drains the repaint queue.
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    assert!(
        !doc.tree.layout_dirty,
        "precondition: the resolve left nothing to do"
    );
    // A resolve does not clear a node's own flags; clear them here so that
    // anything the tick sets is visible.
    doc.tree.get_mut(held.0).unwrap().dirty = rinch_dom::DirtyFlags::empty();

    doc.tick_animations();

    assert!(
        doc.tree.dirty_nodes.is_empty(),
        "a paused `width` animation dirtied layout on a tick: {:?}",
        doc.tree.dirty_nodes
    );
    assert!(!doc.tree.layout_dirty, "nor may it set `layout_dirty`");
    assert!(
        doc.tree.paint_dirty_nodes.is_empty(),
        "nor queue a repaint of a box that did not change: {:?}",
        doc.tree.paint_dirty_nodes
    );
    assert_eq!(
        doc.tree.get(held.0).unwrap().dirty,
        rinch_dom::DirtyFlags::empty(),
        "nor flag the node itself"
    );
}

/// `font-size` is a text-measure property, and `RinchDocument::tick_animations`
/// invalidates the text measure of every node animating one *before* the tick
/// (#678) — setting `layout_dirty`, which the desktop wake and the Android loop
/// both read as "a frame is owed". A frozen font size re-wraps nothing.
#[test]
fn a_paused_font_size_animation_does_not_invalidate_layout() {
    let (mut doc, boxes) = mount(&["box type-held"]);
    assert_eq!(animations(&doc, boxes[0]), 1, "precondition: registered");
    assert!(!doc.tree.layout_dirty, "precondition: clean");

    assert!(!doc.tick_animations(), "nothing to advance");
    assert!(
        !doc.tree.layout_dirty,
        "a paused `font-size` animation re-measured text on a tick"
    );
}

/// Pausing one animation must not silence another.
#[test]
fn a_running_animation_beside_a_paused_one_still_asks_for_frames() {
    let (mut doc, boxes) = mount(&["box held", "box spin"]);
    assert_eq!(
        animations(&doc, boxes[0]),
        1,
        "precondition: held registered"
    );
    assert_eq!(
        animations(&doc, boxes[1]),
        1,
        "precondition: spin registered"
    );
    assert!(
        doc.tick_animations(),
        "one paused box does not make the whole document still"
    );
    assert!(doc.tick_animations(), "on every tick, not just the first");
}

/// Two animations on **one** node, the paused one listed first — so a tick that
/// judged a node by its first entry, or by "any paused", goes quiet here.
#[test]
fn two_animations_on_one_node_one_paused_still_ask_for_frames() {
    let (mut doc, boxes) = mount(&["box both"]);
    let both = boxes[0];
    assert_eq!(animations(&doc, both), 2, "precondition: both registered");
    assert_eq!(
        doc.tree.active_animations[&both.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Paused,
        "precondition: the paused one is first"
    );
    assert!(
        doc.tick_animations(),
        "the running `height` animation on the same node still needs frames"
    );
    assert_eq!(animations(&doc, both), 2, "and neither entry was dropped");
}

/// Resuming schedules frames again. The half of the fix that is easy to lose:
/// a tick that went quiet for a paused entry must come back once it runs.
#[test]
fn resuming_a_paused_animation_asks_for_frames_again() {
    let (mut doc, boxes) = mount(&["box held"]);
    let node = boxes[0];
    assert!(!doc.tick_animations(), "precondition: paused and quiet");

    doc.set_attribute(node, "class", "box spin");
    doc.resolve_layout(VP.0, VP.1);

    assert_eq!(animations(&doc, node), 1, "the same entry, resumed");
    assert_eq!(
        doc.tree.active_animations[&node.0][0].play_state,
        rinch_dom::animation::AnimationPlayState::Running,
        "and it is running now"
    );
    assert!(doc.tick_animations(), "so the frames come back");
}

/// A paused animation resumed later continues from its frozen time — it
/// neither jumps the pause nor starts over.
///
/// `k763-slide` is 0px→10000px over 10000ms, linear, so its `width` in px *is*
/// its elapsed time in ms. The animation runs for at least 80ms before it is
/// paused, which keeps the frozen sample off zero — at zero, "resume from where
/// it was" and "restart from t=0" are the same answer.
#[test]
fn a_resumed_animation_continues_from_where_it_was_paused() {
    let (mut doc, boxes) = mount(&["box slide"]);
    let node = boxes[0];
    std::thread::sleep(std::time::Duration::from_millis(80));

    doc.set_attribute(node, "class", "box slide slide--held");
    doc.resolve_layout(VP.0, VP.1);
    let frozen = width_px(&doc, node).expect("a length");
    assert!(
        frozen >= 80.0,
        "precondition: paused after at least 80ms of running, read {frozen}"
    );

    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(!doc.tick_animations(), "paused, so nothing to advance");
    assert_eq!(
        width_px(&doc, node),
        Some(frozen),
        "and 150ms later it has not moved"
    );

    doc.set_attribute(node, "class", "box slide");
    doc.resolve_layout(VP.0, VP.1);
    assert!(doc.tick_animations(), "running again");
    let resumed = width_px(&doc, node).expect("a length");

    assert!(
        resumed >= frozen,
        "resumed at {resumed}px, before the {frozen}px it was paused at — \
         that is a restart, not a resume"
    );
    assert!(
        resumed < frozen + 100.0,
        "resumed at {resumed}px against a pause at {frozen}px: the 150ms spent \
         paused was counted as running time"
    );
}

/// The tick still re-applies a paused sample — it only stops *counting* it and
/// *dirtying* for it.
///
/// Skipping a paused entry outright would look equivalent, since the cascade
/// already wrote the sample, and it is not: a transition on the same property
/// writes `computed_style` from its own tick, and the one that **finishes**
/// writes its end value there. The animation tick runs after it, and a paused
/// animation re-applying its sample is what puts the value back to what the
/// cascade says it is. Skipped, the box would sit at the transition's end value
/// until something unrelated re-cascaded it.
#[test]
fn a_finished_transition_does_not_displace_a_paused_sample() {
    let (mut doc, boxes) = mount(&["box eased held"]);
    let node = boxes[0];
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "precondition: frozen at 40px"
    );

    // The base width changes under the paused animation, so a `width`
    // transition starts; the cascade keeps the frozen sample on top.
    doc.set_attribute(node, "class", "box eased held narrow");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        doc.tree.active_transitions.get(&node.0).map(|t| t.len()),
        Some(1),
        "precondition: a `width` transition is running"
    );
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "precondition: and the cascade still shows the paused sample"
    );

    std::thread::sleep(std::time::Duration::from_millis(120));
    doc.tick_transitions();
    assert_eq!(
        width_px(&doc, node),
        Some(20.0),
        "precondition: the finishing transition tick wrote its end value — \
         so what follows is the animation tick's doing, not a fixed point"
    );

    assert!(!doc.tick_animations(), "paused, so no frame");
    assert_eq!(
        width_px(&doc, node),
        Some(40.0),
        "a paused animation's tick put its sample back over the transition's \
         end value, as the cascade has it"
    );
}

// ── A paused typography animation is measured in the font it shows ───────────

/// Eight words in a 120px box, so a font-size change re-wraps them into a very
/// different number of lines. `line-height` is a **number**, so the line box
/// scales with the font and a stale measure shows up as a stale height.
const TEXT_CSS: &str = "
    @keyframes k763-big { from { font-size: 32px; } to { font-size: 48px; } }
    @keyframes k763-hold { 0%, 30% { font-size: 16px; } 100% { font-size: 48px; } }
    body { margin: 0; }
    .tb { width: 120px; font-size: 16px; line-height: 1.25; }
    .bigheld { animation: k763-big 1000s linear infinite paused; }
    .hold { animation: k763-hold 1000ms linear 1 forwards; }
    .hold.p { animation-play-state: paused; }
";
const WORDS: &str = "aaaa bbbb cccc dddd eeee ffff gggg hhhh";

fn text_box(classes: &str, inline_font_size: Option<f32>) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(TEXT_CSS);
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", classes);
    if let Some(px) = inline_font_size {
        doc.set_attribute(node, "style", &format!("font-size: {px}px"));
    }
    let text = doc.create_text(WORDS);
    doc.append_child(node, text);
    doc.append_child(body, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    (doc, node)
}

fn font_size(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().computed_style.font_size
}

fn box_height(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

/// The height the same box has when its font size is simply *declared* at
/// `px` — built in this process, so the comparison pins no font set.
fn reference_height(px: f32) -> f32 {
    let (doc, node) = text_box("tb", Some(px));
    box_height(&doc, node)
}

/// One frame the way the shell runs it: tick, hand off the dirty set, resolve.
fn frame(doc: &mut RinchDocument) -> bool {
    let answer = doc.tick_animations();
    let _ = doc.take_dirty_nodes();
    doc.resolve_layout(VP.0, VP.1);
    answer
}

/// A frame that must owe the shell nothing: the tick answers `false` **and**
/// leaves no layout pending. The second half is what a measure healed by
/// re-measuring on every tick fails — its tick answer is `false` too, but it
/// sets `layout_dirty` each time, which the desktop wake and the Android loop
/// both present for.
fn quiet_frame(doc: &mut RinchDocument, round: usize) {
    assert!(!doc.tick_animations(), "round {round}: no frame asked for");
    assert!(
        !doc.tree.layout_dirty,
        "round {round}: and no layout left owing — the tick re-measured a paused \
         sample"
    );
    let _ = doc.take_dirty_nodes();
    doc.resolve_layout(VP.0, VP.1);
}

/// A class adds a **paused** `font-size` animation to a box that is already laid
/// out, and the text has to be measured in the font the box now shows.
///
/// The cascade decides whether the text measure is stale by comparing the old
/// style with the style *before* the animation writes its sample, so an
/// animated typography value never takes part. Before #763 the tick's
/// text-measure pre-pass re-measured every `font-size` animation on every tick,
/// which hid that — by spinning. With the pre-pass narrowed to running
/// animations, nothing measured a paused one at all: 16px text laid out under a
/// 32px style, for good.
///
/// Both halves are asserted, because each has a mutant that passes the other:
/// healing the measure by re-measuring paused animations on every tick gives
/// the right height and asks for frames forever; not measuring gives an idle
/// clock and the wrong height.
#[test]
fn a_class_that_adds_a_paused_font_size_animation_measures_its_text() {
    let (mut doc, node) = text_box("tb", None);
    let base = box_height(&doc, node);
    let reference = reference_height(32.0);
    assert!(
        (base - reference).abs() > 20.0,
        "precondition: 32px text wraps to a different height than 16px \
         ({base} vs {reference}), or this fixture tells nothing apart"
    );

    doc.set_attribute(node, "class", "tb bigheld");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        font_size(&doc, node),
        32.0,
        "precondition: the sample is 32px"
    );
    assert_eq!(
        box_height(&doc, node),
        reference,
        "measured in the font it shows, on the very pass that paused it in"
    );

    for round in 0..3 {
        quiet_frame(&mut doc, round);
        assert_eq!(
            box_height(&doc, node),
            reference,
            "round {round}: and it stays"
        );
    }
}

/// The coincidence route: the last tick before the pause sampled exactly the
/// base font size, so the style the cascade compares against and the style it
/// resolves to agree — and the paused sample, taken later, does not.
///
/// `k763-hold` holds 16px (the base) for its first 300ms. The first tick lands
/// inside the hold, the pause lands at least 500ms in, so its sample is above
/// 16px whatever the scheduler does: past the end, `forwards` holds 48px.
#[test]
fn pausing_after_a_tick_that_sampled_the_base_size_measures_the_paused_size() {
    let (mut doc, node) = text_box("tb hold", None);
    frame(&mut doc);
    assert_eq!(
        font_size(&doc, node),
        16.0,
        "precondition: the last tick before the pause sampled the base size"
    );

    std::thread::sleep(std::time::Duration::from_millis(500));
    doc.set_attribute(node, "class", "tb hold p");
    doc.resolve_layout(VP.0, VP.1);
    let paused = font_size(&doc, node);
    assert!(
        paused > 20.0,
        "precondition: paused off the base, at {paused}px"
    );

    let reference = reference_height(paused);
    assert_eq!(box_height(&doc, node), reference, "measured at {paused}px");
    for round in 0..3 {
        quiet_frame(&mut doc, round);
        assert_eq!(
            box_height(&doc, node),
            reference,
            "round {round}: and it stays"
        );
    }
}

// ── Paused inside its delay ──────────────────────────────────────────────────

/// An animation paused **inside its delay** yields no values (there is no
/// backwards fill), and must still be kept.
///
/// A tick that kept a paused entry only while it produced values would drop
/// this one, and the resume would mint a new animation whose delay starts over
/// — measured by the review of #779: 10px against 85px, 330ms after a resume.
/// The entry's start time is the deterministic spelling of the same thing: a
/// resumed animation's start is the resume minus the time it had already
/// spent, so it lies well before the resume; a re-minted one starts at it.
#[test]
fn an_animation_paused_inside_its_delay_keeps_its_place() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "
        @keyframes k763-slide { from { width: 0px; } to { width: 10000px; } }
        .box { width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
        .dly { animation: k763-slide 10000ms linear 1 400ms; }
        .dly.p { animation-play-state: paused; }
    ",
    );
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", "box dly");
    doc.append_child(body, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);

    std::thread::sleep(std::time::Duration::from_millis(150));
    doc.set_attribute(node, "class", "box dly p");
    doc.resolve_layout(VP.0, VP.1);
    let spent = doc.tree.active_animations[&node.0][0]
        .paused_elapsed_ms
        .expect("precondition: paused");
    assert!(
        (140.0..400.0).contains(&spent),
        "precondition: paused inside the 400ms delay, {spent}ms in"
    );

    assert!(!doc.tick_animations(), "paused, so no frame");
    assert!(!doc.tick_animations(), "still none");
    assert_eq!(
        animations(&doc, node),
        1,
        "and the entry is kept although, inside its delay, it has no value"
    );

    doc.set_attribute(node, "class", "box dly");
    doc.resolve_layout(VP.0, VP.1);
    // Read *after* the resume, so it is later than the cascade's own clock: a
    // resumed start is `cascade - spent`, at most `after - spent`; a re-minted
    // one is `cascade`, above that bound whenever the resuming pass took less
    // than the 140ms+ already spent.
    let after = web_time::SystemTime::now()
        .duration_since(web_time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let start = doc.tree.active_animations[&node.0][0].start_time_ms;
    assert!(
        start <= after - spent + 1.0,
        "the resume kept the {spent}ms already spent in the delay: start \
         {start} should lie at least that far before {after}"
    );
}

/// The restart walk's half of the same rule. A panel un-hidden by an **inline**
/// `display` re-cascades the panel alone, so the paused animation inside it is
/// started by `restart_animations_in_subtree` rather than by its own cascade —
/// and that walk has to ask for the text to be measured in the sample it writes.
///
/// The text is laid out in its base 16px first, then the panel is hidden, the
/// animation class is added while it is hidden (the node's own cascade runs, but
/// a hidden node starts nothing), and the panel is shown by an inline write.
#[test]
fn a_paused_font_size_animation_restarted_by_showing_its_panel_measures_its_text() {
    let mut doc = RinchDocument::new();
    doc.load_css(TEXT_CSS);
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.append_child(body, panel);
    let node = doc.create_element("div");
    doc.set_attribute(node, "class", "tb");
    let text = doc.create_text(WORDS);
    doc.append_child(node, text);
    doc.append_child(panel, node);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(VP.0, VP.1);
    let reference = reference_height(32.0);
    assert_ne!(
        box_height(&doc, node),
        reference,
        "precondition: laid out at 16px"
    );

    doc.set_style(panel, "display", "none");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_attribute(node, "class", "tb bigheld");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, node),
        0,
        "precondition: hidden, so not started"
    );

    doc.set_style(panel, "display", "block");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        animations(&doc, node),
        1,
        "precondition: the walk restarted it"
    );
    assert_eq!(
        font_size(&doc, node),
        32.0,
        "precondition: at its 32px sample"
    );
    assert_eq!(
        box_height(&doc, node),
        reference,
        "and the text is measured in that sample on the pass that showed it"
    );
    for round in 0..2 {
        quiet_frame(&mut doc, round);
        assert_eq!(
            box_height(&doc, node),
            reference,
            "round {round}: and it stays"
        );
    }
}

/// The half of the cascade's rule a block of text does not need: a paused
/// `font-size` animation on a `<span>` inside an `inline-block`.
///
/// The `inline-block` is an atomic inline, detached from its parent's Taffy
/// child list and sized by its own measure pass (#661), so neither a
/// `layout_dirty` compute on its own nor a text-measure invalidation on its own
/// resizes it — measured, each half alone leaves it at its 16px `79x20` against
/// a `159x40` reference. The block-text fixtures above cannot tell those halves
/// apart; this one needs both.
#[test]
fn a_paused_font_size_animation_on_a_span_resizes_its_inline_block() {
    const CSS: &str = "
        @keyframes k763-big { from { font-size: 32px; } to { font-size: 48px; } }
        body { margin: 0; }
        .ib { display: inline-block; }
        .tb { font-size: 16px; line-height: 1.25; }
        .tb.bigheld { animation: k763-big 1000s linear infinite paused; }
    ";
    fn build(inline: Option<&str>) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let ib = doc.create_element("div");
        doc.set_attribute(ib, "class", "ib");
        doc.append_child(body, ib);
        let span = doc.create_element("span");
        doc.set_attribute(span, "class", "tb");
        if let Some(style) = inline {
            doc.set_attribute(span, "style", style);
        }
        let text = doc.create_text("aaaa bbbb");
        doc.append_child(span, text);
        doc.append_child(ib, span);
        doc.tree.transitions_enabled = true;
        doc.resolve_layout(VP.0, VP.1);
        (doc, ib, span)
    }
    fn size(doc: &RinchDocument, node: NodeId) -> (f32, f32) {
        let n = doc.tree.get(node.0).unwrap();
        (n.layout.width, n.layout.height)
    }

    let reference = {
        let (doc, ib, _) = build(Some("font-size: 32px"));
        size(&doc, ib)
    };
    let (mut doc, ib, span) = build(None);
    assert_ne!(
        size(&doc, ib),
        reference,
        "precondition: 16px is a smaller box"
    );

    doc.set_attribute(span, "class", "tb bigheld");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        font_size(&doc, span),
        32.0,
        "precondition: at its 32px sample"
    );
    assert_eq!(
        size(&doc, ib),
        reference,
        "the inline-block is re-measured around the paused sample"
    );
    for round in 0..2 {
        quiet_frame(&mut doc, round);
        assert_eq!(size(&doc, ib), reference, "round {round}: and stays");
    }
}
