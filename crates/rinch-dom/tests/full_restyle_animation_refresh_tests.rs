//! What a running `@keyframes` animation keeps, and what it takes afresh, across
//! `recompute_all_styles_full` — the theme-change path (issue #762, review round 3).
//!
//! #762 stopped that path from clearing `active_animations`, so a running
//! animation survives a theme change with its clock, as in a browser. Keeping
//! the whole entry kept too much: its keyframe stops were extracted from the
//! **old** base style against the **old** `@keyframes` rule, and its timing from
//! the old declaration. So during a full restyle — and only then, behind the
//! dedicated `NodeTree::refreshing_animations` flag — a kept animation keeps its
//! **clock** (`start_time_ms`, `paused_elapsed_ms`, `play_state`) and takes
//! everything else afresh: the keyframes are looked up again (and the animation
//! dropped if the rule is gone), the stops are re-extracted from the new base
//! style, and `duration_ms` / `delay_ms` come from the new declaration.
//!
//! # The oracle
//!
//! Chrome 153.0.8010.36, `reports/review-771b-probes/oracle-771b.html`: a
//! `<style>` element standing in for the theme sheet has its `textContent`
//! replaced while each animation's `currentTime` is seeked to 3000ms (1000ms for
//! the em-sized spinner). Every expected number below is a row of that page.
//!
//! | The swap | Chrome 153 |
//! |---|---|
//! | the theme sheet no longer defines the `@keyframes` | `getAnimations()` 1 → **0** |
//! | `animation-name` `k` → `k2` | a **new** animation, `currentTime` **0** |
//! | `a, b` → `b, a` | the same two objects, reordered, each with its own `currentTime` |
//! | the theme redefines the `@keyframes` body | same object, `currentTime` kept, **new** stops |
//! | `animation-duration` 10s → 20s | same object, `currentTime` 3000, progress **0.15** |
//! | a hidden panel shown by the theme, which also moves `font-size` 10px → 40px under a `1em → 11em` spinner | width **80px** at 1000ms |
//! | a `to`-only `@keyframes`, `color: var(--fg)` changed by the theme | `color` follows the theme, clock kept |
//!
//! Scope: this is the **theme path only**. A plain restyle — a class change —
//! still keeps a running animation's stops and timing whole (#766, #780, #781
//! stay open for that route); refreshing there would cost a keyframes lookup
//! and a stop extraction per animated node per cascade, which a theme toggle can
//! afford and a hover cannot.
//!
//! # Which fixture kills which mutant
//!
//! The table is filled from runs, not reasoning; see the round-3 section of
//! `reports/report-fix-762.md` for the runner.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::animation::{AnimationPlayState, AnimationResult};
use rinch_dom::transition::{AnimatableValue, TransitionProperty};

const VP: (f32, f32) = (800.0, 600.0);

/// Every animated box is 10x10 with a declared `line-height`, so nothing here is
/// measured from text.
const BOX: &str = "div.x { width: 10px; height: 10px; line-height: 10px; \
                   background-color: rgb(0, 0, 255); }";

/// A document whose theme sheet is `theme`, whose author sheet is `author`, laid
/// out at [`VP`] from the start — `set_viewport` before anything is built, as
/// `RinchApp::mount_component` does, so no resolve below is a viewport change
/// that would drop every cached style on its own.
fn document(theme: &str, author: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.set_viewport(VP.0, VP.1);
    doc.update_theme_variables(theme);
    doc.load_css(&format!("{BOX} {author}"));
    doc
}

fn element(doc: &mut RinchDocument, parent: NodeId, class: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "class", class);
    doc.append_child(parent, n);
    n
}

/// The theme-change path exactly as `RinchApp::resolve_and_repaint` takes it.
fn change_theme(doc: &mut RinchDocument, theme: &str) {
    doc.update_theme_variables(theme);
    doc.recompute_all_styles_full();
    doc.resolve_layout(VP.0, VP.1);
}

fn names(doc: &RinchDocument, node: NodeId) -> Vec<String> {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|anims| anims.iter().map(|a| a.name.clone()).collect())
        .unwrap_or_default()
}

/// Move every animation on `node` back by `ms`, as the oracle seeks
/// `currentTime`, and answer the start times they now carry, in order.
fn backdate(doc: &mut RinchDocument, node: NodeId, ms: &[f64]) -> Vec<f64> {
    let anims = doc
        .tree
        .active_animations
        .get_mut(&node.0)
        .expect("an animation should be running");
    assert_eq!(anims.len(), ms.len(), "one backdate per animation");
    anims
        .iter_mut()
        .zip(ms)
        .map(|(a, by)| {
            a.start_time_ms -= by;
            a.start_time_ms
        })
        .collect()
}

fn start_times(doc: &RinchDocument, node: NodeId) -> Vec<f64> {
    doc.tree
        .active_animations
        .get(&node.0)
        .map(|anims| anims.iter().map(|a| a.start_time_ms).collect())
        .unwrap_or_default()
}

/// The value of `property` on `node`'s `index`th animation, `elapsed_ms` into it.
/// Clock-free: sampled at `start_time_ms + elapsed_ms`, never at "now".
fn sample(
    doc: &RinchDocument,
    node: NodeId,
    index: usize,
    elapsed_ms: f64,
    property: TransitionProperty,
) -> AnimatableValue {
    let anim = &doc.tree.active_animations[&node.0][index];
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

fn px(value: &AnimatableValue) -> f32 {
    match value {
        AnimatableValue::Dimension(rinch_dom::computed_style::DimensionValue::Length(px)) => *px,
        other => panic!("not a px length: {other:?}"),
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0
}

// ── required: a `@keyframes` rule the new theme no longer carries ───────────

/// **The regression #762 introduced and this round removes.** Chrome's third
/// row: the declaration still names `gone`, but no `@keyframes gone` exists
/// after the swap, so the animation is cancelled and the element returns to its
/// base style. `main` answered 0 by accident — it cleared the map and never
/// re-registered anything. Keeping the entry by name without asking whether its
/// rule still exists left `gone` interpolating, and the frame clock running, for
/// the life of the element.
///
/// The theme sheet is where this happens: `set_theme_css` takes arbitrary CSS,
/// and rinch's own `generate_theme_css_string` already puts component
/// `@keyframes` in it.
#[test]
fn a_theme_that_drops_the_keyframes_rule_stops_the_animation() {
    let theme1 = ":root { --probe: 1; } \
                  @keyframes gone { from { background-color: rgb(255, 0, 0); } \
                                    to { background-color: rgb(0, 255, 0); } }";
    let mut doc = document(theme1, ".t { animation: gone 10s linear infinite; }");
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(names(&doc, t), ["gone"], "precondition: `gone` is running");
    backdate(&mut doc, t, &[3000.0]);

    change_theme(&mut doc, ":root { --probe: 2; }");

    assert_eq!(
        doc.tree.get(t.0).unwrap().animation_specs.len(),
        1,
        "precondition: the declaration still names `gone` — only the rule went"
    );
    assert_eq!(
        names(&doc, t),
        Vec::<String>::new(),
        "an animation whose @keyframes rule no longer exists is cancelled, as in \
         Chrome 153"
    );
}

// ── required: matching is by name, not by position ─────────────────────────

/// Chrome's rename row: `k` → `k2` is a **new** animation from t=0, not `k`'s
/// clock carried over to a different name.
///
/// Every other fixture has exactly one animation, where "the entry with this
/// name" and "the entry at this index" are the same entry — so a keep-by-index
/// `start_animations` (mutant MA) survived the whole suite. This one and the
/// reorder below are what tell the two apart.
#[test]
fn a_theme_that_renames_the_animation_starts_a_new_one_from_zero() {
    let author = "@keyframes k  { from { background-color: rgb(255, 0, 0); } \
                                  to { background-color: rgb(0, 255, 0); } } \
                  @keyframes k2 { from { background-color: rgb(0, 0, 0); } \
                                  to { background-color: rgb(255, 255, 255); } } \
                  .t { animation: k 10s linear infinite; }";
    let mut doc = document(":root { --probe: 1; }", author);
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    let old_start = backdate(&mut doc, t, &[3000.0])[0];

    let swap_started = now_ms();
    change_theme(
        &mut doc,
        ":root { --probe: 2; } body .t { animation-name: k2; }",
    );

    assert_eq!(names(&doc, t), ["k2"], "only the renamed animation runs");
    let new_start = start_times(&doc, t)[0];
    assert!(
        new_start >= swap_started,
        "`k2` starts from t=0 at the swap ({new_start} < {swap_started}), not on \
         `k`'s clock ({old_start})"
    );
    assert_eq!(
        rgb(&sample(
            &doc,
            t,
            0,
            0.0,
            TransitionProperty::BackgroundColor
        )),
        (0, 0, 0),
        "…and plays `k2`'s keyframes, not `k`'s"
    );
}

/// Chrome's reorder row: `a, b` → `b, a` keeps both animations, each **with its
/// own clock**, in the new order. The two clocks are backdated by different
/// amounts so that swapping them is an observable mistake.
#[test]
fn a_theme_that_reorders_two_animations_keeps_each_clock_with_its_name() {
    let author = "@keyframes a { from { width: 10px; } to { width: 110px; } } \
                  @keyframes b { from { height: 10px; } to { height: 110px; } } \
                  .t { animation: a 10s linear infinite, b 10s linear infinite; }";
    let mut doc = document(":root { --probe: 1; }", author);
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        names(&doc, t),
        ["a", "b"],
        "precondition: both run, a first"
    );
    let starts = backdate(&mut doc, t, &[3000.0, 5000.0]);
    let (a_start, b_start) = (starts[0], starts[1]);

    change_theme(
        &mut doc,
        ":root { --probe: 2; } \
         body .t { animation: b 10s linear infinite, a 10s linear infinite; }",
    );

    assert_eq!(
        names(&doc, t),
        ["b", "a"],
        "the new order is the declaration's"
    );
    assert_eq!(
        start_times(&doc, t),
        [b_start, a_start],
        "each animation keeps its own clock across the reorder"
    );
}

// ── R: what a kept animation takes afresh ──────────────────────────────────

/// #766 on the theme path: the theme redefines `@keyframes kk` while `kk` runs.
/// Chrome keeps `currentTime` and plays the **new** body at once — at 3000ms
/// into a 10s black → white animation that is `rgb(77, 77, 77)`. Keeping the
/// entry whole left the old red → green stops running: `rgb(179, 77, 0)`.
#[test]
fn a_theme_that_redefines_the_keyframes_body_plays_the_new_one_on_the_old_clock() {
    let theme1 = ":root { --probe: 1; } \
                  @keyframes kk { from { background-color: rgb(255, 0, 0); } \
                                  to { background-color: rgb(0, 255, 0); } }";
    let theme2 = ":root { --probe: 2; } \
                  @keyframes kk { from { background-color: rgb(0, 0, 0); } \
                                  to { background-color: rgb(255, 255, 255); } }";
    let mut doc = document(theme1, ".t { animation: kk 10s linear infinite; }");
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    let start = backdate(&mut doc, t, &[3000.0])[0];
    assert_eq!(
        rgb(&sample(
            &doc,
            t,
            0,
            3000.0,
            TransitionProperty::BackgroundColor
        )),
        (179, 77, 0),
        "precondition: 30% along the old red → green body"
    );

    change_theme(&mut doc, theme2);

    assert_eq!(start_times(&doc, t), [start], "the clock is kept");
    assert_eq!(
        rgb(&sample(
            &doc,
            t,
            0,
            3000.0,
            TransitionProperty::BackgroundColor
        )),
        (77, 77, 77),
        "…and the new body plays on it: 30% along black → white, as in Chrome 153"
    );
}

/// #780 on the theme path: `animation-duration` 10s → 20s. Chrome keeps
/// `currentTime` at 3000 and re-times around it — progress 0.15, not 0.3. The
/// animated colour is sampled at 3000ms on the kept clock, off both endpoints.
#[test]
fn a_theme_that_changes_the_duration_re_times_around_the_kept_clock() {
    let author = "@keyframes k { from { background-color: rgb(255, 0, 0); } \
                                 to { background-color: rgb(0, 255, 0); } } \
                  .t { animation: k 10s linear infinite; }";
    let mut doc = document(":root { --probe: 1; }", author);
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    let start = backdate(&mut doc, t, &[3000.0])[0];

    change_theme(
        &mut doc,
        ":root { --probe: 2; } body .t { animation-duration: 20s; }",
    );

    assert_eq!(start_times(&doc, t), [start], "the clock is kept");
    assert_eq!(
        doc.tree.active_animations[&t.0][0].duration_ms, 20_000.0,
        "the new duration is taken"
    );
    // 15% of red → green.
    assert_eq!(
        rgb(&sample(
            &doc,
            t,
            0,
            3000.0,
            TransitionProperty::BackgroundColor
        )),
        (217, 38, 0),
        "3000ms into 20s is 15% along, where the old 10s would say 30%"
    );
}

/// A panel hidden by the author sheet and shown by the theme, which also moves
/// the spinner's `font-size` from 10px to 40px under a `1em → 11em` width
/// animation. Chrome: **80px** at 1000ms (40px + 10% of 400px).
///
/// This is the shape that falsified round 2's docs. The full restyle **does**
/// reach #747's restart walk: it runs on the panel's cascade, before the
/// spinner's own, and mints the spinner's entry from its pre-restyle
/// `computed_style` — a 10px `em` basis, 20px at 1000ms. The spinner's own
/// cascade then used to keep those stops by name. The refresh re-extracts them.
#[test]
fn a_panel_shown_by_the_theme_starts_its_spinner_on_the_new_em_basis() {
    let author = ".p { display: none; } \
                  .x.t { font-size: var(--fs); animation: grow 10s linear infinite; } \
                  @keyframes grow { from { width: 1em; } to { width: 11em; } }";
    let mut doc = document(":root { --fs: 10px; }", author);
    let body = doc.body();
    let panel = element(&mut doc, body, "p");
    let inner = element(&mut doc, panel, "");
    let t = element(&mut doc, inner, "x t");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(
        names(&doc, t),
        Vec::<String>::new(),
        "precondition: the spinner is hidden and runs nothing"
    );

    change_theme(
        &mut doc,
        ":root { --fs: 40px; } body .p { display: block; }",
    );

    assert_eq!(names(&doc, t), ["grow"], "the shown spinner runs");
    assert_eq!(
        doc.tree.get(t.0).unwrap().computed_style.font_size,
        40.0,
        "precondition: the theme's font-size reached the spinner"
    );
    assert_eq!(
        px(&sample(&doc, t, 0, 1000.0, TransitionProperty::Width)),
        80.0,
        "`1em → 11em` resolves against the new 40px, as in Chrome 153"
    );
}

/// #781 on the theme path: a `to`-only `@keyframes` — the `rinch-button-spin` /
/// `rinch-action-icon-spin` shape — gets an implicit `from` stop built from the
/// base style, and that stop carries `color`. A theme that moves `--fg` must
/// move the element's colour; keeping the stops whole pinned it at the colour
/// the animation started with.
#[test]
fn a_theme_colour_change_reaches_a_spinner_with_an_implicit_from_stop() {
    let author = ".x.t { color: var(--fg); animation: spin 10s linear infinite; } \
                  @keyframes spin { to { transform: rotate(360deg); } }";
    let mut doc = document(":root { --fg: rgb(10, 20, 30); }", author);
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    let start = backdate(&mut doc, t, &[3000.0])[0];
    let colour = |doc: &RinchDocument| {
        let c = doc.tree.get(t.0).unwrap().computed_style.color.unwrap();
        let [r, g, b, _] = c.to_rgba8().to_u8_array();
        (r, g, b)
    };
    doc.tick_animations();
    assert_eq!(colour(&doc), (10, 20, 30), "precondition: theme 1's colour");

    change_theme(&mut doc, ":root { --fg: rgb(200, 210, 220); }");
    doc.tick_animations();

    assert_eq!(start_times(&doc, t), [start], "the clock is kept");
    assert_eq!(
        colour(&doc),
        (200, 210, 220),
        "the element takes theme 2's colour through an animation tick"
    );
}

/// A **paused** animation keeps its frozen time across a theme change: the
/// refresh re-reads the rule, the stops and the timing, and leaves
/// `paused_elapsed_ms` and `play_state` exactly as they were. Frozen off zero
/// (3000ms), and re-timed 10s → 20s by the same swap, so a refresh that
/// rescaled the frozen time to the new duration would move it — Chrome keeps
/// `currentTime` 3000 and changes the progress.
///
/// This is the constraint PR #779 (#763) rests on: a paused entry minted
/// afresh carries `paused_elapsed_ms: Some(0.0)`, which would jump a frozen
/// animation to its start.
#[test]
fn a_paused_animation_keeps_its_frozen_time_across_a_theme_change() {
    let author = "@keyframes k { from { background-color: rgb(255, 0, 0); } \
                                 to { background-color: rgb(0, 255, 0); } } \
                  .t { animation: k 10s linear infinite; } \
                  .t.held { animation-play-state: paused; }";
    let mut doc = document(":root { --probe: 1; }", author);
    let body = doc.body();
    let t = element(&mut doc, body, "x t");
    doc.resolve_layout(VP.0, VP.1);
    backdate(&mut doc, t, &[3000.0]);
    doc.set_attribute(t, "class", "x t held");
    doc.resolve_layout(VP.0, VP.1);
    let frozen = doc.tree.active_animations[&t.0][0]
        .paused_elapsed_ms
        .expect("precondition: the animation is paused");
    assert!(
        (3000.0..3500.0).contains(&frozen),
        "precondition: frozen about 3000ms in, got {frozen}"
    );

    change_theme(
        &mut doc,
        ":root { --probe: 2; } body .t { animation-duration: 20s; }",
    );

    let anim = &doc.tree.active_animations[&t.0][0];
    assert_eq!(
        anim.duration_ms, 20_000.0,
        "precondition: the swap re-timed it"
    );
    assert_eq!(
        anim.paused_elapsed_ms,
        Some(frozen),
        "the frozen time is kept as elapsed time, not reset and not rescaled"
    );
    assert_eq!(
        anim.play_state,
        AnimationPlayState::Paused,
        "and it stays paused"
    );
}
