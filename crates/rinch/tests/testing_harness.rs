//! The public test harness, `rinch::testing` (#2553), used the way an app's
//! own tests use it: from outside the crate, through its public API only.
//!
//! The first test is the case the issue was filed for. The others are ports
//! of private `RinchApp` tests (`app/click_link_tests.rs`,
//! `app/element_wheel_tests.rs`, `app/element_key_1678_tests.rs`) to the
//! harness, as evidence that it reaches what those reach.
//!
//! The crate's own dev-dependency on itself turns `testing` on, so this runs
//! under a plain `cargo test -p rinch`.

#![cfg(all(feature = "testing", feature = "desktop"))]

use rinch::platform::{AppAction, CursorStyle};
use rinch::prelude::*;
use rinch::testing::TestApp;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// deskrinch's launcher (#2553): result rows with `onmousedown`/`onmouseup`
/// inside a list that has them too. A press on a row runs the row's handlers
/// and not the list's — the nearest node with the attribute takes it — and
/// the list's run for a press on the list itself.
#[test]
fn a_press_on_a_row_runs_the_rows_handlers_not_the_containers() {
    let log = log();
    let l = log.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (a, b) = (l.clone(), l.clone());
        let list = rsx! {
            div {
                id: "list",
                style: "padding: 20px; width: 300px",
                onmousedown: move || a.borrow_mut().push("list down".into()),
                onmouseup: move || b.borrow_mut().push("list up".into()),
            }
        };
        for i in 0..3 {
            let (c, d) = (l.clone(), l.clone());
            let row = rsx! {
                div {
                    class: "row",
                    style: "height: 30px",
                    onmousedown: move || c.borrow_mut().push(format!("row {i} down")),
                    onmouseup: move || d.borrow_mut().push(format!("row {i} up")),
                    {format!("Result {i}")}
                }
            };
            list.append_child(&row);
        }
        list
    });

    let rows = app.query_all(".row");
    assert_eq!(rows.len(), 3);
    app.click_on(&rows[1]);
    assert_eq!(take(&log), ["row 1 down", "row 1 up"]);

    // Pressed on one row and released on another: each half goes to the row
    // under the pointer at the time.
    app.press(&rows[0], MouseButton::Left);
    app.release(&rows[2], MouseButton::Left);
    assert_eq!(take(&log), ["row 0 down", "row 2 up"]);

    // The list's own padding, above the first row.
    let list = app.bounds("#list");
    app.click(list.x + 10.0, list.y + 10.0);
    assert_eq!(take(&log), ["list down", "list up"]);
}

/// The harness reads the document back after the turn the input ran: a
/// signal a click wrote is in the text, an attribute and the layout.
#[test]
fn effects_have_run_when_the_input_returns() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        let count = Signal::new(0);
        rsx! {
            div {
                button {
                    id: "inc",
                    style: "width: 80px; height: 30px",
                    onclick: move || count.update(|n| *n += 1),
                    "+"
                }
                p { id: "out", data-count: {move || count.get().to_string()}, {move || format!("Count: {}", count.get())} }
                div { id: "bar", style: {move || format!("height: 10px; width: {}px", count.get() * 25)} }
            }
        }
    });
    assert_eq!(app.text("#out"), "Count: 0");
    app.click_on("#inc");
    app.click_on("#inc");
    assert_eq!(app.text("#out"), "Count: 2");
    assert_eq!(app.attribute("#out", "data-count").as_deref(), Some("2"));
    assert_eq!(app.bounds("#bar").width, 50.0);
}

/// Positions are logical pixels at any scale factor: a box read with
/// `bounds` is pressed where it says, on a 2x display too (#299).
#[test]
fn a_box_is_pressed_where_bounds_says_at_scale_2() {
    let clicks = Rc::new(Cell::new(0));
    let c = clicks.clone();
    let mut app = TestApp::builder(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "position: relative; width: 600px; height: 400px",
                div {
                    id: "target",
                    style: "position: absolute; left: 400px; top: 200px; width: 100px; height: 40px",
                    onclick: move || c.set(c.get() + 1),
                }
            }
        }
    })
    .size(640, 480)
    .scale_factor(2.0)
    .mount();
    let b = app.bounds("#target");
    assert_eq!((b.x, b.y, b.width, b.height), (400.0, 200.0, 100.0, 40.0));
    app.click_on("#target");
    assert_eq!(clicks.get(), 1);
    // The physical position of the same point is outside the box.
    app.click(b.x * 2.0 + 10.0, b.y * 2.0 + 10.0);
    assert_eq!(clicks.get(), 1, "a physical coordinate is not the box");
}

/// Hovering a `cursor: pointer` element asks the window for that cursor.
#[test]
fn hover_asks_for_the_elements_cursor() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! { div { id: "link", style: "cursor: pointer; width: 100px; height: 20px" } }
    });
    app.take_actions();
    app.hover("#link");
    assert!(
        app.take_actions()
            .contains(&AppAction::SetCursor(CursorStyle::Pointer)),
        "hovering a cursor: pointer box sets the pointer cursor"
    );
}

// ── Ported from app/click_link_tests.rs ─────────────────────────────────

/// `a_container_handler_learns_which_link_was_clicked`: HTML the app did not
/// build itself, under a container with a click handler; the handler learns
/// which link the click went through.
#[test]
fn a_container_handler_learns_which_link_was_clicked() {
    let links: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(Vec::new()));
    let l = links.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let l = l.clone();
        let host = rsx! {
            div {
                id: "host",
                style: "font-size: 16px; line-height: 20px; font-family: sans-serif",
                onclick: move || l.borrow_mut().push(click_link()),
            }
        };
        host.set_inner_html(
            r#"<p><a id="plain" href="https://one.example/" style="display: inline-block; width: 200px">one</a></p>
               <p><span id="text" style="display: inline-block; width: 200px">no link here</span></p>
               <p><a href="mailto:two@example.org"><b><span id="deep" style="display: inline-block; width: 200px">two</span></b></a></p>"#,
        );
        host
    });
    // The private test clicked 4px in from the left edge; the centre of an
    // inline-block 200px wide is inside it just the same.
    app.click_on("#plain");
    app.click_on("#text");
    app.click_on("#deep");
    assert_eq!(
        *links.borrow(),
        vec![
            Some("https://one.example/".to_owned()),
            None,
            Some("mailto:two@example.org".to_owned()),
        ]
    );
    assert_eq!(click_link(), None, "cleared once the handler has run");
}

// ── Ported from app/element_wheel_tests.rs ──────────────────────────────

/// `onwheel_runs_nearest_first_and_an_unconsumed_wheel_still_scrolls`, and
/// `a_consumed_wheel_stops_the_walk_and_scrolls_nothing`: an element's
/// `onwheel` is offered the wheel before its ancestors; one that consumes it
/// stops the walk and the container does not scroll.
#[test]
fn onwheel_runs_nearest_first_and_a_consumed_wheel_scrolls_nothing() {
    let log = log();
    let consume = Rc::new(Cell::new(false));
    let (l, k) = (log.clone(), consume.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (l1, l2) = (l.clone(), l.clone());
        let consume = k.clone();
        rsx! {
            div {
                id: "scroller",
                style: "width: 300px; height: 100px; overflow-y: auto",
                onwheel: move |_w| {
                    l1.borrow_mut().push("scroller".into());
                    false
                },
                div {
                    style: "height: 400px",
                    onwheel: move |w: &WheelEvent| {
                        l2.borrow_mut().push(format!("inner {}", w.delta_y));
                        consume.get()
                    },
                    div { id: "leaf", style: "height: 50px" }
                }
            }
        }
    });
    let scroller = app.handle("#scroller");
    assert_eq!(scroller.scroll_top(), 0.0);

    // DOM sign: positive scrolls the content down. The pointer stays where
    // it is for the second wheel, as a hand on a wheel does; the leaf has
    // scrolled away from under it by then, and `inner` is there instead.
    let (x, y) = app.center("#leaf");
    app.wheel(x, y, 0.0, 40.0);
    assert_eq!(take(&log), ["inner 40", "scroller"]);
    assert!(
        scroller.scroll_top() > 0.0,
        "an unconsumed wheel scrolls the container: {}",
        scroller.scroll_top()
    );

    let before = scroller.scroll_top();
    consume.set(true);
    app.wheel(x, y, 0.0, 40.0);
    assert_eq!(
        take(&log),
        ["inner 40"],
        "the scroller's onwheel is not asked"
    );
    assert_eq!(
        scroller.scroll_top(),
        before,
        "a consumed wheel scrolls nothing"
    );
}

// ── Ported from app/element_key_1678_tests.rs ───────────────────────────

/// `onkeydown_consumes_enter_before_the_field_and_onsubmit`: Enter sends,
/// Shift+Enter is a newline. The element's `onkeydown` takes Enter before the
/// field inserts a break or runs `onsubmit`; an unconsumed Shift+Enter goes on
/// to the ancestors and the field.
#[test]
fn onkeydown_consumes_enter_before_the_field_and_onsubmit() {
    let log = log();
    let l = log.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (l1, l2, l3, l4) = (l.clone(), l.clone(), l.clone(), l.clone());
        rsx! {
            div {
                style: "padding: 10px",
                onkeydown: move |k| {
                    l1.borrow_mut().push(format!("outer:{}", k.key));
                    false
                },
                textarea {
                    id: "composer",
                    style: "width: 300px; height: 80px; font-size: 16px; line-height: 20px; padding: 4px 6px; border: 0",
                    oninput: move |v: String| l2.borrow_mut().push(format!("input:{v}")),
                    onsubmit: move || l3.borrow_mut().push("submit".into()),
                    onkeydown: move |k: &KeyEventData| {
                        l4.borrow_mut().push(format!("down:{}{}", if k.shift { "S-" } else { "" }, k.key));
                        k.key == "Enter" && !k.shift
                    },
                }
            }
        }
    });
    app.click_on("#composer");
    assert_eq!(
        app.focused().map(|n| n.node_id()),
        Some(app.handle("#composer").node_id()),
        "a click focuses the field"
    );
    app.type_text("hi");
    assert_eq!(app.value("#composer").as_deref(), Some("hi"));
    take(&log);

    app.press_key("Enter");
    assert_eq!(
        app.value("#composer").as_deref(),
        Some("hi"),
        "a consumed Enter inserts nothing"
    );
    let seen = take(&log);
    assert_eq!(seen[0], "down:Enter", "{seen:?}");
    assert!(
        !seen
            .iter()
            .any(|e| e == "submit" || e.starts_with("outer:") || e.starts_with("input:")),
        "consumed: no submit, no ancestor, no edit: {seen:?}"
    );

    app.set_modifiers(ModifierState {
        shift: true,
        ..ModifierState::default()
    });
    app.press_key("Enter");
    app.set_modifiers(ModifierState::default());
    let seen = take(&log);
    assert_eq!(
        seen[..2],
        ["down:S-Enter", "outer:Enter"],
        "nearest first: {seen:?}"
    );
    assert_eq!(
        app.value("#composer").as_deref(),
        Some("hi\n"),
        "Shift+Enter is the field's newline"
    );
}

/// Review of #2582: a named key types and spells what a real keyboard's
/// does. Space types a space (its `key` is `" "`), Shift capitalises a
/// letter, and a named key's `key` is its W3C name.
#[test]
fn space_types_a_space_and_shift_capitalises_a_letter() {
    let keys = log();
    let k = keys.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let k = k.clone();
        rsx! {
            input {
                id: "field",
                style: "width: 200px; height: 30px",
                oninput: move |_v: String| {},
                onkeydown: move |e: &KeyEventData| {
                    k.borrow_mut().push(e.key.clone());
                    false
                },
            }
        }
    });
    app.click_on("#field");
    app.press_key("a");
    app.press_key("Space");
    app.press_key("b");
    app.set_modifiers(ModifierState {
        shift: true,
        ..ModifierState::default()
    });
    app.press_key("c");
    app.set_modifiers(ModifierState::default());
    assert_eq!(app.value("#field").as_deref(), Some("a bC"));
    app.press_key("F5");
    app.press_key("ArrowLeft");
    assert_eq!(take(&keys), ["a", " ", "b", "C", "F5", "ArrowLeft"]);
}

/// The turn after an input paints when the input asked for a frame, and the
/// paint drains the polls (`ShellWindow::paint_software`): a poll that reads
/// a value changed outside any signal is current when the click returns.
#[test]
fn the_turn_after_an_input_paints_and_drains_the_polls() {
    use rinch::core::reactive::{PollRate, poll_signal};
    let source = Rc::new(Cell::new(0));
    let s = source.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let s = s.clone();
        let polled = poll_signal(move || s.get(), PollRate::EveryFrame);
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
    assert_eq!(app.text("#out"), "0 0");
    source.set(7);
    app.click_on("#b");
    assert_eq!(app.text("#out"), "7 1");
}
