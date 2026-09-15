//! `transitions_enabled` arms **transitions**, and nothing else (issue #762).
//!
//! The flag is set at the end of the first `resolve_layout` so that a freshly
//! mounted tree does not animate into existence. That is a rule about
//! transitions: css-transitions-1 §3 needs a *before-change style*, and the
//! first cascade of a node has none. A `@keyframes` animation has no such
//! premise — it does not interpolate from a previous style, it plays its own —
//! and a browser runs one on the very first frame the element exists. The two
//! questions were asked with one `if`, and two user-visible faults followed:
//!
//! 1. an animation present in the first frame never started, and
//! 2. `recompute_all_styles_full` — the theme-change path — cleared every
//!    running animation and, with the gate forced off for its re-cascade,
//!    re-registered none. Toggling dark mode killed every spinner in the app.
//!
//! **Measured in Chrome 150.0.7871.100** (a `<style>` element's `textContent`
//! replaced under a running `animation: … 10s linear infinite`, sampled through
//! `Element.getAnimations()`):
//!
//! | The swap | `getAnimations()` | `currentTime` | effect |
//! |---|---|---|---|
//! | identical rules, one unrelated declaration changed | 1 | **unchanged** (2983 → 2983) | unchanged |
//! | same `animation-name`, **changed** `@keyframes` body | 1 | **unchanged** (2983) | **new** keyframes, applied at once |
//! | `@keyframes` deleted, declaration kept | **0** | — | element back at its base style |
//! | `animation` declaration deleted | **0** | — | element back at its base style |
//!
//! So a stylesheet swap does **not** restart a running animation: it keeps its
//! clock as long as the declaration still names a live `@keyframes` rule, and
//! cancels it when it does not.
//!
//! rinch matches rows 1 and 4, which are the two this file pins. Rows 2 and 3
//! it matches **on this path only**, and they are pinned in
//! `full_restyle_animation_refresh_tests.rs`: during `recompute_all_styles_full`
//! a kept animation keeps its clock and takes its `@keyframes` rule, its stops
//! and its duration and delay afresh, and is dropped when the rule is gone.
//! Row 3 **is** reachable — an earlier revision of this doc said it was not, and
//! was wrong: the theme sheet is replaceable and can carry `@keyframes`
//! (`generate_theme_css_string` already puts component keyframes in it), so a
//! new theme can drop one. Keeping the entry whole left that animation running;
//! Chrome 153 cancels it. Outside the full restyle — a plain class change — an
//! edited `@keyframes` body still never reaches a running animation (#766).
//!
//! # Which fixture kills which mutant
//!
//! Every row was run, not reasoned about. `rinch` names the two fixtures in
//! `rinch/src/app/animation_theme_change_tests.rs`.
//!
//! | Mutant | Killed by |
//! |---|---|
//! | **M1** `if self.tree.transitions_enabled` back around the animation block | `an_animation_declared_before_the_first_layout_runs_in_the_first_frame`, `the_first_frames_computed_style_carries_the_animated_value`, both `rinch` fixtures, and — through a failed precondition rather than their subject — `a_theme_change_keeps_a_running_animation_and_its_clock`, `a_theme_change_that_removes_the_declaration_stops_the_animation` and `a_spinner_hidden_and_shown_before_the_first_layout_restarts_from_zero` (7; re-measured after the rebase over #764) |
//! | **M2** `active_animations.clear()` back in `recompute_all_styles_full` | `a_theme_change_keeps_a_running_animation_and_its_clock`, `a_theme_restyle_keeps_the_loader_spinning_without_restarting_it` |
//! | **M4** `self.tree.transitions_enabled &&` dropped from the *transition* gate | `a_stylesheet_appended_during_construction_does_not_transition`, `a_theme_change_starts_no_transitions` |
//! | **M5** the `transitions_enabled = false` bracket dropped from `recompute_all_styles_full` | `a_theme_change_starts_no_transitions` |
//! | **M7** `transitions_enabled` initialised `true` | `a_stylesheet_appended_during_construction_does_not_transition` |
//! | **M8** the first layout never arms the flag | `a_transition_declared_before_the_first_layout_does_not_run` |
//! | **M9** `self.tree.transitions_enabled &&` kept in front of #747's restart walk | `a_panel_shown_before_the_first_layout_starts_its_spinner`, `a_spinner_hidden_and_shown_before_the_first_layout_restarts_from_zero` |
//!
//! # One consequence, and it is pre-existing
//!
//! An animation's clock starts at the cascade that styles the node, which for a
//! freshly appended element is `append_child`, not the layout pass. So the
//! first frame does not show the animation at t = 0: a few milliseconds of DOM
//! construction have already elapsed. A browser defers the start to the first
//! frame the element is rendered in. This is how every *post*-mount insertion
//! has always behaved; #762 only makes it reach the first frame as well, and it
//! cost one fixture — `transition_tests::a_finished_width_animation_reaches_the_layout`
//! was asserting an exact `from` width that was true by accident. Issue #768.
//!
//! Two results worth keeping. **No animation fixture here moves under M4, M5,
//! M7 or M8** — the flag no longer reaches an animation by any route, which is
//! the whole point of the split. And **M6**, `node_has_been_styled` dropped
//! from the transition gate, is killed by nothing in this file: it is the
//! *other* first-cascade suppressor and belongs to `transition_tests` (1
//! failure) and `reinsertion_transition_tests` (7), where it is already
//! covered.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::animation::AnimationResult;
use rinch_dom::transition::AnimatableValue;
use rinch_dom::transition::TransitionProperty;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// The animation's base colour — what the element is without the animation.
const BASE: (u8, u8, u8) = (0, 0, 255);
/// The `from` stop. Deliberately **not** the base colour: an animation that is
/// registered but never applied to `computed_style`, and one that is applied at
/// `t = 0` from a `from` stop equal to the base, are indistinguishable. Sampling
/// off that fixed point is what makes "the animation ran" an observation.
const FROM: (u8, u8, u8) = (255, 0, 0);
/// The `to` stop.
const TO: (u8, u8, u8) = (0, 255, 0);

const DURATION_MS: f64 = 1000.0;

/// `body > div.spin`, where `.spin` runs a 1000ms linear `background-color`
/// animation over a base colour neither stop equals.
///
/// `width`/`height` are declared so nothing here is measured from text — no
/// font set is pinned.
fn spinner_document() -> (RinchDocument, NodeId) {
    spinner_document_ms(DURATION_MS)
}

/// [`spinner_document`] with an explicit duration.
///
/// A fixture that reads the animated value out of `computed_style` — rather
/// than out of `values_at(start + elapsed)`, which is clock-free — is sampling
/// at whatever wall-clock instant the cascade ran, and the animation's clock
/// started a few milliseconds earlier at `append_child` (issue #768). Over a
/// long enough duration that drift is negligible; over 1000ms it showed up in
/// CI as `(253, 2, 0)` where an idle machine gave `(255, 0, 0)`.
fn spinner_document_ms(duration_ms: f64) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "@keyframes spin {{ \
           from {{ background-color: rgb({}, {}, {}); }} \
           to {{ background-color: rgb({}, {}, {}); }} \
         }} \
         .spin {{ \
           background-color: rgb({}, {}, {}); \
           width: 10px; height: 10px; line-height: 10px; \
           animation: spin {duration_ms}ms linear infinite; \
         }}",
        FROM.0, FROM.1, FROM.2, TO.0, TO.1, TO.2, BASE.0, BASE.1, BASE.2,
    ));
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "spin");
    doc.append_child(body, div);
    (doc, div)
}

/// How many animations are running on `div`.
fn running(doc: &RinchDocument, div: NodeId) -> usize {
    doc.tree
        .active_animations
        .get(&div.0)
        .map_or(0, |anims| anims.len())
}

/// The one running animation's start timestamp.
fn start_time(doc: &RinchDocument, div: NodeId) -> f64 {
    doc.tree
        .active_animations
        .get(&div.0)
        .expect("an animation should be running")[0]
        .start_time_ms
}

/// Move the running animation's clock back by `ms`, and answer the timestamp it
/// now carries.
///
/// This is what keeps "did the theme change restart it?" off a fixed point.
/// `start_time_ms` is a wall clock, so a restart a few microseconds after the
/// original registration writes a *nearly* equal value; backdating makes the
/// two answers 400ms apart and the assertion decisive.
fn backdate(doc: &mut RinchDocument, div: NodeId, ms: f64) -> f64 {
    let anim = &mut doc
        .tree
        .active_animations
        .get_mut(&div.0)
        .expect("an animation should be running")[0];
    anim.start_time_ms -= ms;
    anim.start_time_ms
}

/// The animated `background-color` `elapsed_ms` into the running animation.
fn animated_colour(doc: &RinchDocument, div: NodeId, elapsed_ms: f64) -> peniko::Color {
    let anim = &doc
        .tree
        .active_animations
        .get(&div.0)
        .expect("an animation should be running")[0];
    let AnimationResult::Values(values) = anim.values_at(anim.start_time_ms + elapsed_ms) else {
        panic!("a running animation should yield values");
    };
    values
        .iter()
        .find_map(|(prop, value)| match value {
            AnimatableValue::Color(c) if *prop == TransitionProperty::BackgroundColor => Some(*c),
            _ => None,
        })
        .expect("the animation animates background-color")
}

fn rgb(c: peniko::Color) -> (u8, u8, u8) {
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    (r, g, b)
}

// ── (a) an animation in the first frame runs ──────────────────────────────

/// **The first fault.** `transitions_enabled` is false for the whole of the
/// first cascade, so the animation block never ran and no `ActiveAnimation` was
/// created. Nothing re-cascades the node afterwards, so the animation was
/// simply gone — a `Loader` that is the first thing on screen never spun.
///
/// Kills the mutant "put `if self.tree.transitions_enabled` back around the
/// animation block".
#[test]
fn an_animation_declared_before_the_first_layout_runs_in_the_first_frame() {
    let (mut doc, div) = spinner_document();

    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        doc.tree.get(div.0).unwrap().animation_specs.len(),
        1,
        "precondition: the cascade found the `animation` declaration — without \
         this, zero running animations would say nothing"
    );
    assert_eq!(
        running(&doc, div),
        1,
        "a @keyframes animation has no before-change style to be wrong about, \
         so it runs on the first frame like a browser's"
    );
    assert_eq!(
        rgb(animated_colour(&doc, div, 0.0)),
        FROM,
        "…and its value is applied, not merely registered"
    );
}

/// The same first frame, read where the user sees it: the div's
/// `computed_style` carries the animation's `from` stop rather than its own
/// declared `background-color`.
///
/// Separate from the fixture above because it is a different link in the chain
/// — registration versus application — and the mutant that drops only the
/// `apply_value_to_style` loop leaves the first one green.
#[test]
fn the_first_frames_computed_style_carries_the_animated_value() {
    // 100s, so the milliseconds of DOM construction that have already elapsed
    // when the first layout runs (#768) move the colour by less than a unit.
    let (mut doc, div) = spinner_document_ms(100_000.0);

    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    let (r, g, b) = rgb(doc
        .tree
        .get(div.0)
        .unwrap()
        .computed_style
        .background_color()
        .expect("the div declares a background-color"));
    // The base is pure blue and the animation runs red → green, so **any**
    // point on the animation has `b == 0` and the base has `b == 255`. That
    // half is exact and is the whole claim: the animated value reached
    // `computed_style`, so the element paints animated rather than at its own
    // declared colour.
    assert_eq!(
        b, 0,
        "the animation's value is applied over the cascaded one in the first \
         frame, got rgb({r}, {g}, {b}) — 255 blue means the base colour"
    );
    // …and it is near the animation's start rather than somewhere arbitrary.
    assert!(
        r > 240,
        "…at roughly t = 0 of a 100s animation, got rgb({r}, {g}, {b})"
    );
}

// ── (b) a theme change keeps the clock ────────────────────────────────────

/// **The second fault.** `recompute_all_styles_full` cleared
/// `active_animations` and forced `transitions_enabled` false for its
/// re-cascade, so nothing re-registered them: every `Loader`, `Skeleton` and
/// `Progress` stripe in the app stopped for good the first time the theme
/// changed.
///
/// Chrome keeps the clock across a stylesheet swap (see the module doc), so the
/// assertion is on the exact backdated timestamp, not merely on "something is
/// running".
///
/// Kills M2, "clear `active_animations` in `recompute_all_styles_full`". Note
/// what that mutant now *is*: with the gate split, the re-cascade re-registers
/// what was cleared, so clearing no longer stops the spinner — it restarts it,
/// with a clock 400ms younger. The stop-for-good symptom needed both halves of
/// #762 at once, which is why this fixture asserts the timestamp and not merely
/// that something is running.
#[test]
fn a_theme_change_keeps_a_running_animation_and_its_clock() {
    let (mut doc, div) = spinner_document();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        running(&doc, div),
        1,
        "precondition: the animation is running"
    );

    // 400ms into a 1000ms animation: 40% of the way from FROM to TO.
    let expected_start = backdate(&mut doc, div, 400.0);
    assert_eq!(
        rgb(animated_colour(&doc, div, 400.0)),
        (153, 102, 0),
        "precondition: 400ms in, the animation is 40% of the way along"
    );

    // The theme-change path: new variables, then a full re-cascade.
    doc.update_theme_variables(":root { --rinch-probe: 2; }");
    doc.recompute_all_styles_full();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        running(&doc, div),
        1,
        "a theme change must not stop an animation whose declaration still names \
         a live @keyframes rule"
    );
    assert_eq!(
        start_time(&doc, div),
        expected_start,
        "…and must not restart its clock: Chrome 150 leaves currentTime alone \
         across a stylesheet swap"
    );
    assert_eq!(
        rgb(animated_colour(&doc, div, 400.0)),
        (153, 102, 0),
        "…so the spinner is still 40% of the way along, not back at the start"
    );
}

// ── (c) positive control: a theme change that removes the animation ───────

/// The instrument the fixture above relies on can say "stopped" as well as
/// "running": a re-cascade whose new rules no longer declare `animation` drops
/// the `ActiveAnimation`, matching Chrome's fourth row.
///
/// Without this, `a_theme_change_keeps_a_running_animation_and_its_clock` would
/// pass against a fix that simply never removes anything. It stays green under
/// M2 — measured — so the two are not the same assertion twice.
#[test]
fn a_theme_change_that_removes_the_declaration_stops_the_animation() {
    let (mut doc, div) = spinner_document();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        running(&doc, div),
        1,
        "precondition: the animation is running"
    );

    // A theme sheet that wins on specificity and turns the animation off.
    doc.update_theme_variables("body .spin { animation: none; }");
    doc.recompute_all_styles_full();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        doc.tree.get(div.0).unwrap().animation_specs.len(),
        0,
        "precondition: the re-cascade really did drop the declaration"
    );
    assert_eq!(
        running(&doc, div),
        0,
        "an animation whose declaration is gone stops, as in Chrome"
    );
}

// ── (d) transitions are still suppressed on the first frame ──────────────

/// A node's **first** cascade has no before-change style, so nothing transitions
/// into existence. That half is `has_been_styled`, not the flag.
///
/// Kills M8, "the first layout never arms the flag" — the second half of this
/// fixture is what fails there. It does **not** kill M6, dropping
/// `node_has_been_styled`: on a first cascade the flag is off anyway, so the
/// two suppressors cannot be told apart here. M6 is covered by
/// `transition_tests` and `reinsertion_transition_tests`; measured.
#[test]
fn a_transition_declared_before_the_first_layout_does_not_run() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".box { width: 10px; height: 10px; line-height: 10px; \
                background-color: rgb(0, 0, 255); \
                transition: background-color 1000ms linear; } \
         .box.hot { background-color: rgb(255, 0, 0); }",
    );
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "box");
    doc.append_child(body, div);

    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert!(
        !doc.tree.get(div.0).unwrap().transition_specs.is_empty(),
        "precondition: the box declares a transition"
    );
    assert!(
        doc.tree.active_transitions.is_empty(),
        "nothing transitions on the first frame — a node being styled for the \
         first time has no before-change style"
    );

    // And the flag is armed by the end of that pass, so the *next* change does
    // transition. Without this half, the assertion above would also pass for a
    // build that never starts a transition at all.
    doc.set_attribute(div, "class", "box hot");
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        doc.tree
            .active_transitions
            .get(&div.0)
            .map_or(0, |m| m.len()),
        1,
        "a change after the first layout does transition"
    );
}

/// **What `transitions_enabled` itself buys, and the reason it cannot just be
/// deleted once animations stop reading it.**
///
/// `has_been_styled` only suppresses a node's *first* cascade. A tree is
/// cascaded more than once before its first layout: appending a `<style>`
/// element re-resolves the whole document there and then
/// (`maybe_load_style_css`), so a component that appends its own stylesheet
/// after building its markup leaves every node already styled, and the very
/// next rule it loads is a *change* on an already-styled node. Without the
/// flag that is a transition running on page load, which is the thing the flag
/// exists to prevent.
///
/// Kills M4, "drop `self.tree.transitions_enabled &&` from the transition
/// gate", and M7, "initialise the flag `true`". The fixture above kills
/// neither — measured — which is why this one exists.
#[test]
fn a_stylesheet_appended_during_construction_does_not_transition() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 10px; height: 10px; line-height: 10px; \
         background-color: rgb(0, 0, 255); \
         transition: background-color 1000ms linear;",
    );
    doc.append_child(body, div);

    // A first stylesheet: cascades the whole document, so the div is styled
    // blue and `has_been_styled` is set — all before any layout.
    let first = doc.create_element("style");
    let first_css = doc.create_text(".irrelevant { color: rgb(1, 2, 3); }");
    doc.append_child(first, first_css);
    doc.append_child(body, first);

    // A second one that retargets it. On an already-styled node this is exactly
    // the shape §3 starts a transition for.
    let second = doc.create_element("style");
    let second_css = doc.create_text("div { background-color: rgb(255, 0, 0) !important; }");
    doc.append_child(second, second_css);
    doc.append_child(body, second);

    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert!(
        doc.tree.get(div.0).unwrap().has_been_styled,
        "precondition: the div really was cascaded before the retargeting sheet \
         arrived — otherwise `has_been_styled` would be doing this test's work"
    );
    assert_eq!(
        rgb(doc
            .tree
            .get(div.0)
            .unwrap()
            .computed_style
            .background_color()
            .expect("the div has a background-color")),
        (255, 0, 0),
        "precondition: the second sheet really did retarget the colour"
    );
    assert!(
        doc.tree.active_transitions.is_empty(),
        "nothing transitions before the first layout has completed, however many \
         times the tree was cascaded on the way there"
    );
}

// ── (e) transitions stay suppressed during the theme re-cascade ──────────

/// A theme change applies instantly. `recompute_all_styles_full` forces
/// `transitions_enabled` off for its re-cascade so a `transition: color` does
/// not start animating from the old palette and bake stale colours into text
/// layouts — and splitting the animation half out must not cost that.
///
/// Kills M5, "drop the `transitions_enabled = false` bracket in
/// `recompute_all_styles_full` now that animations no longer need it", and M4.
#[test]
fn a_theme_change_starts_no_transitions() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".box { width: 10px; height: 10px; line-height: 10px; \
                background-color: var(--probe, rgb(0, 0, 255)); \
                transition: background-color 1000ms linear; }",
    );
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "box");
    doc.append_child(body, div);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    assert!(
        !doc.tree.get(div.0).unwrap().transition_specs.is_empty(),
        "precondition: the box declares a transition"
    );

    doc.update_theme_variables(":root { --probe: rgb(255, 0, 0); }");
    doc.recompute_all_styles_full();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        rgb(doc
            .tree
            .get(div.0)
            .unwrap()
            .computed_style
            .background_color()
            .expect("the div has a background-color")),
        (255, 0, 0),
        "precondition: the new theme variable really did reach the element"
    );
    assert!(
        doc.tree.active_transitions.is_empty(),
        "a theme change applies instantly — no element transitions from the old \
         palette to the new one"
    );
}

// ── (f) a subtree shown on a pass the flag is off for gets its animations back ──

/// Wall-clock milliseconds, on the same clock `ActiveAnimation::start_time_ms`
/// is written from.
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0
}

/// `body > div.panel > div > div.spin`, **not laid out**, with the panel's
/// `display` written inline — the route `restart_animations_in_subtree` exists
/// for, since an inline write re-cascades the node it was written to and
/// nothing under it.
///
/// The spinner is the panel's *grandchild*, so nothing here passes merely
/// because a walk reached the direct children.
///
/// **The viewport is set before anything is built, and that is load-bearing.**
/// It is what `RinchApp::mount_component` does (so `vh`/`vw` resolve during
/// DOM construction), and it means the first `resolve_layout` at that same size
/// is not a viewport *change*. A change of more than half a pixel drops every
/// cached style, re-cascades the whole document and restarts the spinner
/// through its own per-node cascade — which is exactly what hides the walk.
/// Measured: without this line both fixtures below pass with the guard kept.
///
/// Returns `(doc, panel, spinner)`.
fn panel_document(panel_display: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.set_viewport(VIEWPORT.0, VIEWPORT.1);
    doc.load_css(&format!(
        "@keyframes spin {{ \
           from {{ background-color: rgb({}, {}, {}); }} \
           to {{ background-color: rgb({}, {}, {}); }} \
         }} \
         .spin {{ \
           background-color: rgb({}, {}, {}); \
           width: 10px; height: 10px; line-height: 10px; \
           animation: spin {DURATION_MS}ms linear infinite; \
         }}",
        FROM.0, FROM.1, FROM.2, TO.0, TO.1, TO.2, BASE.0, BASE.1, BASE.2,
    ));
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_style(panel, "display", panel_display);
    doc.append_child(body, panel);
    let inner = doc.create_element("div");
    doc.append_child(panel, inner);
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(inner, spinner);
    (doc, panel, spinner)
}

/// **The restart walk is not gated on `transitions_enabled` either** — the
/// #747 half of #762.
///
/// The spinner was appended into a hidden panel, so its own cascade started
/// nothing. The inline `display` write re-cascades the panel alone, so the only
/// thing that can start the spinner on the first layout is
/// `restart_animations_in_subtree` — and that layout runs with the flag still
/// `false`, as does every cascade before it.
///
/// **What this does not claim.** The shape is a subtree that is connected,
/// cascaded hidden, and shown again, all before the first layout completes.
/// `RinchApp::mount_component` appends the component's root to `<body>` only
/// after the component has returned and lays out straight away, so an ordinary
/// component tree is not expected to reach it through the shell; a node attached
/// to the document *during* the render (a body portal) plausibly could. Neither
/// has been built as a shell fixture.
///
/// The other pass that runs with the flag off, `recompute_all_styles_full`,
/// **does** reach the walk once the guard is gone, and not harmlessly: the walk
/// runs on the shown panel's cascade, before the spinner's own, and mints the
/// spinner's entry from its pre-restyle `computed_style`. On that pass this
/// fixture's M9 changes nothing a count can see — every node is re-cascaded, so
/// with the guard kept the spinner's own cascade starts it anyway — which is why
/// round 2's probe, which checked only the count, read it as unreachable. What
/// differs is the stops: an `em`-sized spinner came out on the old font-size.
/// The refresh during the full restyle re-extracts them;
/// `full_restyle_animation_refresh_tests::a_panel_shown_by_the_theme_starts_its_spinner_on_the_new_em_basis`
/// is that pin.
///
/// Kills **M9**, "keep `self.tree.transitions_enabled &&` in front of the
/// restart walk" (the guard as #764 shipped it): the spinner stays still until
/// something unrelated re-cascades it, which at a fixed viewport is never.
#[test]
fn a_panel_shown_before_the_first_layout_starts_its_spinner() {
    let (mut doc, panel, spinner) = panel_document("none");
    assert_eq!(
        doc.tree.get(spinner.0).unwrap().animation_specs.len(),
        1,
        "precondition: the cascade found the spinner's `animation` declaration"
    );
    assert_eq!(
        running(&doc, spinner),
        0,
        "precondition: a spinner appended into a hidden panel starts nothing"
    );

    doc.set_style(panel, "display", "block");
    assert!(
        !doc.tree.transitions_enabled,
        "precondition: the pass below is the first layout, with the flag off"
    );
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert!(
        matches!(
            doc.tree.get(panel.0).unwrap().computed_style.display,
            rinch_dom::computed_style::DisplayValue::Block
        ),
        "precondition: the panel really was shown on that pass"
    );
    assert_eq!(
        running(&doc, spinner),
        1,
        "a subtree shown on the first layout gets its animations, like one shown \
         on any later pass"
    );
}

/// The same pass, reached from a spinner that **was** running: shown, hidden
/// and shown again, all before the first layout. What comes back is a **new**
/// animation from t=0 (css-animations-1 §3), not the old one resumed.
///
/// Off the fixed point twice over. The original clock is backdated 400ms, so a
/// resurrected entry and a restarted one carry timestamps 400ms apart rather
/// than a wall-clock tie; and the assertion is that the new clock started
/// **inside** the show pass, which a stale entry cannot satisfy.
///
/// Kills M9 as well — measured — so it is not the fixture above twice: that one
/// never had a clock to be wrong about.
#[test]
fn a_spinner_hidden_and_shown_before_the_first_layout_restarts_from_zero() {
    let (mut doc, panel, spinner) = panel_document("block");
    assert_eq!(
        running(&doc, spinner),
        1,
        "precondition: a spinner appended into a rendered panel runs at once (#762)"
    );
    let old_start = backdate(&mut doc, spinner, 400.0);

    // Hide it, and let the tree be cascaded before any layout: any append
    // re-resolves on the spot, which is how a mounting component reaches this.
    doc.set_style(panel, "display", "none");
    let body = doc.body();
    let unrelated = doc.create_element("div");
    doc.append_child(body, unrelated);
    assert_eq!(
        running(&doc, spinner),
        0,
        "precondition: the hide dropped the spinner's animation (#747)"
    );

    doc.set_style(panel, "display", "block");
    assert!(
        !doc.tree.transitions_enabled,
        "precondition: the pass below is the first layout, with the flag off"
    );
    let show_started = now_ms();
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);

    assert_eq!(
        running(&doc, spinner),
        1,
        "a spinner shown again on the first layout spins again"
    );
    let new_start = start_time(&doc, spinner);
    assert!(
        new_start >= show_started,
        "…from a clock started by the show ({new_start} < {show_started}), \
         not the one it had before the hide ({old_start})"
    );
}
