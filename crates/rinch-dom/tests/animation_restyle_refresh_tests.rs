//! A running `@keyframes` animation across **every** restyle, not only the theme
//! path (issues #766, #780, #781).
//!
//! `full_restyle_animation_refresh_tests.rs` pins what a running animation keeps
//! and takes afresh across `recompute_all_styles_full`. These fixtures pin the
//! same rule on the other routes a cascade reaches it by: a `<style>` appended
//! after the first layout, a class change, and a viewport change that flips which
//! `@keyframes` rule a `@media` block supplies. CSS Animations 1 §3: a change to
//! an `animation-*` longhand applies to the running animation, which keeps its
//! start time, and a changed `@keyframes` rule takes effect at once.
//!
//! # The oracle
//!
//! Chrome 156.0.8078.73, headless, `getAnimations()[i].currentTime` seeked and
//! `getComputedStyle` read after the change (the page is the PR's
//! `oracle-766.html`, quoted in its body). Every expected number is a row of it.
//!
//! | Row | The change | Chrome 156 |
//! |---|---|---|
//! | A | a `<style>` appended, redefining `@keyframes k` red → green as black → white, at 3000ms of 10s | same object, `currentTime` 3000, `rgb(77, 77, 77)` |
//! | C | a class sets `animation-duration: 20s` | same object, `currentTime` 3000, duration 20000, `rgb(217, 38, 0)` |
//! | D | a class sets `animation-delay: 1s` | delay 1000, `rgb(204, 51, 0)` |
//! | E | `k 10s, k 20s` seeked to 3000 / 5000, then an unrelated class | durations `10000, 20000`, `currentTime`s `3000, 5000` |
//! | F | `to { transform: rotate(360deg) }` under `color: rgb(10, 20, 30)`, a class sets `rgb(200, 210, 220)` | `rgb(200, 210, 220)` |
//! | G | `to { background-color: rgb(0, 255, 0) }` over a red base, a class makes the base blue, at 3000ms | `rgb(0, 77, 179)`: the implicit `from` is the **new** base |
//! | H | `from { width: 1em } to { width: 11em }`, `font-size` 10px → 40px, at 1000ms | `80px` |
//! | I | `from { opacity: 0.2 } to { opacity: 0.8; background-color: rgb(0, 255, 0) }` over a red base, at 3000ms | `rgb(179, 77, 0)`: the `from` keyframe has no `background-color`, so its value there is the base |
//! | K | an iframe 800 → 1000px wide, where `@media (min-width: 900px)` holds a later `@keyframes k` | same `currentTime` 3000, `rgb(77, 77, 77)` |
//! | L | the same resize, where `@media (max-width: 850px)` holds the only `@keyframes gone` | `getAnimations()` 0, the base `rgb(0, 0, 255)` |
//!
//! # Which fixture kills which mutant
//!
//! See the PR body for the run; each mutant is applied to the committed source.
//!
//! | Mutant | Killed by |
//! |---|---|
//! | **M1** a kept animation's stops are never re-extracted when the rule's blocks change | `a_style_appended_after_layout_redefines_a_running_keyframes_body`, `a_media_query_that_swaps_the_keyframes_rule_plays_the_new_body` |
//! | **M2** the font-size / colour inputs are left out of the re-extraction key | `an_em_stop_follows_a_font_size_class_change` |
//! | **M3** duration and delay are not copied from the spec | `a_class_change_re_times_a_running_animation_around_its_clock`, `a_class_change_that_adds_a_delay_applies_it_to_the_running_animation` |
//! | **M4** a repeated name matches the first entry (`find`) | `two_animations_of_one_name_keep_their_own_durations_and_clocks` |
//! | **M5** the implicit stop takes the fixed property list (no filter) | `a_colour_class_change_reaches_a_spinner_with_a_to_only_rule` |
//! | **M6** implicit values are not refreshed when the inputs key is unchanged | `an_implicit_from_stop_follows_the_base_value_it_stands_for` |
//! | **M7** a property missing from an explicit `from`/`to` is not filled from the base | `a_property_missing_from_an_explicit_from_starts_at_the_base_value` |
//! | **M8** a kept animation whose rule is gone is kept | `a_media_query_that_drops_the_keyframes_rule_cancels_the_animation` |

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::animation::AnimationResult;
use rinch_dom::transition::{AnimatableValue, TransitionProperty};

const VP: (f32, f32) = (800.0, 600.0);

/// Every animated box is 10x10 with a declared `line-height`, so nothing here is
/// measured from text.
const BOX: &str = "div.x { width: 10px; height: 10px; line-height: 10px; \
                   background-color: rgb(0, 0, 255); }";

const RED_GREEN: &str = "@keyframes k { from { background-color: rgb(255, 0, 0); } \
                                        to { background-color: rgb(0, 255, 0); } }";

fn document(author: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.set_viewport(VP.0, VP.1);
    doc.load_css(&format!("{BOX} {author}"));
    doc
}

/// One `x`-classed box carrying `class`, laid out, its animations backdated by
/// `backdate` (one per animation), as the oracle seeks `currentTime`.
fn running(doc: &mut RinchDocument, class: &str, backdate: &[f64]) -> NodeId {
    let body = doc.body();
    let n = doc.create_element("div");
    doc.set_attribute(n, "class", &format!("x {class}"));
    doc.append_child(body, n);
    doc.resolve_layout(VP.0, VP.1);
    let anims = doc
        .tree
        .active_animations
        .get_mut(&n.0)
        .expect("precondition: an animation is running");
    assert_eq!(anims.len(), backdate.len(), "one backdate per animation");
    for (a, by) in anims.iter_mut().zip(backdate) {
        a.start_time_ms -= by;
    }
    n
}

fn restyle_class(doc: &mut RinchDocument, n: NodeId, class: &str) {
    doc.set_attribute(n, "class", &format!("x {class}"));
    doc.resolve_layout(VP.0, VP.1);
}

/// A `<style>` element connected first and given its text second, so the sheet
/// is loaded by a connected element's text change whichever way the document
/// installs late sheets.
fn append_style(doc: &mut RinchDocument, css: &str) {
    let body = doc.body();
    let style = doc.create_element("style");
    doc.append_child(body, style);
    let text = doc.create_text(css);
    doc.append_child(style, text);
    doc.resolve_layout(VP.0, VP.1);
}

fn start_times(doc: &RinchDocument, n: NodeId) -> Vec<f64> {
    doc.tree
        .active_animations
        .get(&n.0)
        .map(|anims| anims.iter().map(|a| a.start_time_ms).collect())
        .unwrap_or_default()
}

/// The value of `property` on `n`'s `index`th animation, `elapsed_ms` into it,
/// sampled on the animation's own clock rather than at "now".
fn sample(
    doc: &RinchDocument,
    n: NodeId,
    index: usize,
    elapsed_ms: f64,
    property: TransitionProperty,
) -> AnimatableValue {
    let anim = &doc.tree.active_animations[&n.0][index];
    let AnimationResult::Values(values) = anim.values_at(anim.start_time_ms + elapsed_ms) else {
        panic!("{} yields no values {elapsed_ms}ms in", anim.name);
    };
    values
        .into_iter()
        .find_map(|(p, v)| (p == property).then_some(v))
        .unwrap_or_else(|| panic!("{} does not animate {property:?}", anim.name))
}

fn rgb(value: &AnimatableValue) -> (u8, u8, u8) {
    let AnimatableValue::Color(c) = value else {
        panic!("not a colour: {value:?}");
    };
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    (r, g, b)
}

fn bg_at(doc: &RinchDocument, n: NodeId, elapsed_ms: f64) -> (u8, u8, u8) {
    rgb(&sample(
        doc,
        n,
        0,
        elapsed_ms,
        TransitionProperty::BackgroundColor,
    ))
}

// ── #766: a changed `@keyframes` rule reaches the running animation ─────────

/// Row A, the issue's own example on the `<style>` route. Chrome keeps
/// `currentTime` and plays the new body: 30% along black → white.
#[test]
fn a_style_appended_after_layout_redefines_a_running_keyframes_body() {
    let mut doc = document(&format!(
        "{RED_GREEN} .t {{ animation: k 10s linear infinite; }} \
         .t.other {{ color: rgb(1, 2, 3); }}"
    ));
    let t = running(&mut doc, "t", &[3000.0]);
    let start = start_times(&doc, t);
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (179, 77, 0),
        "precondition: old body"
    );

    append_style(
        &mut doc,
        "@keyframes k { from { background-color: rgb(0, 0, 0); } \
                        to { background-color: rgb(255, 255, 255); } }",
    );

    assert_eq!(start_times(&doc, t), start, "the clock is kept");
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (77, 77, 77),
        "the redefined body plays on the kept clock, as in Chrome 156"
    );

    // And a later ordinary restyle keeps the new body (the issue's second
    // measurement).
    restyle_class(&mut doc, t, "t other");
    assert_eq!(bg_at(&doc, t, 3000.0), (77, 77, 77));
}

/// The rule an `animation-name` resolves to can change without any sheet being
/// added: a `@media` block that supplies a later `@keyframes k` starts to
/// apply when the viewport grows. Same object, same clock, new body.
#[test]
fn a_media_query_that_swaps_the_keyframes_rule_plays_the_new_body() {
    let mut doc = document(&format!(
        "{RED_GREEN} .t {{ animation: k 10s linear infinite; }} \
         @media (min-width: 900px) {{ \
           @keyframes k {{ from {{ background-color: rgb(0, 0, 0); }} \
                           to {{ background-color: rgb(255, 255, 255); }} }} }}"
    ));
    let t = running(&mut doc, "t", &[3000.0]);
    let start = start_times(&doc, t);
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (179, 77, 0),
        "precondition: old body"
    );

    doc.resolve_layout(1000.0, VP.1);

    assert_eq!(start_times(&doc, t), start, "the clock is kept");
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (77, 77, 77),
        "the @media body plays"
    );
}

/// Chrome cancels an animation whose `@keyframes` rule is gone while its
/// `animation-name` stays (#766's last paragraph). A `@media` block that stops
/// applying is a route to that state with no sheet removal.
#[test]
fn a_media_query_that_drops_the_keyframes_rule_cancels_the_animation() {
    let mut doc = document(
        ".t { animation: gone 10s linear infinite; } \
         @media (max-width: 850px) { \
           @keyframes gone { from { background-color: rgb(255, 0, 0); } \
                             to { background-color: rgb(0, 255, 0); } } }",
    );
    let t = running(&mut doc, "t", &[3000.0]);

    doc.resolve_layout(1000.0, VP.1);

    assert_eq!(
        doc.tree.get(t.0).unwrap().animation_specs.len(),
        1,
        "precondition: the declaration still names `gone`"
    );
    assert!(
        !doc.tree.active_animations.contains_key(&t.0),
        "an animation whose @keyframes rule no longer exists is cancelled"
    );
}

// ── #780: a changed `animation-*` longhand applies, keeping the clock ───────

/// Row C: 10s → 20s by a class. Chrome keeps `currentTime` 3000; progress 0.15.
#[test]
fn a_class_change_re_times_a_running_animation_around_its_clock() {
    let mut doc = document(&format!(
        "{RED_GREEN} .t {{ animation: k 10s linear infinite; }} \
         .t.slow {{ animation-duration: 20s; }}"
    ));
    let t = running(&mut doc, "t", &[3000.0]);
    let start = start_times(&doc, t);

    restyle_class(&mut doc, t, "t slow");

    assert_eq!(start_times(&doc, t), start, "the clock is kept");
    assert_eq!(doc.tree.active_animations[&t.0][0].duration_ms, 20_000.0);
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (217, 38, 0),
        "15% along red → green"
    );
}

/// Row D: a class adds `animation-delay: 1s`. Chrome: progress 0.2 at 3000ms.
#[test]
fn a_class_change_that_adds_a_delay_applies_it_to_the_running_animation() {
    let mut doc = document(&format!(
        "{RED_GREEN} .t {{ animation: k 10s linear infinite; }} \
         .t.late {{ animation-delay: 1s; }}"
    ));
    let t = running(&mut doc, "t", &[3000.0]);
    let start = start_times(&doc, t);

    restyle_class(&mut doc, t, "t late");

    assert_eq!(start_times(&doc, t), start, "the clock is kept");
    assert_eq!(doc.tree.active_animations[&t.0][0].delay_ms, 1000.0);
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (204, 51, 0),
        "20% along red → green"
    );
}

/// Row E: two animations of one name are matched by position, so a restyle
/// that changes nothing about them leaves each its own duration and clock.
/// Matching the first entry by name made the second a copy of the first:
/// `[10000, 10000]`.
#[test]
fn two_animations_of_one_name_keep_their_own_durations_and_clocks() {
    let mut doc = document(&format!(
        "{RED_GREEN} .d {{ animation: k 10s linear infinite, k 20s linear infinite; }} \
         .d.other {{ color: rgb(1, 2, 3); }}"
    ));
    let t = running(&mut doc, "d", &[3000.0, 5000.0]);
    let starts = start_times(&doc, t);
    assert_ne!(starts[0], starts[1], "precondition: two different clocks");

    restyle_class(&mut doc, t, "d other");

    let durations: Vec<f64> = doc.tree.active_animations[&t.0]
        .iter()
        .map(|a| a.duration_ms)
        .collect();
    assert_eq!(durations, [10_000.0, 20_000.0]);
    assert_eq!(start_times(&doc, t), starts, "each keeps its own clock");
}

// ── #781: implicit keyframes are the element's current base value ──────────

/// Row F, the issue's example: a `to`-only spin under a colour a class
/// changes. The implicit `from` covers only `transform`, the property the rule
/// names, so the spin writes no `color` and the class's colour shows.
#[test]
fn a_colour_class_change_reaches_a_spinner_with_a_to_only_rule() {
    let mut doc = document(
        ".sp { color: rgb(10, 20, 30); animation: spin 10s linear infinite; } \
         .sp.hot { color: rgb(200, 210, 220); } \
         @keyframes spin { to { transform: rotate(360deg); } }",
    );
    let t = running(&mut doc, "sp", &[1000.0]);
    let colour = |doc: &RinchDocument| {
        let c = doc.tree.get(t.0).unwrap().computed_style.color.unwrap();
        let [r, g, b, _] = c.to_rgba8().to_u8_array();
        (r, g, b)
    };
    assert!(
        !doc.tree.active_animations[&t.0][0].animates(TransitionProperty::Color),
        "a rule that names only `transform` animates only `transform`"
    );

    restyle_class(&mut doc, t, "sp hot");
    doc.tick_animations();

    assert_eq!(colour(&doc), (200, 210, 220), "as in Chrome 156");
}

/// Row G: the implicit `from` of a `to`-only colour animation is the base
/// value **now**, not when the animation started. 30% along blue → green.
#[test]
fn an_implicit_from_stop_follows_the_base_value_it_stands_for() {
    let mut doc = document(
        ".x.fd { background-color: rgb(255, 0, 0); animation: fade 10s linear infinite; } \
         .x.fd.blue { background-color: rgb(0, 0, 255); } \
         @keyframes fade { to { background-color: rgb(0, 255, 0); } }",
    );
    let t = running(&mut doc, "fd", &[3000.0]);
    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (179, 77, 0),
        "precondition: red base"
    );

    restyle_class(&mut doc, t, "fd blue");

    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (0, 77, 179),
        "from the new blue base"
    );
}

/// Row H: an `em` stop follows the element's font size on a class change.
#[test]
fn an_em_stop_follows_a_font_size_class_change() {
    let mut doc = document(
        ".gr { font-size: 10px; animation: grow 10s linear infinite; } \
         .gr.big { font-size: 40px; } \
         @keyframes grow { from { width: 1em; } to { width: 11em; } }",
    );
    let t = running(&mut doc, "gr", &[1000.0]);
    let width = |doc: &RinchDocument| match sample(doc, t, 0, 1000.0, TransitionProperty::Width) {
        AnimatableValue::Dimension(rinch_dom::computed_style::DimensionValue::Length(px)) => px,
        other => panic!("not a px length: {other:?}"),
    };
    assert_eq!(width(&doc), 20.0, "precondition: the 10px basis");

    restyle_class(&mut doc, t, "gr big");

    assert_eq!(width(&doc), 80.0, "40px + 10% of 400px, as in Chrome 156");
}

/// Row I: an explicit `from` that does not name `background-color` while the
/// `to` does. The property's value at 0% is the element's base value (Web
/// Animations' neutral keyframe), so it interpolates from red; it used to jump
/// straight to the `to` value.
#[test]
fn a_property_missing_from_an_explicit_from_starts_at_the_base_value() {
    let mut doc = document(
        ".x.part { background-color: rgb(255, 0, 0); animation: part 10s linear infinite; } \
         @keyframes part { from { opacity: 0.2; } \
                           to { opacity: 0.8; background-color: rgb(0, 255, 0); } }",
    );
    let t = running(&mut doc, "part", &[3000.0]);

    assert_eq!(
        bg_at(&doc, t, 3000.0),
        (179, 77, 0),
        "30% along red → green"
    );
}

/// Repeated names across the two maps: `k 1s` has finished (no fill, so its
/// entry is a settled marker in `finished_animations`, #783) while `k 20s`
/// still runs in `active_animations`. A restyle must give each spec its own
/// entry back by position, not the running one's clock to the finished spec.
#[test]
fn a_finished_and_a_running_animation_of_one_name_keep_their_own_entries() {
    let mut doc = document(&format!(
        "{RED_GREEN} .d {{ animation: k 1s linear, k 20s linear infinite; }} \
         .d.other {{ color: rgb(1, 2, 3); }}"
    ));
    // Both started 2000ms ago on the document's clock, so `k 1s` is over by
    // any later cascade's "now".
    let t = running(&mut doc, "d", &[2000.0, 2000.0]);
    doc.tick_animations();
    assert_eq!(
        doc.tree.finished_animations[&t.0].len(),
        1,
        "precondition: `k 1s` has finished"
    );
    let running_start = {
        let anims = doc.tree.active_animations.get_mut(&t.0).unwrap();
        assert_eq!(anims.len(), 1, "precondition: `k 20s` still runs");
        anims[0].start_time_ms -= 5000.0;
        anims[0].start_time_ms
    };

    restyle_class(&mut doc, t, "d other");

    let active = &doc.tree.active_animations[&t.0];
    assert_eq!(active.len(), 1, "only `k 20s` runs");
    assert_eq!(active[0].duration_ms, 20_000.0);
    assert_eq!(
        active[0].start_time_ms, running_start,
        "`k 20s` keeps its own clock"
    );
    assert_eq!(
        doc.tree.finished_animations[&t.0].len(),
        1,
        "`k 1s` stays finished"
    );
}
