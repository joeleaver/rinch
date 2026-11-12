//! A `@keyframes` stop is computed at each sample, against the element as it
//! is at that sample (issues #2056, #2058, #2059).
//!
//! Stops used to be resolved once, when they were extracted, against the
//! element's **base** style: an `em` against the unanimated `font-size`, a
//! `currentcolor` against the unanimated `color`, a `rem` against the document
//! node's 16px instead of `<html>`'s font size, and the implicit `from` of a
//! property whose base value is `currentcolor` against the base colour. A
//! transition on a property an animation drives started anyway, and the
//! animation's implicit stop took the transition's sample.
//!
//! # The oracle
//!
//! Chrome 157, headless: each animation paused, seeked with `currentTime`,
//! then `getComputedStyle` read. Parent `font-size: 10px`, 1000ms linear
//! unless noted.
//!
//! | base | keyframes | 250 | 500 | 750 |
//! |---|---|---|---|---|
//! | `width:100px; font-size:10px` | `to { font-size: 30px; width: 10em }` | width 112.5 | 150 | 212.5 |
//! | (inherits 10px) | `to { font-size: 2em }` | 12.5 | 15 | 17.5 |
//! | (inherits 10px) | `to { font-size: 150% }` | 11.25 | 12.5 | 13.75 |
//! | `font-size: 30px` (parent 10px) | `to { font-size: 2em }` | 27.5 | 25 | 22.5 |
//! | `font-size: 30px` (parent 10px) | `to { font-size: 150% }` | 26.25 | 22.5 | 18.75 |
//! | `color:red; border-left:10px solid` | `to { color: blue; border-left-color: lime }` | (143,64,48) | (64,128,64) | (16,191,48) |
//! | `color:red; border-left:10px solid` | `from { border-left-color: lime } to { color: blue; border-left-color: currentcolor }` | (48,191,16) | (64,128,64) | (48,64,143) |
//!
//! `html { font-size: 32px }`, `from { width: 1rem } to { width: 11rem }`,
//! 10s, at 3000ms: **128px** (whether the root was 32px from the start or
//! changed to it mid-animation).
//!
//! `background-color: red; transition: background-color 10s linear;
//! animation: t 10s linear infinite` with `@keyframes t { to {
//! background-color: rgb(0,255,0) } }`, the base moved to blue at 3000ms:
//! `getAnimations()` lists only the `CSSAnimation` (no transition starts), and
//! the colour is `rgb(0, 77, 179)` — the implicit `from` is the new base.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A `div` with inline style `base` and `animation: k <timing>`, under
/// `@keyframes k { keyframes }` and the extra `css`, inside a container whose
/// `font-size` is 10px. Laid out once, so the animation is minted.
fn animated_full(
    css: &str,
    base: &str,
    keyframes: &str,
    timing: &str,
    text: bool,
) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(1_000_000.0));
    doc.load_css(&format!(
        "html, body {{ margin: 0; }}
         .c {{ width: 400px; font-size: 10px; line-height: 10px; }}
         .b {{ height: 20px; }}
         {css}
         @keyframes k {{ {keyframes} }}"
    ));
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "class", "c");
    doc.append_child(body, c);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "b");
    doc.set_attribute(div, "style", &format!("{base}; animation: k {timing}"));
    if text {
        let t = doc.create_text("x");
        doc.append_child(div, t);
    }
    doc.append_child(c, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.active_animations.contains_key(&div.0),
        "positive control: the animation must be running after the first layout"
    );
    (doc, div)
}

fn animated(base: &str, keyframes: &str) -> (RinchDocument, NodeId) {
    animated_full("", base, keyframes, "1000ms linear", false)
}

/// Tick the div's animation to `ms` after its start the way a frame does, and
/// lay out again.
fn tick_to(doc: &mut RinchDocument, div: NodeId, ms: f64) {
    let now = doc.animation_now_ms();
    for anim in doc.tree.active_animations.get_mut(&div.0).unwrap() {
        anim.start_time_ms = now - ms;
    }
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
}

/// Within 0.5: the laid-out box is snapped to whole pixels (#2437), so
/// Chrome's 112.5 lays out as 113.
fn close(got: f32, want: f32, what: &str) {
    assert!(
        (got - want).abs() <= 0.5,
        "{what}: Chrome gives {want}, got {got}"
    );
}

fn rgb(c: Option<peniko::Color>) -> (f32, f32, f32) {
    let c = c.expect("a colour").to_rgba8();
    (c.r as f32, c.g as f32, c.b as f32)
}

fn close_rgb(got: (f32, f32, f32), want: (f32, f32, f32), what: &str) {
    assert!(
        (got.0 - want.0).abs() <= 1.0
            && (got.1 - want.1).abs() <= 1.0
            && (got.2 - want.2).abs() <= 1.0,
        "{what}: Chrome gives {want:?}, got {got:?}"
    );
}

// --- #2056 -------------------------------------------------------------------

#[test]
fn an_em_stop_resolves_against_the_animated_font_size() {
    let (mut doc, div) = animated(
        "width: 100px; font-size: 10px",
        "to { font-size: 30px; width: 10em; }",
    );
    for (ms, want) in [(250.0, 112.5), (500.0, 150.0), (750.0, 212.5)] {
        tick_to(&mut doc, div, ms);
        let n = doc.tree.get(div.0).unwrap();
        close(n.layout.width, want, &format!("laid-out width at {ms}ms"));
    }
}

#[test]
fn a_font_size_em_stop_resolves_against_the_parent_font_size() {
    let (mut doc, div) = animated_full("", "", "to { font-size: 2em; }", "1000ms linear", true);
    for (ms, want) in [(250.0, 12.5), (500.0, 15.0), (750.0, 17.5)] {
        tick_to(&mut doc, div, ms);
        let fs = doc.tree.get(div.0).unwrap().computed_style.font_size;
        close(fs, want, &format!("font-size at {ms}ms"));
    }
}

#[test]
fn a_font_size_percentage_stop_resolves_against_the_parent_font_size() {
    let (mut doc, div) = animated_full("", "", "to { font-size: 150%; }", "1000ms linear", true);
    for (ms, want) in [(250.0, 11.25), (500.0, 12.5), (750.0, 13.75)] {
        tick_to(&mut doc, div, ms);
        let fs = doc.tree.get(div.0).unwrap().computed_style.font_size;
        close(fs, want, &format!("font-size at {ms}ms"));
    }
}

/// Off the fixed point of the two above, where the element's own font size is
/// its parent's: here the own size is 30px, so resolving against it would run
/// to 60px (2em) or 45px (150%).
#[test]
fn a_font_size_stop_resolves_against_the_parent_not_the_own_size() {
    for (kf, want) in [
        ("to { font-size: 2em; }", [27.5, 25.0, 22.5]),
        ("to { font-size: 150%; }", [26.25, 22.5, 18.75]),
    ] {
        let (mut doc, div) = animated_full("", "font-size: 30px", kf, "1000ms linear", true);
        for (ms, want) in [250.0, 500.0, 750.0].into_iter().zip(want) {
            tick_to(&mut doc, div, ms);
            let fs = doc.tree.get(div.0).unwrap().computed_style.font_size;
            close(fs, want, &format!("{kf}: font-size at {ms}ms"));
        }
    }
}

#[test]
fn an_implicit_currentcolor_from_follows_the_animated_color() {
    let (mut doc, div) = animated(
        "color: rgb(255, 0, 0); border-left: 10px solid",
        "to { color: rgb(0, 0, 255); border-left-color: rgb(0, 255, 0); }",
    );
    for (ms, want) in [
        (250.0, (143.0, 64.0, 48.0)),
        (500.0, (64.0, 128.0, 64.0)),
        (750.0, (16.0, 191.0, 48.0)),
    ] {
        tick_to(&mut doc, div, ms);
        let n = doc.tree.get(div.0).unwrap();
        close_rgb(
            rgb(n.computed_style.border_left_color),
            want,
            &format!("border-left-color at {ms}ms"),
        );
    }
}

#[test]
fn an_authored_currentcolor_stop_follows_the_animated_color() {
    let (mut doc, div) = animated(
        "color: rgb(255, 0, 0); border-left: 10px solid",
        "from { border-left-color: rgb(0, 255, 0); }
         to { color: rgb(0, 0, 255); border-left-color: currentcolor; }",
    );
    for (ms, want) in [
        (250.0, (48.0, 191.0, 16.0)),
        (500.0, (64.0, 128.0, 64.0)),
        (750.0, (48.0, 64.0, 143.0)),
    ] {
        tick_to(&mut doc, div, ms);
        let n = doc.tree.get(div.0).unwrap();
        close_rgb(
            rgb(n.computed_style.border_left_color),
            want,
            &format!("border-left-color at {ms}ms"),
        );
    }
}

/// The same `currentcolor` stop, sampled by the cascade rather than a tick: an
/// unrelated class change on the node re-cascades it mid-animation.
#[test]
fn a_cascade_samples_a_currentcolor_stop_at_the_animated_color_too() {
    let (mut doc, div) = animated_full(
        ".b.o { opacity: 0.9; }",
        "color: rgb(255, 0, 0); border-left: 10px solid",
        "to { color: rgb(0, 0, 255); border-left-color: rgb(0, 255, 0); }",
        "1000ms linear",
        false,
    );
    let now = doc.animation_now_ms();
    for anim in doc.tree.active_animations.get_mut(&div.0).unwrap() {
        anim.start_time_ms = now - 500.0;
    }
    doc.set_attribute(div, "class", "b o");
    doc.resolve_layout(800.0, 600.0);
    let n = doc.tree.get(div.0).unwrap();
    close_rgb(
        rgb(n.computed_style.border_left_color),
        (64.0, 128.0, 64.0),
        "border-left-color after a cascade at 500ms",
    );
}

// --- #2058 -------------------------------------------------------------------

const REM: &str = "from { width: 1rem; } to { width: 11rem; }";

#[test]
fn a_rem_stop_resolves_against_the_html_font_size() {
    let (mut doc, div) = animated_full(
        "html { font-size: 32px; }",
        "",
        REM,
        "10s linear infinite",
        false,
    );
    tick_to(&mut doc, div, 3000.0);
    close(
        doc.tree.get(div.0).unwrap().layout.width,
        128.0,
        "width at 3000ms",
    );
}

#[test]
fn a_rem_stop_follows_a_root_font_size_change() {
    let (mut doc, div) = animated_full(
        "html { font-size: 16px; } html.big { font-size: 32px; }",
        "",
        REM,
        "10s linear infinite",
        false,
    );
    tick_to(&mut doc, div, 3000.0);
    close(
        doc.tree.get(div.0).unwrap().layout.width,
        64.0,
        "width at 3000ms against a 16px root",
    );
    let html = NodeId(doc.tree.html_id);
    doc.set_attribute(html, "class", "big");
    doc.resolve_layout(800.0, 600.0);
    tick_to(&mut doc, div, 3000.0);
    close(
        doc.tree.get(div.0).unwrap().layout.width,
        128.0,
        "width at 3000ms once the root is 32px",
    );
}

// --- #2059 -------------------------------------------------------------------

#[test]
fn no_transition_starts_on_a_property_an_animation_drives() {
    let (mut doc, div) = animated_full(
        ".b { background-color: rgb(255, 0, 0); transition: background-color 10s linear; }
         .b.blue { background-color: rgb(0, 0, 255); }",
        "",
        "to { background-color: rgb(0, 255, 0); }",
        "10s linear infinite",
        false,
    );
    let now = doc.animation_now_ms();
    for anim in doc.tree.active_animations.get_mut(&div.0).unwrap() {
        anim.start_time_ms = now - 3000.0;
    }
    doc.set_attribute(div, "class", "b blue");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree
            .active_transitions
            .get(&div.0)
            .is_none_or(|m| m.is_empty()),
        "Chrome starts no transition on a property a running animation animates"
    );
    let bg = |doc: &RinchDocument| match &doc.tree.get(div.0).unwrap().computed_style.background {
        rinch_dom::computed_style::BackgroundValue::Color(c) => rgb(Some(*c)),
        other => panic!("a colour background, got {other:?}"),
    };
    close_rgb(
        bg(&doc),
        (0.0, 77.0, 179.0),
        "background after the class change",
    );
    tick_to(&mut doc, div, 3000.0);
    close_rgb(bg(&doc), (0.0, 77.0, 179.0), "background on the next tick");
}

/// An animation still in its delay, with no backwards fill, has no effect yet,
/// so a base change under it **does** start a transition (review of #2538).
/// Chrome 157: `animation: w 1s linear 10s` plus a base `width` change gives
/// `getAnimations()` = `CSSTransition:width, CSSAnimation:w`.
#[test]
fn a_transition_starts_under_an_animation_still_in_its_delay() {
    let (mut doc, div) = animated_full(
        ".b { width: 100px; transition: width 10s linear; }
         .b.wide { width: 200px; }",
        "",
        "to { width: 300px; }",
        "1s linear 10s",
        false,
    );
    doc.set_attribute(div, "class", "b wide");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.active_transitions.get(&div.0).map(|m| m.len()),
        Some(1),
        "Chrome starts the `width` transition: the delayed animation has no effect yet"
    );
}

/// A paused animation's `em` stop beside its `font-size` animation is resolved
/// at the cascade, and a tick then has nothing to resolve: the sample is
/// constant, so the frame basis agrees with the one the stops hold, and the
/// tick leaves the node clean.
#[test]
fn a_paused_em_animation_does_no_resolve_work_on_a_tick() {
    let (mut doc, div) = animated_full(
        "",
        "width: 100px; font-size: 10px",
        "to { font-size: 30px; width: 10em; }",
        "1000ms linear paused",
        false,
    );
    let w = doc.tree.get(div.0).unwrap().layout.width;
    let _ = doc.take_dirty_nodes();
    doc.tree.paint_dirty_nodes.clear();
    doc.tree.get_mut(div.0).unwrap().dirty = rinch_dom::DirtyFlags::empty();
    doc.advance_animation_clock_ms(100.0);
    assert!(!doc.tick_animations(), "paused: no frame asked for");
    assert!(
        doc.tree.paint_dirty_nodes.is_empty() && !doc.tree.dirty_nodes.contains(&div.0),
        "a paused `em` animation's tick re-resolved its stops and dirtied the node"
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.width,
        w,
        "and nothing moved"
    );
}
