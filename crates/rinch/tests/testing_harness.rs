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

#![cfg(all(feature = "testing", feature = "windowing"))]

use rinch::platform::{AppAction, CursorStyle};
use rinch::prelude::*;
use rinch::testing::{TestApp, WindowRequest};
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

/// A key goes to the menu chords first, as from a real keyboard, and the
/// shell's rules come with it: no chord runs under an open runtime popup
/// (#822), and the chord runs again once the popup closed.
#[test]
fn a_menu_chord_runs_from_press_key_but_not_under_an_open_select() {
    use rinch::menu::{Menu, MenuItem, register_menu_shortcuts};
    let fired = Rc::new(Cell::new(0u32));
    let f = fired.clone();
    let menu = Menu::new().item(
        MenuItem::new("New")
            .shortcut("Ctrl+N")
            .on_click(move || f.set(f.get() + 1)),
    );
    let _chords = register_menu_shortcuts(&[("File", &menu)]);
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                select {
                    id: "pick",
                    style: "width: 200px; height: 30px",
                    option { value: "one", "one" }
                    option { value: "two", "two" }
                }
            }
        }
    });
    let ctrl = ModifierState {
        ctrl: true,
        ..ModifierState::default()
    };
    app.set_modifiers(ctrl);
    app.press_key("n");
    assert_eq!(fired.get(), 1, "positive control: the chord runs");

    app.set_modifiers(ModifierState::default());
    app.click_on("#pick");
    app.set_modifiers(ctrl);
    app.press_key("n");
    assert_eq!(fired.get(), 1, "no chord under the open <select> popup");

    app.set_modifiers(ModifierState::default());
    app.press_key("Escape");
    app.set_modifiers(ctrl);
    app.press_key("n");
    assert_eq!(fired.get(), 2, "the popup closed: the chord runs again");
}

// ── #2742: the window's props, its requests, its state, and time ─────────

/// #2742 part 4: the window's state is published as the desktop shell
/// publishes it, so a component reading `rinch::window` follows the focus and
/// the size the test gives the window.
#[test]
fn window_focused_and_window_size_follow_the_harness_2742() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| {
        rsx! {
            div {
                p { id: "focused", {|| rinch::window::window_focused().to_string()} }
                p { id: "size", {|| format!("{:?}", rinch::window::window_size())} }
            }
        }
    })
    .size(640, 360)
    .mount();
    assert_eq!(app.text("#focused"), "true", "a mounted window has focus");
    assert_eq!(app.text("#size"), "(640.0, 360.0)");

    app.window_focus(false);
    assert_eq!(app.text("#focused"), "false");
    app.window_focus(true);
    assert_eq!(app.text("#focused"), "true");

    app.resize(500, 410);
    assert_eq!(app.text("#size"), "(500.0, 410.0)");
}

/// #2742 part 4: the scale factor too, from the builder and from a move to
/// another display.
#[test]
fn scale_factor_follows_the_builder_and_set_scale_factor_2742() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| {
        rsx! { p { id: "scale", {|| rinch::window::scale_factor().to_string()} } }
    })
    .scale_factor(1.5)
    .mount();
    assert_eq!(app.text("#scale"), "1.5");
    app.set_scale_factor(2.0);
    assert_eq!(app.text("#scale"), "2");
    assert_eq!(app.scale_factor(), 2.0);
}

/// deskrinch's launcher (#2742 parts 2 and 4): an Effect on
/// `window_focused()` hides the window when it loses the keyboard. The hide
/// is recorded, and applied: the window is no longer shown.
#[test]
fn a_launcher_hides_its_window_when_it_loses_the_keyboard_2742() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        // On losing the keyboard, not before the window first has it (a
        // window is mounted before it is focused, here as on the desktop).
        let had_it = Cell::new(false);
        rinch::Effect::new(move || {
            let focused = rinch::window::window_focused();
            if had_it.replace(focused) && !focused {
                rinch::windows::hide_current_window();
            }
        });
        rsx! { input { id: "query" } }
    });
    assert!(app.is_visible());
    assert!(app.is_window_focused());
    assert_eq!(app.take_window_requests(), [], "nothing asked at the mount");

    app.window_focus(false);
    assert_eq!(app.take_window_requests(), [WindowRequest::Hide]);
    assert!(!app.is_visible());

    // Typing while hidden asks for nothing more.
    app.focus("#query");
    app.type_text("x");
    assert_eq!(app.take_window_requests(), []);
}

/// #2742 parts 1-3: a window the props start hidden is mounted without being
/// shown or focused, prepares itself, and is shown by its own request (here
/// from a timer): shown, it is painted and takes the keyboard.
#[test]
fn a_window_started_hidden_is_shown_by_its_request_2742() {
    let focus_seen = Rc::new(RefCell::new(Vec::<bool>::new()));
    let seen = focus_seen.clone();
    let mut app = TestApp::builder(move |__scope: &mut RenderScope| {
        rinch::windows::prepare_current_window();
        set_timeout(250, rinch::windows::show_current_window);
        rinch::Effect::new(move || seen.borrow_mut().push(rinch::window::window_focused()));
        rsx! { p { id: "label", "Launcher" } }
    })
    .window_props(WindowProps {
        visible: false,
        width: 420,
        height: 260,
        ..WindowProps::default()
    })
    .mount();
    assert!(!app.is_visible());
    assert!(!app.is_window_focused());
    assert_eq!(app.size(), (420, 260), "the props' size");
    assert_eq!(app.take_window_requests(), [WindowRequest::Prepare(None)]);
    assert!(app.bounds("#label").width > 0.0, "the prepare laid it out");

    app.advance_time(std::time::Duration::from_millis(249));
    assert!(!app.is_visible(), "not yet");
    assert_eq!(app.take_window_requests(), []);

    app.advance_time(std::time::Duration::from_millis(1));
    assert_eq!(app.take_window_requests(), [WindowRequest::Show]);
    assert!(app.is_visible());
    assert!(app.is_window_focused());
    assert_eq!(*focus_seen.borrow(), [false, true]);

    // Hidden again by the test's own request: the keyboard goes with it.
    rinch::windows::hide_current_window();
    app.settle();
    assert_eq!(app.take_window_requests(), [WindowRequest::Hide]);
    assert!(!app.is_visible());
    assert_eq!(*focus_seen.borrow(), [false, true, false]);
}

/// #2742 part 1: a non-activating window is shown without the keyboard,
/// at the mount and on a show.
#[test]
fn a_non_activating_window_is_shown_without_the_keyboard_2742() {
    let props = |visible| WindowProps {
        visible,
        non_activating: true,
        ..WindowProps::default()
    };
    let shown = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(props(true))
        .mount();
    assert!(shown.is_visible());
    assert!(!shown.is_window_focused());
    drop(shown);

    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(props(false))
        .mount();
    rinch::windows::show_current_window();
    app.settle();
    assert!(app.is_visible());
    assert!(!app.is_window_focused());
}

/// `focus_current_window` (#2310) shows a hidden window and gives it the
/// keyboard, gives a shown one the keyboard, and never gives it to a
/// non-activating one.
#[test]
fn focus_current_window_shows_a_hidden_window_and_focuses_it_2310() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(WindowProps {
            visible: false,
            ..WindowProps::default()
        })
        .mount();
    assert!(!app.is_visible());
    rinch::windows::focus_current_window();
    app.settle();
    assert_eq!(app.take_window_requests(), [WindowRequest::Focus]);
    assert!(app.is_visible(), "a hidden window is shown");
    assert!(app.is_window_focused());

    // Shown and focused elsewhere: the request takes the keyboard back.
    app.window_focus(false);
    assert!(!app.is_window_focused());
    rinch::windows::focus_current_window();
    app.settle();
    assert!(app.is_window_focused());
    drop(app);

    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(WindowProps {
            visible: false,
            non_activating: true,
            ..WindowProps::default()
        })
        .mount();
    rinch::windows::focus_current_window();
    app.settle();
    assert!(app.is_visible(), "a hidden non-activating window is shown");
    assert!(!app.is_window_focused(), "but never takes the keyboard");
}

/// `activate_current_window_with_token` (#2817) is a focus request that
/// carries the token to the shell, and is applied as one: a hidden window is
/// shown and focused, a shown one takes the keyboard back, a non-activating
/// one never does. `WindowHandle::activate_with_token` names its window.
#[test]
fn activate_with_token_is_a_focus_request_with_the_token_2817() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(WindowProps {
            visible: false,
            ..WindowProps::default()
        })
        .mount();
    rinch::windows::activate_current_window_with_token("sway-launch-17");
    app.settle();
    assert_eq!(
        app.take_window_requests(),
        [WindowRequest::ActivateWithToken("sway-launch-17".into())]
    );
    assert!(app.is_visible(), "a hidden window is shown");
    assert!(app.is_window_focused());

    app.window_focus(false);
    rinch::windows::activate_current_window_with_token("kwin-9");
    app.settle();
    assert!(
        app.is_window_focused(),
        "a shown window takes the keyboard back"
    );
    // Without a token it is the plain focus request it always was.
    rinch::windows::focus_current_window();
    app.settle();
    assert_eq!(
        app.take_window_requests(),
        [
            WindowRequest::ActivateWithToken("kwin-9".into()),
            WindowRequest::Focus
        ]
    );
    drop(app);

    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(WindowProps {
            non_activating: true,
            ..WindowProps::default()
        })
        .mount();
    app.window_focus(false);
    rinch::windows::activate_current_window_with_token("t");
    app.settle();
    assert!(
        !app.is_window_focused(),
        "a non-activating window never takes the keyboard"
    );
}

/// #2742 part 1: a window sized by its content takes the content's size, and
/// follows it when the content grows.
#[test]
fn a_content_sized_window_takes_and_follows_its_contents_size_2742() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| {
        let tall = Signal::new(false);
        rsx! {
            div {
                id: "box",
                style: {move || format!(
                    "width: 237px; height: {}px",
                    if tall.get() { 153 } else { 91 }
                )},
                onclick: move || tall.set(true),
                span { id: "size", {|| format!("{:?}", rinch::window::window_size())} }
            }
        }
    })
    .window_props(WindowProps {
        size_to_content: Some(rinch::ContentSizing::new()),
        ..WindowProps::default()
    })
    .mount();
    assert_eq!(app.size(), (237, 91));
    assert_eq!(app.text("#size"), "(237.0, 91.0)");

    app.click_on("#box");
    assert_eq!(app.size(), (237, 153));
    assert_eq!(app.text("#size"), "(237.0, 153.0)");
}

/// #2742 part 3: timers run on a virtual clock the test advances, firing in
/// deadline order (timers due together in the order they were armed), each
/// followed by a turn, and only when the clock reaches them.
#[test]
fn timers_fire_in_deadline_order_on_the_virtual_clock_2742() {
    use std::time::Duration;
    let log = log();
    let l = log.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let ticks = Signal::new(0);
        let push = |l: &Log, s: &'static str| {
            let l = l.clone();
            move || l.borrow_mut().push(s.to_string())
        };
        set_timeout(300, push(&l, "c@300"));
        set_timeout(100, push(&l, "a@100"));
        let cleared = set_timeout(120, push(&l, "cleared"));
        clear_timeout(cleared);
        let il = l.clone();
        set_interval(150, move || {
            ticks.update(|t| *t += 1);
            il.borrow_mut().push(format!("tick{}", ticks.get()));
        });
        let bl = l.clone();
        set_timeout(100, move || {
            bl.borrow_mut().push("b@100".into());
            // Armed at 100, due at 160: inside the advance below, and after
            // the tick at 150 only if the clock stood at 100 when it was armed.
            set_timeout(60, push(&bl, "d@160"));
        });
        let zl = l.clone();
        rsx! {
            div {
                button {
                    id: "zero",
                    onclick: move || { set_timeout(0, push(&zl, "zero")); },
                    "0"
                }
                p { id: "ticks", {move || ticks.get().to_string()} }
            }
        }
    });

    app.advance_time(Duration::from_millis(50));
    assert_eq!(take(&log), Vec::<String>::new());
    assert_eq!(app.text("#ticks"), "0");

    app.advance_time(Duration::from_millis(260));
    assert_eq!(
        take(&log),
        ["a@100", "b@100", "tick1", "d@160", "c@300", "tick2"]
    );
    assert_eq!(app.text("#ticks"), "2", "the DOM followed the ticks");
    assert_eq!(app.elapsed(), Duration::from_millis(310));

    // A timeout of 0 fires in the turn after the input that armed it, with
    // the clock where it is.
    app.click_on("#zero");
    assert_eq!(take(&log), ["zero"]);
    assert_eq!(app.elapsed(), Duration::from_millis(310));

    app.advance_time(Duration::from_millis(140));
    assert_eq!(take(&log), ["tick3"]);
}

/// #2742 review: a request made through the handle of a window the app
/// opened is that window's, and never applied to the test's window
/// (deskrinch's launcher with a popup). Since #2793 the window is open in
/// the harness, so its requests are its own (`TestWindow::take_requests`),
/// not `OtherWindow` in the app's list; once it is closed, a request
/// through its handle is `OtherWindow` again.
#[test]
fn another_windows_requests_are_not_the_test_windows_2742() {
    let handle: Rc<RefCell<Option<rinch::WindowHandle>>> = Rc::new(RefCell::new(None));
    let h = handle.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let w = rinch::open_window(WindowProps::default(), |__scope: &mut RenderScope| {
            rsx! { div { "popup" } }
        });
        *h.borrow_mut() = Some(w);
        rsx! { div { "main" } }
    });
    let w = handle.borrow().unwrap();
    w.hide();
    w.set_title("Popup");
    app.settle();
    assert_eq!(
        app.window(w).take_requests(),
        [WindowRequest::Hide, WindowRequest::SetTitle("Popup".into())]
    );
    assert!(app.take_window_requests().is_empty());
    assert!(app.is_visible(), "the popup's hide hid the test's window");
    assert!(app.is_window_focused(), "and took its keyboard");
    w.close();
    app.settle();
    w.set_title("Gone");
    app.settle();
    assert_eq!(
        app.take_window_requests(),
        [WindowRequest::OtherWindow {
            window: w.id(),
            request: Box::new(WindowRequest::SetTitle("Gone".into())),
        }]
    );
}

/// #2742 review: two apps on one thread. A request goes to the app whose
/// document sent it, not to the newest; the two share the thread's clock.
#[test]
fn two_apps_on_one_thread_each_get_their_own_requests_2742() {
    use std::time::Duration;
    let mut a = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! { button { id: "b", onclick: rinch::windows::hide_current_window, "x" } }
    });
    a.advance_time(Duration::from_millis(40));
    let mut b = TestApp::mount(|__scope: &mut RenderScope| rsx! { div {} });
    a.click_on("#b");
    b.settle();
    assert_eq!(a.take_window_requests(), [WindowRequest::Hide]);
    assert_eq!(b.take_window_requests(), []);
    assert!(!a.is_visible());
    assert!(b.is_visible());
    assert_eq!(b.elapsed(), Duration::from_millis(40), "one clock");
}

/// #2742 review: a timer cancelling a later one and arming another for its
/// own instant, during one advance; an interval clearing itself; and the
/// timers due at one instant fire in arming order.
#[test]
fn timers_cancel_and_arm_during_an_advance_2742() {
    use std::time::Duration;
    let log = log();
    let l = log.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let push = |l: &Log, s: &'static str| {
            let l = l.clone();
            move || l.borrow_mut().push(s.to_string())
        };
        let victim = set_timeout(200, push(&l, "victim"));
        let l2 = l.clone();
        set_timeout(100, move || {
            l2.borrow_mut().push("canceller".into());
            clear_timeout(victim);
            // Due at 100 too: after the other timers due at 100.
            set_timeout(0, push(&l2, "child@100"));
        });
        set_timeout(100, push(&l, "sibling@100"));
        let l3 = l.clone();
        let count = Rc::new(Cell::new(0));
        let me: Rc<Cell<Option<IntervalHandle>>> = Rc::new(Cell::new(None));
        let m = me.clone();
        me.set(Some(set_interval(70, move || {
            count.set(count.get() + 1);
            l3.borrow_mut().push(format!("iv{}", count.get()));
            if count.get() == 3 {
                clear_interval(m.get().unwrap());
            }
        })));
        rsx! { div {} }
    });
    app.advance_time(Duration::from_millis(1000));
    assert_eq!(
        take(&log),
        ["iv1", "canceller", "sibling@100", "child@100", "iv2", "iv3"]
    );
    assert_eq!(app.elapsed(), Duration::from_millis(1000));
}

/// #2742 review: an interval over an advance its period does not divide,
/// and the clock goes with the app.
#[test]
fn an_interval_over_a_long_advance_and_the_clock_goes_with_the_app_2742() {
    use std::time::Duration;
    let ticks = Rc::new(Cell::new(0u32));
    let t = ticks.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        set_interval(150, move || t.set(t.get() + 1));
        rsx! { div {} }
    });
    app.advance_time(Duration::from_millis(1000));
    assert_eq!(ticks.get(), 6);
    app.advance_time(Duration::from_millis(50));
    assert_eq!(ticks.get(), 7);
    assert!(rinch_core::timer::virtual_time::is_installed());
    drop(app);
    assert!(!rinch_core::timer::virtual_time::is_installed());
}

/// #2742 review: a hidden window cannot gain the keyboard.
#[test]
fn a_hidden_window_does_not_take_the_keyboard_2742() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| {
        rsx! { p { id: "focused", {|| rinch::window::window_focused().to_string()} } }
    })
    .window_props(WindowProps {
        visible: false,
        ..WindowProps::default()
    })
    .mount();
    app.window_focus(true);
    assert!(!app.is_window_focused());
    assert_eq!(app.text("#focused"), "false");
}

/// #2778 part 2: a test reads back the props its app was mounted with —
/// what the harness does not act on (the layer surface, its keyboard mode,
/// the app id) as well as what it does.
#[test]
fn a_test_reads_back_the_window_props_it_mounted_with_2778() {
    let layer = LayerSurface {
        layer: LayerShellLayer::Overlay,
        keyboard: LayerKeyboard::Exclusive,
        ..Default::default()
    };
    let mut app = TestApp::builder(|__scope: &mut RenderScope| rsx! { div {} })
        .window_props(WindowProps {
            width: 420,
            height: 90,
            visible: false,
            app_id: Some("org.example.launcher".into()),
            wayland_layer: Some(layer.clone()),
            ..WindowProps::default()
        })
        .mount();
    let props = app.window_props();
    assert_eq!(props.app_id.as_deref(), Some("org.example.launcher"));
    assert_eq!(props.wayland_layer.as_ref(), Some(&layer));
    assert_eq!((props.width, props.height), (420, 90));
    assert!(!props.visible);

    // As mounted: what happened since is the app's state, not the props'.
    app.resize(500, 120);
    assert_eq!(app.size(), (500, 120));
    assert_eq!(
        (app.window_props().width, app.window_props().height),
        (420, 90)
    );

    // A plain mount reports the defaults.
    let plain = TestApp::mount(|__scope: &mut RenderScope| rsx! { div {} });
    let defaults = WindowProps::default();
    let props = plain.window_props();
    assert_eq!(
        (props.width, props.height),
        (defaults.width, defaults.height)
    );
    assert!(props.visible && props.app_id.is_none() && props.wayland_layer.is_none());
}

/// #2839: under the harness `rinch::time::now()` is the virtual clock. It
/// moves by exactly what `advance_time` gives (the wall clock moving
/// meanwhile does not move it), and a timer's callback reads its own
/// deadline: the clock the timers fire on is the one `now()` answers.
#[test]
fn time_now_advances_by_exactly_what_advance_time_gives_2839() {
    use rinch::time::{Instant, now};
    use std::time::Duration;
    let fired_at: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let f = fired_at.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        set_timeout(250, move || f.set(Some(now())));
        rsx! { div {} }
    });
    let t0 = now();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(now(), t0, "real time passing does not move it");

    app.advance_time(Duration::from_millis(1234));
    assert_eq!(now() - t0, Duration::from_millis(1234));
    assert_eq!(
        fired_at.get().map(|at| at - t0),
        Some(Duration::from_millis(250)),
        "the timeout's callback ran with the clock at its deadline"
    );
    app.advance_time(Duration::from_millis(1));
    assert_eq!(now() - t0, Duration::from_millis(1235));
    assert_eq!(app.elapsed(), Duration::from_millis(1235));
}

/// #2839, the case it was filed for: a taskbar keeps a new window off its
/// bar for a 300 ms grace that starts when the window is heard of. One
/// timeout is armed for the grace's end, computed from `now()`, and on
/// firing the code judges the grace over by comparing instants. At 299 ms
/// the grace is not over and the timer has not fired; one more millisecond
/// and both are true — on every run, because nothing here reads real time.
#[test]
fn a_300ms_grace_armed_from_time_now_ends_exactly_at_300ms_2839() {
    use rinch::time::{Instant, now};
    use std::time::Duration;
    const GRACE: Duration = Duration::from_millis(300);
    let heard: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let fired = Rc::new(Cell::new(false));
    let (h, f) = (heard.clone(), fired.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let shown = Signal::new(false);
        let at = now();
        h.set(Some(at));
        let end = at + GRACE;
        let delay = end.saturating_duration_since(now());
        let delay_ms = u32::try_from(delay.as_nanos().div_ceil(1_000_000)).unwrap();
        let f = f.clone();
        set_timeout(delay_ms, move || {
            f.set(true);
            if now() >= end {
                shown.set(true);
            }
        });
        rsx! { p { id: "bar", {move || if shown.get() { "shown" } else { "hidden" }} } }
    });
    let heard = heard.get().unwrap();
    let grace_over = || now() - heard >= GRACE;

    app.advance_time(Duration::from_millis(299));
    assert!(!grace_over(), "299 ms: the grace is not over");
    assert!(!fired.get(), "299 ms: the timer has not fired");
    assert_eq!(app.text("#bar"), "hidden");

    app.advance_time(Duration::from_millis(1));
    assert!(grace_over(), "300 ms: the grace is over");
    assert!(fired.get(), "300 ms: the timer fired");
    assert_eq!(
        app.text("#bar"),
        "shown",
        "and, judged on the same clock, it found the grace over"
    );
}

/// #2839: the virtual clock `now()` reads is the thread's. Two `TestApp`s
/// alive on one thread share it: advancing either moves `now()` for both.
/// A `TestApp` on another thread has a clock of its own, which this
/// thread's advances do not move and whose advances do not move this one;
/// and a thread with no harness reads the wall clock.
#[test]
fn time_now_is_per_thread_and_shared_by_the_apps_on_a_thread_2839() {
    use rinch::time::{Instant, now};
    use std::time::Duration;
    let ms = Duration::from_millis;
    let mut a = TestApp::mount(|__scope: &mut RenderScope| rsx! { div {} });
    let t0 = now();
    let mut b = TestApp::mount(|__scope: &mut RenderScope| rsx! { div {} });
    assert_eq!(now(), t0, "the second app joins the first's clock");
    a.advance_time(ms(100));
    assert_eq!(now() - t0, ms(100));
    b.advance_time(ms(50));
    assert_eq!(now() - t0, ms(150), "either app's advance moves it");
    assert_eq!((a.elapsed(), b.elapsed()), (ms(150), ms(150)));

    // Another thread's harness: its own clock, from zero.
    let theirs = std::thread::spawn(move || {
        let mut c = TestApp::mount(|__scope: &mut RenderScope| rsx! { div {} });
        let start = now();
        c.advance_time(ms(10_000));
        (now() - start, c.elapsed())
    })
    .join()
    .unwrap();
    assert_eq!(theirs, (ms(10_000), ms(10_000)));
    assert_eq!(
        now() - t0,
        ms(150),
        "its advance did not move this thread's"
    );

    // A thread with no harness: the wall clock.
    let (before, n, after) = std::thread::spawn(|| (Instant::now(), now(), Instant::now()))
        .join()
        .unwrap();
    assert!(before <= n && n <= after);

    drop(b);
    a.advance_time(ms(1));
    assert_eq!(
        now() - t0,
        ms(151),
        "the clock stays while one app is alive"
    );
}
