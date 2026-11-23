//! UI timing on rinch's clock (#2842): the behaviours that judge elapsed time
//! read `rinch::time::now()`, so under `TestApp` the virtual clock decides
//! them and a test can stand on a boundary with no sleep. The tests put the
//! clock exactly at, or one millisecond short of, a boundary, or at a fixed
//! fraction of a duration; on the wall clock each of them would see almost no
//! time pass.
//!
//! The editor's caret blink and undo-merge window, the `Select` component's
//! type-ahead and the graph canvas's wheel gesture are pinned in their own
//! crates (`rinch-editor-view`, `rinch-components`, `rinch-graph-view`).

#![cfg(all(feature = "testing", feature = "windowing"))]

use rinch::prelude::*;
use rinch::testing::TestApp;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Two clicks, `gap` apart on the clock, on a link in selectable text; answers
/// how many times the link was followed. A single click on such a link
/// follows it on the release; a double click selects the word and follows
/// nothing.
fn link_follows_after_two_clicks(gap: Duration) -> usize {
    let follows = Rc::new(Cell::new(0));
    let f = follows.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let f = f.clone();
        let host = rsx! {
            div {
                style: "padding: 20px; user-select: text; font-size: 16px; line-height: 20px; font-family: sans-serif",
                onclick: move || {
                    if click_link().is_some() {
                        f.set(f.get() + 1);
                    }
                },
            }
        };
        host.set_inner_html(r#"<p id="para"><a href="https://one.example/">hello world</a></p>"#);
        host
    });
    let b = app.bounds("#para");
    let (x, y) = (b.x + 14.0, b.y + b.height / 2.0);
    app.click(x, y);
    assert_eq!(
        follows.get(),
        1,
        "precondition: a single click follows the link"
    );
    app.advance_time(gap);
    app.click(x, y);
    follows.get()
}

/// A second click 499 ms after the first is a double click (it selects and
/// follows nothing); 500 ms after it, a single click (it follows the link
/// again). The window is the desktop's 500 ms.
#[test]
fn a_second_click_is_a_double_click_up_to_499ms_2842() {
    assert_eq!(
        link_follows_after_two_clicks(ms(499)),
        1,
        "499 ms: a double click"
    );
    assert_eq!(
        link_follows_after_two_clicks(ms(500)),
        2,
        "500 ms: a single click"
    );
}

/// A pointer resting on a `title` shows the tooltip at its 500 ms delay,
/// not before.
#[test]
fn the_title_tooltip_shows_at_its_delay_2842() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "padding: 20px",
                div { id: "tip", title: "Saved", style: "width: 100px; height: 40px" }
            }
        }
    });
    app.hover("#tip");
    app.advance_time(ms(499));
    assert_eq!(app.app().title_tooltip_text(), None, "499 ms: not yet");
    app.advance_time(ms(1));
    assert_eq!(
        app.app().title_tooltip_text(),
        Some("Saved"),
        "500 ms: shown"
    );
}

/// A linear 1000 ms width transition from 100px to 300px is at 200px when
/// 500 ms have been advanced, and done at 1000.
#[test]
fn a_transition_is_at_its_midpoint_halfway_through_2842() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        let wide = Signal::new(false);
        rsx! {
            div {
                button {
                    id: "go",
                    style: "width: 40px; height: 20px",
                    onclick: move || wide.set(true),
                }
                div {
                    id: "box",
                    style: {move || format!(
                        "height: 20px; transition: width 1000ms linear; width: {}px",
                        if wide.get() { 300 } else { 100 }
                    )},
                }
            }
        }
    });
    assert_eq!(app.bounds("#box").width, 100.0);
    app.click_on("#go");
    assert_eq!(
        app.bounds("#box").width,
        100.0,
        "the transition starts here"
    );
    app.advance_time(ms(500));
    assert_eq!(app.bounds("#box").width, 200.0, "the midpoint at 500 ms");
    app.advance_time(ms(500));
    assert_eq!(app.bounds("#box").width, 300.0, "done at 1000 ms");
}

/// A drag resting in a scroller's bottom band scrolls by its speed times the
/// time since the last frame: nothing for no time, 1.5 first steps (1/60 s
/// each) for 25 ms, three for 50 ms.
#[test]
fn drag_auto_scroll_moves_with_the_time_between_frames_2842() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        let list = rsx! {
            div {
                id: "list",
                style: "position: absolute; left: 0px; top: 100px; width: 300px; \
                        height: 200px; overflow-y: auto",
            }
        };
        for i in 0..20 {
            let row = rsx! {
                div { id: {format!("row{i}")}, style: "height: 40px", ondrop: || {} }
            };
            list.append_child(&row);
        }
        rsx! {
            div {
                style: "position: relative; width: 800px; height: 600px",
                div {
                    id: "src",
                    draggable: "true",
                    ondragend: || {},
                    style: "position: absolute; left: 500px; top: 40px; width: 100px; height: 40px",
                }
                {list}
            }
        }
    });
    let scrolled = |app: &TestApp| 100.0 - app.bounds("#row0").y;
    app.mouse_down(520.0, 55.0, MouseButton::Left);
    app.mouse_move(540.0, 70.0);
    // Into the band: the first frame there scrolls its fixed first step,
    // 1/60 s at the band's speed.
    app.mouse_move(100.0, 292.0);
    let first = scrolled(&app);
    assert!(first > 0.0, "the drag is live and the band scrolls");
    // A turn with no time passed scrolls nothing.
    app.advance_time(ms(0));
    assert_eq!(scrolled(&app), first, "no time, no scroll");
    // 25 ms is 1.5 first steps, 50 ms three (offsets land on whole pixels).
    app.advance_time(ms(25));
    let d25 = scrolled(&app) - first;
    app.advance_time(ms(50));
    let d50 = scrolled(&app) - first - d25;
    assert!(
        (d25 - 1.5 * first).abs() <= 1.5,
        "25 ms scrolled {d25}px, the first step {first}px"
    );
    assert!(
        (d50 - 3.0 * first).abs() <= 1.5,
        "50 ms scrolled {d50}px, the first step {first}px"
    );
    app.mouse_up(100.0, 292.0, MouseButton::Left);
}

/// A native `<select>`'s type-ahead prefix lives 900 ms after the last
/// letter: "b", then "l" at 900 ms is "bl" (Blueberry); "c" 901 ms after
/// that starts a new prefix (Cherry).
#[test]
fn a_select_type_ahead_prefix_expires_after_900ms_2842() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "padding: 20px",
                select {
                    id: "s",
                    style: "width: 160px",
                    option { value: "apple", "Apple" }
                    option { value: "banana", "Banana" }
                    option { value: "cherry", "Cherry" }
                    option { value: "blueberry", "Blueberry" }
                }
            }
        }
    });
    let pick = |app: &mut TestApp, keys: &[(u64, &str)]| {
        app.click_on("#s");
        for &(gap, key) in keys {
            app.advance_time(ms(gap));
            app.press_key(key);
        }
        app.press_key("Enter");
        app.value("#s").unwrap()
    };
    assert_eq!(pick(&mut app, &[(0, "b"), (900, "l")]), "blueberry");
    assert_eq!(pick(&mut app, &[(0, "b"), (901, "c")]), "cherry");
}

/// A `poll_signal` at `Millis(100)` samples its source when 100 ms have
/// passed since its last sample, and not at 99.
#[test]
fn a_timed_poll_comes_due_on_the_clock_2842() {
    use rinch::core::reactive::{PollRate, poll_signal};
    let source = Rc::new(Cell::new(0));
    let s = source.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let s = s.clone();
        let polled = poll_signal(move || s.get(), PollRate::Millis(100));
        let clicks = Signal::new(0);
        rsx! {
            div {
                button {
                    id: "b",
                    style: "width: 40px; height: 20px",
                    onclick: move || clicks.update(|n| *n += 1),
                }
                p { id: "out", {move || format!("{} {}", polled.get(), clicks.get())} }
            }
        }
    });
    // The mount's frame sampled it at the clock's zero.
    assert_eq!(app.text("#out"), "0 0");
    source.set(7);
    app.advance_time(ms(99));
    app.click_on("#b"); // a painted frame drains the polls
    assert_eq!(app.text("#out"), "0 1", "99 ms: not due");
    app.advance_time(ms(1));
    app.click_on("#b");
    assert_eq!(app.text("#out"), "7 2", "100 ms: sampled");
}
