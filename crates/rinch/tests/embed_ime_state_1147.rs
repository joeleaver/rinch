//! Issue #1147: an embedded host can see the IME state the focus arbiter wants.
//!
//! The desktop shell reconciles the window's input method with the focused
//! target after every event-loop turn (`sync_ime`). An embedded host has no
//! window rinch owns, so `RinchContext` hands it the same two answers:
//! [`RinchContext::ime_state`] (enabled + the caret's candidate-box rect, in
//! logical px) and [`RinchContext::take_ime_change`] (the `Enable` / `Update` /
//! `Disable` request that brings the host's IME from what this method last
//! reported to that state — the decision `sync_ime` makes, one function).
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test embed_ime_state_1147
//! The editor case also needs `desktop`:
//!     cargo test -p rinch --features embed,desktop --test embed_ime_state_1147

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{ElementBounds, ImeChange, ImeState, RinchContext, RinchContextConfig};
use rinch::platform::{ImeEvent, MouseButton, PlatformEvent};
use rinch::prelude::*;

// ── harness ──────────────────────────────────────────────────────────────────
//
// `RinchContext::new` registers its thread as THE main thread process-wide, so
// every test runs on one worker thread.

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });
    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 800,
        height: 600,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

fn click(x: f32, y: f32) -> [PlatformEvent; 2] {
    [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ]
}

/// A text field at (100, 100), 300x30, and empty space below it.
fn input_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! {
            div { style: "width: 800px; height: 600px; padding: 100px; box-sizing: border-box;",
                input {
                    style: "display: block; width: 300px; height: 30px; padding: 0 4px; \
                            font-size: 16px; line-height: 20px;",
                    oninput: move |_v: String| {},
                }
            }
        }
    })
}

// ── tests ────────────────────────────────────────────────────────────────────

/// The area an `Enable` carries, `None` for any other request.
fn enabled_at(change: Option<ImeChange>) -> Option<Option<ElementBounds>> {
    match change {
        Some(ImeChange::Enable { cursor_area, .. }) => Some(cursor_area),
        _ => None,
    }
}

/// The area an `Update` carries, `None` for any other request.
fn updated_to(change: Option<ImeChange>) -> Option<ElementBounds> {
    match change {
        Some(ImeChange::Update { cursor_area, .. }) => Some(cursor_area),
        _ => None,
    }
}

struct InputRun {
    before: (ImeState, Option<ImeChange>),
    first: Option<ImeChange>,
    enabled: ImeState,
    again: Option<ImeChange>,
    one_char: (ImeState, Option<ImeChange>),
    six_chars: (ImeState, Option<ImeChange>),
    blurred: (ImeState, Option<ImeChange>),
}

#[test]
fn a_focused_input_enables_ime_at_its_caret_and_a_blur_disables_it() {
    let run = on_ui_thread(|| {
        let mut ctx = input_context();
        ctx.update(&[]);
        let before = (ctx.ime_state(), ctx.take_ime_change());

        ctx.update(&click(150.0, 115.0));
        let first = ctx.take_ime_change();
        let enabled = ctx.ime_state();
        // Nothing moved: no second request.
        let again = ctx.take_ime_change();

        // A composition fed back in moves the caret; the host is told where
        // the candidate box goes now. The vertical baseline is the caret after
        // one character: an empty field's area is its text origin, which is
        // not the typed caret's y to the pixel under every stylesheet.
        let mut commit = |text: &str| {
            ctx.update(&[PlatformEvent::Ime(ImeEvent::Commit(text.into()))]);
            (ctx.ime_state(), ctx.take_ime_change())
        };
        let one_char = commit("a");
        let six_chars = commit("bcdef");

        ctx.update(&click(600.0, 500.0));
        let blurred = (ctx.ime_state(), ctx.take_ime_change());
        InputRun {
            before,
            first,
            enabled,
            again,
            one_char,
            six_chars,
            blurred,
        }
    });

    assert!(!run.before.0.enabled, "nothing focused: no IME");
    assert_eq!(run.before.1, None, "and nothing to tell the host");

    assert!(run.enabled.enabled, "a focused <input> wants the IME");
    let start = run.enabled.cursor_area.expect("a caret area");
    assert!(
        (100.0..=110.0).contains(&start.x)
            && (100.0..=130.0).contains(&start.y)
            && start.height > 0.0,
        "the caret area is at the start of the field's text: {start:?}"
    );
    assert_eq!(enabled_at(run.first), Some(Some(start)), "{:?}", run.first);
    assert_eq!(run.again, None, "an unchanged state asks for nothing");

    let a = run.one_char.0.cursor_area.expect("a caret area");
    assert!(
        a.x > start.x,
        "one character moves the caret: {start:?} -> {a:?}"
    );
    assert_eq!(updated_to(run.one_char.1), Some(a), "{:?}", run.one_char.1);

    let f = run.six_chars.0.cursor_area.expect("a caret area");
    assert!(
        f.x > a.x + 20.0 && (f.y - a.y).abs() < 0.5,
        "the caret moved right along the line past the committed text: {a:?} -> {f:?}"
    );
    assert_eq!(
        updated_to(run.six_chars.1),
        Some(f),
        "{:?}",
        run.six_chars.1
    );

    assert_eq!(
        run.blurred.1,
        Some(ImeChange::Disable),
        "a blur turns the IME off"
    );
    assert!(!run.blurred.0.enabled);
}

#[test]
fn a_window_blur_disables_ime_and_refocus_enables_it_again() {
    let (blur, refocus) = on_ui_thread(|| {
        let mut ctx = input_context();
        ctx.update(&click(150.0, 115.0));
        assert!(matches!(
            ctx.take_ime_change(),
            Some(ImeChange::Enable { .. })
        ));
        ctx.update(&[PlatformEvent::WindowFocus(false)]);
        let blur = ctx.take_ime_change();
        ctx.update(&[PlatformEvent::WindowFocus(true)]);
        (blur, ctx.take_ime_change())
    });
    assert_eq!(blur, Some(ImeChange::Disable));
    assert!(matches!(enabled_at(refocus), Some(Some(_))), "{refocus:?}");
}

#[test]
fn a_registered_on_ime_node_drives_ime_with_its_own_caret_rect_and_a_plain_one_does_not() {
    let (text_node, plain_node) = on_ui_thread(|| {
        let mut ctx = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            let root = __scope.create_element("div");
            let text = __scope.create_element("div");
            text.set_attribute("tabindex", "0");
            text.set_attribute("style", "width: 800px; height: 300px;");
            register_focus_target(
                &text,
                FocusEntry::new()
                    .on_ime(|_e| {})
                    .caret_rect(|| Some((37.0, 51.0, 2.0, 19.0))),
            );
            let plain = __scope.create_element("div");
            plain.set_attribute("tabindex", "0");
            plain.set_attribute("style", "width: 800px; height: 300px;");
            register_focus_target(&plain, FocusEntry::new().on_key(|_k| true));
            root.append_child(&text);
            root.append_child(&plain);
            root
        });
        ctx.update(&click(400.0, 100.0));
        let text_node = (ctx.ime_state(), ctx.take_ime_change());
        ctx.update(&click(400.0, 450.0));
        let plain_node = (ctx.ime_state(), ctx.wants_keyboard(), ctx.take_ime_change());
        (text_node, plain_node)
    });
    assert!(text_node.0.enabled, "an `on_ime` target is a text target");
    let rect = ElementBounds {
        x: 37.0,
        y: 51.0,
        width: 2.0,
        height: 19.0,
    };
    assert_eq!(text_node.0.cursor_area, Some(rect));
    assert_eq!(enabled_at(text_node.1), Some(Some(rect)));
    // A key-consuming node wants the keyboard and no IME: the question
    // `wants_keyboard` cannot answer.
    assert!(plain_node.1, "control: it does take the keys");
    assert!(!plain_node.0.enabled);
    assert_eq!(plain_node.2, Some(ImeChange::Disable));
}

#[cfg(feature = "desktop")]
#[test]
fn a_focused_editor_enables_ime_at_its_caret_and_read_only_turns_it_off() {
    let (focused, change, read_only) = on_ui_thread(|| {
        use std::cell::RefCell;
        use std::rc::Rc;
        let slot: Rc<RefCell<Option<rinch::editor::EditorHandle>>> = Rc::default();
        let slot_in = slot.clone();
        let mut ctx = RinchContext::new(cfg(), move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "padding: 60px 0 0 80px;");
            let (container, handle) = rinch::editor::mount_editor(scope);
            handle.load_html("<p>hello</p>");
            container.set_attribute(
                "style",
                "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
            );
            root.append_child(&container);
            *slot_in.borrow_mut() = Some(handle);
            root
        });
        ctx.update(&[]);
        let handle = slot.borrow_mut().take().expect("mounted");
        handle.focus();
        for _ in 0..3 {
            ctx.update(&[]);
        }
        assert!(ctx.app().has_focused_contenteditable(), "precondition");
        let focused = ctx.ime_state();
        let change = ctx.take_ime_change();
        handle.set_read_only(true);
        ctx.update(&[]);
        (focused, change, (ctx.ime_state(), ctx.take_ime_change()))
    });
    assert!(focused.enabled, "a focused editor wants the IME");
    let caret = focused.cursor_area.expect("the editor's caret");
    assert!(
        caret.x >= 80.0 && caret.y >= 60.0 && caret.height > 0.0,
        "{caret:?}"
    );
    assert_eq!(enabled_at(change), Some(Some(caret)));
    assert!(!read_only.0.enabled, "a read-only editor composes nothing");
    assert_eq!(read_only.1, Some(ImeChange::Disable));
}
