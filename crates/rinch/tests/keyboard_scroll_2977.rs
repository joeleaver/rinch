//! #2977: keyboard scrolling on desktop. With nothing editable holding the
//! key, the arrows, Page Up / Down, Home / End and Space scroll the focused
//! scroller (else the one last pressed in, else the page), as Chrome's
//! default action does, on Chrome's keyboard curve: 150 ms whatever the
//! distance, `ease-in-out` (Chrome 158, measured). Driven through
//! `rinch::testing`, the virtual clock stepped 10 ms a turn; every scroller
//! starts part-way down.

#![cfg(all(feature = "testing", feature = "windowing"))]

use rinch::prelude::*;
use rinch::testing::TestApp;
use std::time::Duration;

const START: f64 = 333.0;
const FRAME: Duration = Duration::from_millis(10);

/// A focusable 200x300 scroller `#s` holding 3000 px, inside a 5000 px page,
/// with a text field `#field` above it.
fn mount() -> TestApp {
    let mut t = TestApp::mount(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element("input");
        field.set_attribute("id", "field");
        root.append_child(&field);
        let s = scope.create_element("div");
        s.set_attribute("id", "s");
        s.set_attribute("tabindex", "0");
        s.set_attribute("style", "width: 200px; height: 300px; overflow-y: auto");
        let inner = scope.create_element("div");
        inner.set_attribute("style", "height: 3000px");
        s.append_child(&inner);
        root.append_child(&s);
        let tail = scope.create_element("div");
        tail.set_attribute("style", "height: 5000px");
        root.append_child(&tail);
        root
    });
    t.handle("#s")
        .scroll_to(None, Some(START), ScrollBehavior::Instant);
    t.settle();
    t
}

fn top(t: &TestApp) -> f64 {
    t.handle("#s").scroll_top()
}

/// CSS `ease-in-out` at `x`, solved independently.
fn ease_in_out(x: f64) -> f64 {
    if x >= 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..80 {
        let t = (lo + hi) / 2.0;
        let u = 1.0 - t;
        if 3.0 * u * u * t * 0.42 + 3.0 * u * t * t * 0.58 + t * t * t < x {
            lo = t;
        } else {
            hi = t;
        }
    }
    let t = (lo + hi) / 2.0;
    let u = 1.0 - t;
    3.0 * u * t * t + t * t * t
}

/// Turns until nothing animates (at most `max`): `scrollTop` after each.
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

/// ArrowDown on the focused scroller: 40 px, on `ease-in-out` over 150 ms,
/// then idle. Mutants it kills: no keyboard scroll, a programmatic or wheel
/// duration (40 px is 105 / 200 ms there), a linear ease.
#[test]
fn an_arrow_scrolls_forty_pixels_on_chromes_keyboard_curve() {
    let mut t = mount();
    t.focus("#s");
    t.press_key("ArrowDown");
    let samples = frames(&mut t, 40);
    for (i, got) in samples.iter().enumerate() {
        let ms = 10.0 * (i + 1) as f64;
        let want = START + 40.0 * ease_in_out(ms / 150.0);
        assert!(
            (got - want).abs() <= 1.0,
            "at {ms} ms: {got}, the curve is at {want:.2} ({samples:?})"
        );
    }
    assert_eq!(samples.len(), 15, "150 ms: {samples:?}");
    assert_eq!(top(&t), START + 40.0);
    t.press_key("ArrowUp");
    frames(&mut t, 40);
    assert_eq!(top(&t), START);
}

/// Page Down and Space move 0.875 of the scrollport (262 px of 300, Chrome's
/// figure); Page Up and Shift+Space move back.
#[test]
fn a_page_is_seven_eighths_of_the_scrollport() {
    let mut t = mount();
    t.focus("#s");
    for (key, shift, want) in [
        ("PageDown", false, START + 262.0),
        ("PageUp", false, START),
        (" ", false, START + 262.0),
        (" ", true, START),
    ] {
        let mut mods = t.modifiers();
        mods.shift = shift;
        t.set_modifiers(mods);
        t.press_key(key);
        let samples = frames(&mut t, 40);
        assert!(samples.len() >= 14, "{key:?} animated: {samples:?}");
        assert!(
            (top(&t) - want).abs() < 1.0,
            "{key:?} shift={shift}: {}",
            top(&t)
        );
    }
}

/// End goes to the end and Home to the start, in 150 ms however far.
#[test]
fn home_and_end_go_to_the_edges() {
    let mut t = mount();
    t.focus("#s");
    let end = t.handle("#s").scroll_height() - t.handle("#s").client_height();
    t.press_key("End");
    let samples = frames(&mut t, 40);
    assert_eq!(samples.len(), 15, "{samples:?}");
    assert_eq!(top(&t), end);
    t.press_key("Home");
    frames(&mut t, 40);
    assert_eq!(top(&t), 0.0);
}

/// A key pressed again mid-scroll moves on from where the scroll is headed.
#[test]
fn a_repeated_key_adds_to_the_target() {
    let mut t = mount();
    t.focus("#s");
    t.press_key("ArrowDown");
    frames(&mut t, 5);
    t.press_key("ArrowDown");
    frames(&mut t, 40);
    assert_eq!(top(&t), START + 80.0);
}

/// With nothing focused the scroller last pressed in scrolls; with no press
/// either, the page does.
#[test]
fn without_focus_the_pressed_scroller_then_the_page_scrolls() {
    let mut t = mount();
    // Not focusable: the press focuses nothing.
    t.handle("#s").remove_attribute("tabindex");
    t.settle();
    let (x, y) = t.center("#s");
    t.click(x, y);
    t.press_key("ArrowDown");
    frames(&mut t, 40);
    assert_eq!(top(&t), START + 40.0, "the pressed scroller");

    let mut t = mount();
    let body = t.query("body").unwrap();
    assert_eq!(body.scroll_top(), 0.0);
    t.press_key("PageDown");
    frames(&mut t, 40);
    assert!(body.scroll_top() > 300.0, "the page: {}", body.scroll_top());
    assert_eq!(top(&t), START, "the scroller is not the page");
}

/// A scroller at its end hands the key to the one around it (the page).
#[test]
fn a_scroller_at_its_end_hands_the_key_on() {
    let mut t = mount();
    t.focus("#s");
    t.press_key("End");
    frames(&mut t, 40);
    let body = t.query("body").unwrap();
    assert_eq!(body.scroll_top(), 0.0);
    t.press_key("ArrowDown");
    frames(&mut t, 40);
    assert_eq!(body.scroll_top(), 40.0);
}

/// A focused text field keeps its keys: nothing scrolls.
#[test]
fn a_focused_text_field_keeps_the_keys() {
    let mut t = mount();
    t.focus("#field");
    for key in ["ArrowDown", "End", " "] {
        t.press_key(key);
        frames(&mut t, 20);
    }
    assert_eq!(t.query("body").unwrap().scroll_top(), 0.0);
    assert_eq!(top(&t), START);
}

/// With smooth scrolling off (the wheel's switch, Chrome's one setting for
/// user scrolls), a key scrolls its whole distance at once.
#[test]
fn the_smooth_switch_makes_it_instant() {
    let mut t = mount();
    t.app().set_smooth_wheel_scroll(false);
    t.focus("#s");
    t.press_key("ArrowDown");
    assert_eq!(top(&t), START + 40.0);
    assert!(!t.app().wheel_scroll_animating());
}

/// Turns until nothing animates, at most `max`.
fn run(t: &mut TestApp, max: usize) {
    for _ in 0..max {
        t.advance_time(FRAME);
        if !t.app().wheel_scroll_animating() {
            break;
        }
    }
}

/// A 5000 px page with a fixed header `#hdr`, a link `#link` and a button
/// `#button` (with a click handler that counts).
fn page_with_header(clicks: std::rc::Rc<std::cell::Cell<u32>>) -> TestApp {
    let mut t = TestApp::mount(move |__scope: &mut RenderScope| {
        let c = clicks.clone();
        rsx! {
            div {
                div { id: "hdr", style: "position: fixed; top: 0px; left: 0px; width: 400px; height: 40px", "header" }
                div { style: "padding-top: 60px",
                    a { id: "link", href: "#x", "link" }
                    button { id: "button", onclick: move || c.set(c.get() + 1), "button" }
                }
                div { style: "height: 5000px" }
            }
        }
    });
    t.settle();
    t
}

/// A press in a fixed header, then Page Down: the page scrolls, as in Chrome
/// (the walk used to stop at the fixed box and scroll nothing). Review of
/// #2987.
#[test]
fn a_fixed_box_hands_the_key_to_the_page() {
    let mut t = page_with_header(Default::default());
    let body = t.query("body").unwrap();
    let (x, y) = t.center("#hdr");
    t.click(x, y);
    t.press_key("PageDown");
    run(&mut t, 40);
    let paged = body.scroll_top();
    assert!(paged > 300.0, "the page: {paged}");
    t.press_key("ArrowUp");
    run(&mut t, 40);
    assert_eq!(body.scroll_top(), paged - 40.0, "an arrow too");
}

/// Space on a focused link scrolls the page and follows nothing, as in Chrome
/// 158; on a focused button it activates the button and scrolls nothing.
#[test]
fn space_scrolls_from_a_link_and_activates_a_button() {
    let clicks: std::rc::Rc<std::cell::Cell<u32>> = Default::default();
    let mut t = page_with_header(clicks.clone());
    let body = t.query("body").unwrap();
    t.focus("#link");
    t.press_key(" ");
    run(&mut t, 40);
    assert!(
        body.scroll_top() > 300.0,
        "Space on a link: {}",
        body.scroll_top()
    );

    let mut t = page_with_header(clicks.clone());
    let body = t.query("body").unwrap();
    t.focus("#button");
    t.press_key(" ");
    run(&mut t, 40);
    assert_eq!(body.scroll_top(), 0.0, "Space on a button scrolls nothing");
    assert_eq!(clicks.get(), 1, "Space on a button activates it");
}
