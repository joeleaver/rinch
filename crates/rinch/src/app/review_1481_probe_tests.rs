//! Review probes for PR #1481 (`EditorHandle::blur`), desktop.
//! Each prints what it measured; assertions say what the PR's docs promise.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    a: crate::editor::EditorHandle,
    b: crate::editor::EditorHandle,
    a_id: usize,
    #[allow(dead_code)]
    b_id: usize,
    input: usize,
    changes: Rc<Cell<usize>>,
}

fn editor_with(lines: usize) -> crate::editor::EditorHandle {
    let handle = crate::editor::create_editor();
    let html: String = (0..lines).map(|i| format!("<p>line {i}</p>")).collect();
    assert!(handle.load_html(&html));
    handle
}

fn page() -> Page {
    let a = editor_with(3);
    let b = editor_with(3);
    let changes: Rc<Cell<usize>> = Rc::default();
    let oninput = register_input_handler(InputCallback::new(|_| {}));
    let ch = changes.clone();
    let onchange = register_input_handler(InputCallback::new(move |_| ch.set(ch.get() + 1)));
    let ids: Rc<Cell<(usize, usize, usize)>> = Rc::default();
    let (a_in, b_in, ids_in) = (a.clone(), b.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        input.set_attribute("data-oninput", &oninput.0.to_string());
        input.set_attribute("data-onchange", &onchange.0.to_string());
        root.append_child(&input);
        let style = "width: 400px; height: 120px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif";
        let ea = a_in.mount(scope);
        ea.set_attribute("style", style);
        root.append_child(&ea);
        let eb = b_in.mount(scope);
        eb.set_attribute("style", style);
        root.append_child(&eb);
        ids_in.set((ea.node_id().0, eb.node_id().0, input.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a_id, b_id, input) = ids.get();
    a.set_selection(Selection::cursor(Pos(7)));
    b.set_selection(Selection::cursor(Pos(7)));
    Page {
        app,
        a,
        b,
        a_id,
        b_id,
        input,
        changes,
    }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn idle(app: &mut RinchApp) {
    for _ in 0..4 {
        ev(app, PlatformEvent::AboutToWait);
    }
}

fn key_down(app: &mut RinchApp, key: KeyCode, logical: Option<&str>, text: Option<&str>) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: logical.map(str::to_string),
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
    );
}

fn first_line(handle: &crate::editor::EditorHandle) -> String {
    let doc = handle.doc();
    let block = doc.child(0);
    (0..block.child_count())
        .filter_map(|j| block.child(j).text().map(str::to_string))
        .collect()
}

fn preedit_shown(app: &RinchApp) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes.iter().any(|(_, n)| {
        n.attributes.contains_key("data-pm-preedit")
            && !n.attributes.get("style").is_some_and(|s| s.contains("display: none"))
    })
}

fn caret_shown(app: &RinchApp) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes.iter().any(|(_, n)| {
        n.attributes.contains_key("data-pm-caret")
            && n.attributes
                .get("style")
                .is_some_and(|s| s.contains("visibility: visible"))
    })
}

fn focus_a(p: &mut Page) {
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id), "control");
}

/// P1: blur of an unfocused editor while a modified <input> holds the
/// keyboard fires no `onchange` and leaves the field's edit state.
#[test]
fn p1_blur_of_unfocused_editor_commits_no_onchange() {
    let mut p = page();
    rinch_core::request_focus(p.app.doc_key(), p.input);
    idle(&mut p.app);
    key_down(&mut p.app, KeyCode::KeyQ, None, Some("q"));
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input), "control");
    p.a.blur();
    p.b.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
    assert_eq!(p.changes.get(), 0, "no change commit");
}

/// P2: a preedit shown in the editor when blur() lands.
#[test]
fn p2_blur_mid_preedit() {
    let mut p = page();
    focus_a(&mut p);
    ev(
        &mut p.app,
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "nihon".into(),
            cursor: None,
        }),
    );
    idle(&mut p.app);
    assert!(preedit_shown(&p.app), "control: a preedit is shown");
    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
    // What the shell does next: the IME is switched off, winit reports it.
    ev(&mut p.app, PlatformEvent::Ime(ImeEvent::Disabled));
    idle(&mut p.app);
    assert!(!preedit_shown(&p.app), "a blurred editor shows no preedit");
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id));
    assert!(!preedit_shown(&p.app), "the composition ended with the blur: focus brings no preedit back");
}

/// P2b: the same through a press on the <input> (no blur()): is it pre-existing?
#[test]
fn p2b_press_away_mid_preedit() {
    let mut p = page();
    focus_a(&mut p);
    ev(
        &mut p.app,
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "nihon".into(),
            cursor: None,
        }),
    );
    idle(&mut p.app);
    assert!(preedit_shown(&p.app), "control");
    rinch_core::request_focus(p.app.doc_key(), p.input);
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
    ev(&mut p.app, PlatformEvent::Ime(ImeEvent::Disabled));
    idle(&mut p.app);
    assert!(!preedit_shown(&p.app), "a blurred editor shows no preedit");
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id));
    assert!(!preedit_shown(&p.app), "the composition ended with the blur: focus brings no preedit back");
}

/// P3: the caret stays hidden through a relayout after blur.
#[test]
fn p3_caret_stays_hidden_after_relayout() {
    let mut p = page();
    focus_a(&mut p);
    assert!(caret_shown(&p.app), "control");
    p.a.blur();
    idle(&mut p.app);
    assert!(!caret_shown(&p.app));
    p.app.resolve_and_repaint(700.0, 500.0);
    for _ in 0..4 {
        p.app.handle_event(PlatformEvent::AboutToWait, (700, 500), 1.0);
    }
    assert!(!caret_shown(&p.app), "still hidden after a relayout");
    // a programmatic selection change on the blurred editor draws no caret
    p.a.set_selection(Selection::cursor(Pos(3)));
    idle(&mut p.app);
    assert!(!caret_shown(&p.app), "still hidden after set_selection");
    assert!(!p.app.ime_state().enabled, "IME off");
}

/// P4: blur() from an `on_key` handler (Escape), then another key in the same
/// batch (no AboutToWait between): does the second key still type?
#[test]
fn p4_blur_from_on_key_then_a_key_in_the_same_batch() {
    let mut p = page();
    focus_a(&mut p);
    let a = p.a.clone();
    p.a.on_key(move |k| {
        if k.key == "Escape" {
            a.blur();
            true
        } else {
            false
        }
    });
    key_down(&mut p.app, KeyCode::Escape, Some("Escape"), None);
    let right_after = p.app.focus_target;
    key_down(&mut p.app, KeyCode::KeyX, None, Some("x"));
    idle(&mut p.app);
    eprintln!(
        "P4 focus right after the Escape handler = {right_after:?}; A's line = {:?}",
        first_line(&p.a)
    );
    assert_eq!(p.app.focus_target, FocusTarget::None, "after the frame clock");
    assert_eq!(first_line(&p.a), "line 0", "the key after the blur typed");
}

/// P5: selection / caret hooks across a blur.
#[test]
fn p5_hooks_across_blur() {
    let mut p = page();
    focus_a(&mut p);
    let sel: Rc<Cell<usize>> = Rc::default();
    let moved: Rc<Cell<usize>> = Rc::default();
    let chg: Rc<Cell<usize>> = Rc::default();
    let (s, m, c) = (sel.clone(), moved.clone(), chg.clone());
    p.a.on_selection_change(move |_| s.set(s.get() + 1));
    p.a.on_caret_moved(move || m.set(m.get() + 1));
    p.a.on_change(move || c.set(c.get() + 1));
    p.a.blur();
    idle(&mut p.app);
    eprintln!(
        "P5 on_selection_change={} on_caret_moved={} on_change={}",
        sel.get(),
        moved.get(),
        chg.get()
    );
    assert_eq!(sel.get(), 0);
    assert_eq!(chg.get(), 0);
}

/// P6: a read-only editor blurs like any other.
#[test]
fn p6_read_only_editor_blurs() {
    let mut p = page();
    p.a.set_read_only(true);
    focus_a(&mut p);
    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
    assert!(!caret_shown(&p.app));
}

/// P7: two documents on one thread: a release for one is not applied or
/// consumed by the other's loop.
#[test]
fn p7_two_documents() {
    let mut p1 = page();
    let mut p2 = page();
    focus_a(&mut p1);
    focus_a(&mut p2);
    p1.a.blur();
    idle(&mut p2.app);
    assert_eq!(p2.app.focus_target, FocusTarget::Editor(p2.a_id), "doc 2 untouched");
    assert_eq!(p1.app.focus_target, FocusTarget::Editor(p1.a_id), "not yet applied");
    idle(&mut p1.app);
    assert_eq!(p1.app.focus_target, FocusTarget::None);
    assert_eq!(p2.app.focus_target, FocusTarget::Editor(p2.a_id));
}

/// P8: a blur with no change of owner asks for what?
#[test]
fn p8_noop_blur_actions() {
    let mut p = page();
    idle(&mut p.app);
    p.a.blur();
    let actions = ev(&mut p.app, PlatformEvent::AboutToWait);
    eprintln!("P8 actions after a no-op blur: {actions:?}");
}

/// P9: an effect that focuses, queued by a signal write BEFORE blur() in the
/// same batch.
#[test]
fn p9_signal_write_whose_effect_focuses_then_blur() {
    let mut p = page();
    let sig = rinch_core::Signal::new(0);
    let a = p.a.clone();
    let _e = rinch_core::reactive::Effect::new(move || {
        if sig.get() > 0 {
            a.focus();
        }
    });
    idle(&mut p.app);
    let a = p.a.clone();
    rinch_core::batch(|| {
        sig.set(1); // program order: focus (via the effect) ...
        a.blur(); // ... then blur
    });
    idle(&mut p.app);
    eprintln!("P9 focus after set(effect focuses); blur() = {:?}", p.app.focus_target);
    assert_eq!(p.app.focus_target, FocusTarget::None, "set (its effect focuses) then blur(): the later call wins");
}
