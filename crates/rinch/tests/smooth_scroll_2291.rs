//! #2291: CSS `scroll-behavior: smooth` for programmatic scrolls on desktop.
//!
//! A `scroll_top` / `scroll_left` write, `NodeHandle::scroll_to` and a
//! scroll-into-view of a box whose computed `scroll-behavior` is `smooth`
//! animate on the curve Chrome 158 runs (measured: `sqrt(|delta|)` frames of
//! 60 Hz capped at 1.5 s, `cubic-bezier(0.42, 0, 0, 1)`, from the write),
//! instead of jumping in one frame. Driven through `rinch::testing`, the
//! virtual clock stepped 10 ms a turn. Every scroller starts part-way down,
//! off offset 0, and the samples are taken mid-curve.

#![cfg(all(feature = "testing", feature = "windowing"))]

use rinch::prelude::*;
use rinch::testing::TestApp;
use std::time::Duration;

const START: f64 = 333.0;
const FRAME: Duration = Duration::from_millis(10);

/// A 200x300 scroller `#s` (with `style`) holding `content` px, at `START`.
fn scroller(style: &'static str, content: u32) -> TestApp {
    let mut t = TestApp::mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let s = scope.create_element("div");
        s.set_attribute("id", "s");
        s.set_attribute(
            "style",
            &format!("width: 200px; height: 300px; overflow-y: auto; {style}"),
        );
        let inner = scope.create_element("div");
        inner.set_attribute("style", &format!("height: {content}px"));
        s.append_child(&inner);
        root.append_child(&s);
        root
    });
    t.handle("#s")
        .scroll_to(None, Some(START), ScrollBehavior::Instant);
    t.settle();
    assert_eq!(t.handle("#s").scroll_top(), START);
    t
}

fn top(t: &TestApp) -> f64 {
    t.handle("#s").scroll_top()
}

/// `cubic-bezier(0.42, 0, 0, 1)` at `x`, the curve the implementation should
/// follow, solved independently by bisection.
fn ease(x: f64) -> f64 {
    if x >= 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..80 {
        let t = (lo + hi) / 2.0;
        let u = 1.0 - t;
        if 3.0 * u * u * t * 0.42 + t * t * t < x {
            lo = t;
        } else {
            hi = t;
        }
    }
    let t = (lo + hi) / 2.0;
    3.0 * (1.0 - t) * t * t + t * t * t
}

/// Frames until the app stops animating, at most `max`: `scrollTop` after
/// each.
fn frames(t: &mut TestApp, max: usize) -> Vec<f64> {
    let mut out = Vec::new();
    for _ in 0..max {
        t.advance_time(FRAME);
        out.push(top(t));
        if !t.app().wheel_scroll_animating() {
            break;
        }
    }
    out
}

/// A smooth write moves nothing at once (`scrollTop` reads the old offset,
/// as in a browser), then follows Chrome's curve over `sqrt(400)` = 20
/// frames of 60 Hz (333 ms), each frame on the device pixel grid, lands
/// exactly and goes idle.
///
/// Mutants it kills: no smooth path (jumps), the wheel's `ease-in-out`
/// (about 80 px at 100 ms, not 222), the 12-frame cap the source names
/// (lands at 200 ms), a curve started at the first frame rather than at the
/// write (one frame late throughout).
#[test]
fn a_smooth_scroll_top_write_follows_chromes_curve() {
    let mut t = scroller("scroll-behavior: smooth", 3000);
    t.handle("#s").set_scroll_top(START + 400.0);
    assert_eq!(top(&t), START, "a smooth write moves nothing at once");
    let samples = frames(&mut t, 60);
    let duration = 400.0_f64.sqrt() / 60.0 * 1000.0;
    for (i, got) in samples.iter().enumerate() {
        let ms = 10.0 * (i + 1) as f64;
        let want = START + 400.0 * ease(ms / duration);
        assert!(
            (got - want).abs() <= 1.0,
            "at {ms} ms: {got} where the curve is at {want:.2} (all: {samples:?})"
        );
    }
    assert_eq!(
        samples.len(),
        34,
        "lands in the frame after 333 ms: {samples:?}"
    );
    assert_eq!(*samples.last().unwrap(), START + 400.0);
    t.settle();
    assert!(!t.app().wheel_scroll_animating());
    assert_eq!(top(&t), START + 400.0);
}

/// The same write to an `auto` scroller jumps, as before.
#[test]
fn an_auto_scroller_still_jumps() {
    let mut t = scroller("", 3000);
    t.handle("#s").set_scroll_top(START + 400.0);
    assert_eq!(top(&t), START + 400.0);
    t.advance_time(FRAME);
    assert!(!t.app().wheel_scroll_animating());
}

/// The duration grows with `sqrt(|delta|)` and stops at 1.5 s: 2500 px takes
/// 833 ms, 10 000 px 1.5 s (Chrome 158: 820 ms and 1.48 s).
#[test]
fn a_longer_jump_takes_longer_up_to_one_and_a_half_seconds() {
    for (delta, ms) in [(2500.0, 840), (10_000.0, 1500)] {
        let mut t = scroller("scroll-behavior: smooth", 12_000);
        t.handle("#s").set_scroll_top(START + delta);
        let samples = frames(&mut t, 400);
        let landed = samples
            .iter()
            .position(|&v| v == START + delta)
            .map(|i| 10 * (i + 1));
        assert_eq!(landed, Some(ms), "{delta} px");
    }
}

/// `scroll_to` with an explicit behavior overrides the stylesheet both ways,
/// as `scrollTo({behavior})` does: `Instant` jumps a smooth scroller and
/// drops the animation running there; `Smooth` animates an `auto` one.
#[test]
fn an_explicit_behavior_overrides_the_stylesheet() {
    let mut t = scroller("scroll-behavior: smooth", 3000);
    t.handle("#s").set_scroll_top(START + 400.0);
    frames(&mut t, 5);
    let mid = top(&t);
    assert!(mid > START && mid < START + 400.0, "mid-glide: {mid}");
    t.handle("#s")
        .scroll_to(None, Some(100.0), ScrollBehavior::Instant);
    assert_eq!(top(&t), 100.0);
    t.advance_time(FRAME);
    t.advance_time(FRAME);
    assert_eq!(top(&t), 100.0, "the glide it replaced does not resume");
    assert!(!t.app().wheel_scroll_animating());

    let mut t = scroller("", 3000);
    t.handle("#s")
        .scroll_to(None, Some(START + 400.0), ScrollBehavior::Smooth);
    assert_eq!(top(&t), START);
    let samples = frames(&mut t, 60);
    assert!(samples.len() > 20, "it animated: {samples:?}");
    assert_eq!(top(&t), START + 400.0);
}

/// A second smooth write mid-way retargets from where the scroll is: no
/// jump back, and it ends at the new target.
#[test]
fn a_second_write_retargets_from_where_it_is() {
    let mut t = scroller("scroll-behavior: smooth", 3000);
    t.handle("#s").set_scroll_top(START + 400.0);
    frames(&mut t, 8);
    let before = top(&t);
    t.handle("#s").set_scroll_top(START + 800.0);
    t.advance_time(FRAME);
    let after = top(&t);
    assert!(after >= before, "carried on from {before}, got {after}");
    frames(&mut t, 200);
    assert_eq!(top(&t), START + 800.0);
}

/// The switch (the hook for the OS reduced-motion setting, #2209): off, a
/// smooth write lands on the next frame.
#[test]
fn the_switch_turns_it_off() {
    let mut t = scroller("scroll-behavior: smooth", 3000);
    t.app().set_smooth_programmatic_scroll(false);
    t.handle("#s").set_scroll_top(START + 400.0);
    t.advance_time(FRAME);
    assert_eq!(top(&t), START + 400.0);
    assert!(!t.app().wheel_scroll_animating());
}

/// The page scroller (`<body>` here, standing for the viewport) takes the
/// root element's `scroll-behavior`, as Chrome's viewport does; `<body>`'s own
/// value does not apply to it.
#[test]
fn the_page_takes_the_root_elements_scroll_behavior() {
    for (css, smooth) in [
        ("html { scroll-behavior: smooth }", true),
        ("body { scroll-behavior: smooth }", false),
    ] {
        let mut t = TestApp::mount(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let style = scope.create_element("style");
            style.append_child(&scope.create_text(css));
            root.append_child(&style);
            let tall = scope.create_element("div");
            tall.set_attribute("style", "height: 5000px");
            root.append_child(&tall);
            root
        });
        let body = t.query("body").expect("a body");
        body.set_scroll_top(500.0);
        if smooth {
            assert_eq!(body.scroll_top(), 0.0, "{css}: nothing at once");
            for _ in 0..10 {
                t.advance_time(FRAME);
            }
            let mid = body.scroll_top();
            assert!(
                mid > 0.0 && mid < 500.0,
                "{css}: mid-glide at 100 ms: {mid}"
            );
            for _ in 0..60 {
                t.advance_time(FRAME);
            }
        }
        assert_eq!(body.scroll_top(), 500.0, "{css}");
    }
}

/// A scroll-into-view moves an `auto` outer scroller at once and animates a
/// smooth inner one, and both end where an all-`auto` reveal puts them (the
/// outer one is placed for the inner one's *final* offset).
#[test]
fn scroll_into_view_animates_the_smooth_scroller_only() {
    let mount = |inner_style: &'static str| {
        let mut t = TestApp::mount(move |scope: &mut RenderScope| {
            let outer = scope.create_element("div");
            outer.set_attribute("id", "outer");
            outer.set_attribute("style", "width: 300px; height: 400px; overflow-y: auto");
            let pad = scope.create_element("div");
            pad.set_attribute("style", "height: 900px");
            outer.append_child(&pad);
            let inner = scope.create_element("div");
            inner.set_attribute("id", "s");
            inner.set_attribute(
                "style",
                &format!("width: 200px; height: 200px; overflow-y: auto; {inner_style}"),
            );
            let gap = scope.create_element("div");
            gap.set_attribute("style", "height: 1500px");
            inner.append_child(&gap);
            let target = scope.create_element("div");
            target.set_attribute("id", "target");
            target.set_attribute("style", "height: 40px");
            inner.append_child(&target);
            let tail = scope.create_element("div");
            tail.set_attribute("style", "height: 800px");
            inner.append_child(&tail);
            outer.append_child(&inner);
            let tail = scope.create_element("div");
            tail.set_attribute("style", "height: 900px");
            outer.append_child(&tail);
            outer
        });
        t.handle("#s")
            .scroll_to(None, Some(37.0), ScrollBehavior::Instant);
        t.settle();
        t
    };
    let mut instant = mount("");
    instant.handle("#target").scroll_into_view();
    instant.advance_time(FRAME);
    let (outer_end, inner_end) = (
        instant.handle("#outer").scroll_top(),
        instant.handle("#s").scroll_top(),
    );
    assert!(
        outer_end > 0.0 && inner_end > 37.0,
        "positive control: both move"
    );

    let mut smooth = mount("scroll-behavior: smooth");
    smooth.handle("#target").scroll_into_view();
    smooth.advance_time(FRAME);
    assert_eq!(
        smooth.handle("#outer").scroll_top(),
        outer_end,
        "the auto one jumps"
    );
    let first = smooth.handle("#s").scroll_top();
    assert!(first < inner_end, "the smooth one has only begun: {first}");
    for _ in 0..100 {
        smooth.advance_time(FRAME);
    }
    assert_eq!(smooth.handle("#s").scroll_top(), inner_end);
    assert_eq!(smooth.handle("#outer").scroll_top(), outer_end);
}
