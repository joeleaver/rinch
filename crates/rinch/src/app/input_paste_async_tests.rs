//! Ctrl+V in a plain `<input>` / `<textarea>` reads the clipboard **off the UI
//! thread** (issue #328), the way the editor's paste has since #149.
//!
//! The read is a request to another process; against a hung X11 selection
//! owner arboard waits up to four seconds, and `handle_paste` used to make it
//! synchronously, freezing the window for that long. Now the chord returns at
//! once, the read runs on the clipboard worker, and the insertion is deferred
//! work (`RinchApp::run_deferred_work`) that runs with `&mut RinchApp` on a
//! later turn — landing in the field's **current** selection if that field
//! still holds the keyboard, and nowhere if focus moved on meanwhile.
//!
//! Every fixture reads through `rinch_clipboard`'s in-memory backend under the
//! process-wide clipboard lock, and `settle` waits for the completion **by
//! count**, so a paste that is dropped is still observed to have been
//! delivered and judged — a zero here is never "the read never answered".
//! The selection is `2..5` of `hello world`, off the `0..n` fixed point, and
//! the pasted text is three bytes so a one-character undo cannot pass for a
//! whole-paste one.

use super::text_context_menu_tests::clipboard_lock;
use super::*;
use rinch_core::dom::NodeId;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;
use std::time::{Duration, Instant};

const VP: (u32, u32) = (800, 600);

const FIELD_STYLE: &str = "width: 300px; height: 30px; padding: 0; margin: 0; \
     font-size: 16px; line-height: 20px; font-family: sans-serif";

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn left_press(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

fn key_with(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
}

fn key(app: &mut RinchApp, key: KeyCode) {
    key_with(app, key, Modifiers::default());
}

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn attr(app: &RinchApp, id: usize, name: &str) -> Option<String> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(id).and_then(|n| n.attributes.get(name).cloned())
}

/// `(value, selection start, cursor)` as the DOM carries them.
fn field(app: &RinchApp, id: usize) -> (String, String, String) {
    (
        attr(app, id, "value").unwrap_or_default(),
        attr(app, id, "data-selection-start").unwrap_or_default(),
        attr(app, id, "data-cursor-pos").unwrap_or_default(),
    )
}

fn state(value: &str, anchor: usize, head: usize) -> (String, String, String) {
    (value.into(), anchor.to_string(), head.to_string())
}

fn abs_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, id)
}

/// Focus `id` with a press at its left edge.
fn focus(app: &mut RinchApp, id: usize) {
    let (bx, by, _, bh) = abs_box(app, id);
    left_press(app, bx + 1.0, by + bh / 2.0);
    assert_eq!(app.focus_target, FocusTarget::Input(id));
}

/// Focus `id` and select bytes `2..5` through the keyboard.
fn focus_and_select(app: &mut RinchApp, id: usize) {
    focus(app, id);
    key(app, KeyCode::Home);
    key(app, KeyCode::ArrowRight);
    key(app, KeyCode::ArrowRight);
    for _ in 0..3 {
        key_with(app, KeyCode::ArrowRight, shift());
    }
    assert_eq!(field(app, id), state("hello world", 2, 5), "precondition");
}

fn ctrl_v(app: &mut RinchApp) {
    key_with(app, KeyCode::KeyV, primary());
}

/// Wait for the clipboard worker to answer and the deferred completion to
/// run; answers how many completions ran. Panics after five seconds, so a
/// completion that never arrives fails loudly rather than reading as a
/// dropped paste. Shared with `text_context_menu_tests`, whose Paste item
/// runs this same asynchronous path.
pub(super) fn settle(app: &mut RinchApp) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        rinch_core::drain_main_callbacks();
        let ran = app.run_deferred_work();
        if ran > 0 {
            app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            return ran;
        }
        assert!(
            Instant::now() < deadline,
            "the paste completion never arrived"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

type Log = Rc<RefCell<Vec<String>>>;

#[derive(Clone, Copy)]
struct Ids {
    input: usize,
    textarea: usize,
}

/// An `<input>` and a `<textarea>`, both holding `hello world`, each logging
/// its `oninput`. `extra` lands on the input as attributes.
fn page(extra: &'static [(&'static str, &'static str)]) -> (RinchApp, Ids, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str| {
        let log = log.clone();
        register_input_handler(InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        }))
    };
    let input_h = record("input");
    let textarea_h = record("textarea");
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("style", FIELD_STYLE);
        input.set_attribute("value", "hello world");
        input.set_attribute("data-oninput", &input_h.0.to_string());
        for (k, v) in extra {
            input.set_attribute(k, v);
        }
        let textarea = scope.create_element("textarea");
        textarea.set_attribute("style", FIELD_STYLE);
        textarea.set_attribute("value", "hello world");
        textarea.set_attribute("data-oninput", &textarea_h.0.to_string());
        root.append_child(&input);
        root.append_child(&textarea);
        ids_in.set(Some(Ids {
            input: input.node_id().0,
            textarea: textarea.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, ids.get().expect("ids captured at mount"), log)
}

fn set_attr(app: &mut RinchApp, id: usize, name: &str) {
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_attribute(NodeId(id), name, "");
}

fn clip(text: &str) {
    rinch_clipboard::clear().unwrap();
    rinch_clipboard::copy_text(text).unwrap();
}

// ── The read no longer holds the UI thread ───────────────────────────────────

/// The reproduction: `handle_paste` used to insert before the chord returned,
/// because it read the clipboard on the UI thread. Now the chord returns with
/// the field untouched and the paste lands when the read answers — one
/// `oninput`, the caret after the pasted text.
#[test]
fn ctrl_v_returns_before_the_read_and_the_paste_lands_when_it_answers() {
    let _lock = clipboard_lock();
    let (mut app, ids, log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    log.borrow_mut().clear();

    ctrl_v(&mut app);
    assert_eq!(
        field(&app, ids.input),
        state("hello world", 2, 5),
        "the chord returned without waiting for the clipboard"
    );
    assert!(log.borrow().is_empty(), "and fired no oninput yet");

    assert_eq!(settle(&mut app), 1);
    assert_eq!(field(&app, ids.input), state("heXYZ world", 5, 5));
    assert_eq!(*log.borrow(), vec!["input:heXYZ world".to_string()]);
    assert_eq!(app.focus_target, FocusTarget::Input(ids.input));
}

/// A `<textarea>` goes through the same path.
#[test]
fn a_textarea_pastes_asynchronously_too() {
    let _lock = clipboard_lock();
    let (mut app, ids, _log) = page(&[]);
    focus_and_select(&mut app, ids.textarea);
    clip("XYZ");
    ctrl_v(&mut app);
    assert_eq!(field(&app, ids.textarea), state("hello world", 2, 5));
    assert_eq!(settle(&mut app), 1);
    assert_eq!(field(&app, ids.textarea), state("heXYZ world", 5, 5));
}

/// One paste is one undo step (#288/#928), deferred or not: Ctrl+Z takes all
/// three pasted bytes out and puts the replaced `llo` back.
#[test]
fn a_deferred_paste_is_one_undo_step() {
    let _lock = clipboard_lock();
    let (mut app, ids, _log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    ctrl_v(&mut app);
    settle(&mut app);
    assert_eq!(field(&app, ids.input).0, "heXYZ world");
    key_with(&mut app, KeyCode::KeyZ, primary());
    assert_eq!(field(&app, ids.input).0, "hello world");
}

// ── Where it lands: the field's selection when the read answers ──────────────

/// The browser's rule: the paste goes into the focused field's **current**
/// selection. The caret moved to the end while the read was in flight, so the
/// text is appended there and `llo` — selected at the chord — survives.
#[test]
fn the_paste_lands_at_the_selection_the_field_holds_when_the_read_answers() {
    let _lock = clipboard_lock();
    let (mut app, ids, _log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    ctrl_v(&mut app);
    key(&mut app, KeyCode::End);
    assert_eq!(field(&app, ids.input), state("hello world", 11, 11));
    assert_eq!(settle(&mut app), 1);
    assert_eq!(field(&app, ids.input), state("hello worldXYZ", 14, 14));
}

/// Focus moved to another field while the read was in flight: the paste was
/// aimed at the first field, so it lands in neither. The second Ctrl+V is the
/// positive control — its completion arrives after the first one's, so the
/// textarea holding exactly one `XYZ` shows the first was judged and dropped,
/// not delivered late.
#[test]
fn a_focus_move_during_the_read_drops_the_paste() {
    let _lock = clipboard_lock();
    let (mut app, ids, log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    ctrl_v(&mut app);
    focus_and_select(&mut app, ids.textarea);
    assert_eq!(settle(&mut app), 1, "the first completion ran");
    assert_eq!(field(&app, ids.input).0, "hello world");
    assert_eq!(field(&app, ids.textarea), state("hello world", 2, 5));
    assert!(log.borrow().is_empty(), "no oninput anywhere");

    ctrl_v(&mut app);
    settle(&mut app);
    assert_eq!(field(&app, ids.textarea), state("heXYZ world", 5, 5));
    assert_eq!(field(&app, ids.input).0, "hello world");
}

/// Away and back again before the read answers is still a focus move: the
/// paste belongs to the gesture that asked for it, and a node id alone cannot
/// tell the two apart.
#[test]
fn focus_away_and_back_during_the_read_drops_the_paste() {
    let _lock = clipboard_lock();
    let (mut app, ids, _log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    ctrl_v(&mut app);
    focus(&mut app, ids.textarea);
    focus_and_select(&mut app, ids.input);
    assert_eq!(settle(&mut app), 1, "the completion ran");
    assert_eq!(field(&app, ids.input), state("hello world", 2, 5));

    // Positive control: a paste asked for in this gesture lands, once.
    ctrl_v(&mut app);
    settle(&mut app);
    assert_eq!(field(&app, ids.input), state("heXYZ world", 5, 5));
}

// ── Access: read-only and disabled ───────────────────────────────────────────

/// A read-only field starts no read at all — the insertion would be refused,
/// and a read is not free to make and discard (four seconds against a hung
/// owner; a clipboard-access prompt on some platforms). What the editor does
/// for a read-only editor (#149).
#[test]
fn a_readonly_field_starts_no_read() {
    let _lock = clipboard_lock();
    let (mut app, ids, _log) = page(&[("readonly", "")]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    assert!(
        !app.handle_paste(),
        "no clipboard read for a readonly field"
    );
    assert_eq!(field(&app, ids.input), state("hello world", 2, 5));

    // Control: the same field without `readonly` does start one.
    let (mut app, ids, _log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    assert!(app.handle_paste());
    settle(&mut app);
    assert_eq!(field(&app, ids.input).0, "heXYZ world");
}

/// A field that became read-only while the read was in flight refuses the
/// paste when it answers.
#[test]
fn a_field_made_readonly_during_the_read_refuses_the_paste() {
    let _lock = clipboard_lock();
    let (mut app, ids, log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    log.borrow_mut().clear();
    ctrl_v(&mut app);
    set_attr(&mut app, ids.input, "readonly");
    assert_eq!(settle(&mut app), 1);
    assert_eq!(field(&app, ids.input).0, "hello world");
    assert!(log.borrow().is_empty());
}

/// A field that became disabled while the read was in flight gets nothing,
/// and loses the keyboard as a disabled field does (#315).
#[test]
fn a_field_disabled_during_the_read_gets_nothing() {
    let _lock = clipboard_lock();
    let (mut app, ids, log) = page(&[]);
    focus_and_select(&mut app, ids.input);
    clip("XYZ");
    log.borrow_mut().clear();
    ctrl_v(&mut app);
    set_attr(&mut app, ids.input, "disabled");
    assert_eq!(settle(&mut app), 1);
    assert_eq!(field(&app, ids.input).0, "hello world");
    assert!(log.borrow().is_empty());
}

// ── The deferred-work queue itself ───────────────────────────────────────────

/// Work sent from another thread runs on the next `run_deferred_work`, with
/// the app, in the order it was sent — and exactly once.
#[test]
fn deferred_work_runs_once_in_order_with_the_app() {
    let (mut app, ids, _log) = page(&[]);
    let sender = app.app_work_sender();
    let worker = std::thread::spawn(move || {
        for tag in ["a", "b", "c"] {
            sender.send(move |app: &mut RinchApp| {
                let d = app.doc.clone().unwrap();
                let mut d = d.borrow_mut();
                let old = d
                    .tree
                    .get(ids.textarea)
                    .and_then(|n| n.attributes.get("data-order").cloned())
                    .unwrap_or_default();
                d.set_attribute(NodeId(ids.textarea), "data-order", &format!("{old}{tag}"));
            });
        }
    });
    worker.join().unwrap();
    assert_eq!(app.run_deferred_work(), 3);
    assert_eq!(
        attr(&app, ids.textarea, "data-order").as_deref(),
        Some("abc")
    );
    assert_eq!(app.run_deferred_work(), 0, "each item runs once");
}

/// Work sent after its app was dropped goes nowhere, and does not panic.
#[test]
fn deferred_work_for_a_dropped_app_is_discarded() {
    let (app, _ids, _log) = page(&[]);
    let sender = app.app_work_sender();
    drop(app);
    // The closure must be `Send`; a flag through an `Arc` records a run.
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = flag.clone();
    sender.send(move |_app: &mut RinchApp| {
        f.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
}
