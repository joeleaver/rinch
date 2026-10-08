//! `EditorHandle::blur` on desktop: an app with several editors takes the
//! keyboard away from one (a pane that stops holding a document) and leaves
//! nothing focused, without touching a keyboard owner that is not that editor.
//!
//! Driven through `handle_event`, as `editor_focus_and_reveal_tests` drives
//! `focus()`: from outside any input event with the frame clock the shell runs
//! (`AboutToWait`), and from a click handler with the drain that follows it.
//! The release and a `focus()` request are applied in the order they were
//! posted (`rinch_core::take_pending_focus_steps`, pinned on its own in
//! `rinch-core`'s `events::selection::tests`).

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

/// Paragraph `i` is `line i`: 6 characters, so it ends at `7 + 8i`.
fn end_of(i: usize) -> Pos {
    Pos(7 + 8 * i)
}

/// What the button's click handler runs, set by each test.
type OnClick = Rc<RefCell<Option<Box<dyn Fn()>>>>;

struct Page {
    app: RinchApp,
    a: crate::editor::EditorHandle,
    b: crate::editor::EditorHandle,
    a_id: usize,
    b_id: usize,
    input: usize,
    /// A plain `div` with a click handler and no `tabindex`: a toolbar
    /// button, whose press leaves the keyboard where it is.
    button: usize,
    on_click: OnClick,
}

fn editor_with(lines: usize) -> crate::editor::EditorHandle {
    let handle = crate::editor::create_editor();
    let html: String = (0..lines).map(|i| format!("<p>line {i}</p>")).collect();
    assert!(handle.load_html(&html));
    handle
}

/// A text input, a toolbar button and two editors, A then B. Nothing is
/// focused.
fn page() -> Page {
    let a = editor_with(3);
    let b = editor_with(3);
    let on_click: OnClick = Rc::default();
    let oninput = register_input_handler(InputCallback::new(|_| {}));
    let ids: Rc<Cell<(usize, usize, usize, usize)>> = Rc::default();
    let (a_in, b_in, click_in, ids_in) = (a.clone(), b.clone(), on_click.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let input = scope.create_element("input");
        input.set_attribute("style", "width: 200px; height: 30px");
        input.set_attribute("data-oninput", &oninput.0.to_string());
        root.append_child(&input);
        let button = scope.create_element("div");
        button.set_attribute("style", "width: 80px; height: 30px");
        let rid = scope.register_handler({
            let click = click_in.clone();
            move || {
                if let Some(f) = click.borrow().as_ref() {
                    f();
                }
            }
        });
        button.set_attribute("data-rid", &rid.0.to_string());
        root.append_child(&button);
        let style = "width: 400px; height: 120px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif";
        let ea = a_in.mount(scope);
        ea.set_attribute("style", style);
        root.append_child(&ea);
        let eb = b_in.mount(scope);
        eb.set_attribute("style", style);
        root.append_child(&eb);
        ids_in.set((
            ea.node_id().0,
            eb.node_id().0,
            input.node_id().0,
            button.node_id().0,
        ));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (a_id, b_id, input, button) = ids.get();
    assert_eq!(app.focus_target, FocusTarget::None, "nothing is focused");
    a.set_selection(Selection::cursor(end_of(0)));
    b.set_selection(Selection::cursor(end_of(0)));
    Page {
        app,
        a,
        b,
        a_id,
        b_id,
        input,
        button,
        on_click,
    }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

/// What the shell does between inputs: run the frame clock a few times.
fn idle(app: &mut RinchApp) {
    for _ in 0..4 {
        ev(app, PlatformEvent::AboutToWait);
    }
}

fn type_char(app: &mut RinchApp, key: KeyCode, text: &str) {
    let modifiers = Modifiers::default();
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: Some(text.to_string()),
            modifiers,
            repeat: KeyRepeat::Fresh,
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

fn click_node(app: &mut RinchApp, id: usize) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, id);
        (x + w / 2.0, y + h / 2.0)
    };
    for event in [
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
    ] {
        ev(app, event);
    }
}

fn first_line(handle: &crate::editor::EditorHandle) -> String {
    let doc = handle.doc();
    let block = doc.child(0);
    (0..block.child_count())
        .filter_map(|j| block.child(j).text().map(str::to_string))
        .collect()
}

fn input_value(page: &Page) -> String {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(page.input)
        .and_then(|n| n.attributes.get("value").cloned())
        .unwrap_or_default()
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

fn focused_a(p: &mut Page) {
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id), "control");
    assert!(caret_shown(&p.app), "control: A's caret is drawn");
}

#[test]
fn blur_leaves_nothing_focused_and_keys_reach_no_editor() {
    let mut p = page();
    focused_a(&mut p);

    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
    assert!(!caret_shown(&p.app), "the caret is hidden");
    type_char(&mut p.app, KeyCode::KeyX, "X");
    idle(&mut p.app);
    assert_eq!(first_line(&p.a), "line 0", "the key did not reach A");
    assert_eq!(first_line(&p.b), "line 0", "nor B");
    assert_eq!(input_value(&p), "", "nor the input");
}

#[test]
fn blur_of_an_editor_without_the_keyboard_leaves_another_editor_focused() {
    let mut p = page();
    p.b.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.b_id), "control");

    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.b_id));
    assert!(caret_shown(&p.app), "B's caret is still drawn");
    type_char(&mut p.app, KeyCode::KeyZ, "Z");
    assert_eq!(first_line(&p.b), "line 0Z", "the key went to B");
    assert_eq!(first_line(&p.a), "line 0");
}

#[test]
fn blur_of_an_editor_without_the_keyboard_leaves_a_text_input_focused() {
    let mut p = page();
    rinch_core::request_focus(p.app.doc_key(), p.input);
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input), "control");

    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
    type_char(&mut p.app, KeyCode::KeyQ, "q");
    assert_eq!(input_value(&p), "q", "the key went to the input");
    assert_eq!(first_line(&p.a), "line 0");
}

#[test]
fn blur_while_nothing_is_focused_changes_nothing() {
    let mut p = page();
    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
}

#[test]
fn focus_then_blur_in_one_turn_leaves_nothing_focused() {
    // B holds the keyboard; A takes it and lets go, in that order: nobody
    // holds it, as if the two had run one after the other.
    let mut p = page();
    p.b.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.b_id), "control");

    p.a.focus();
    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
    assert!(!caret_shown(&p.app), "no caret is drawn");
}

#[test]
fn blur_then_focus_in_one_turn_leaves_the_editor_focused() {
    let mut p = page();
    focused_a(&mut p);
    p.a.blur();
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id));
    assert!(caret_shown(&p.app));

    // And from nothing focused.
    let mut p = page();
    p.a.blur();
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id));
}

#[test]
fn from_a_click_handler_the_later_call_wins_before_the_next_key() {
    let mut p = page();
    let a = p.a.clone();
    *p.on_click.borrow_mut() = Some(Box::new(move || {
        a.focus();
        a.blur();
    }));
    p.b.focus();
    idle(&mut p.app);
    click_node(&mut p.app, p.button);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::None,
        "focus() then blur(): applied right after the handler"
    );

    let a = p.a.clone();
    *p.on_click.borrow_mut() = Some(Box::new(move || {
        a.blur();
        a.focus();
    }));
    click_node(&mut p.app, p.button);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Editor(p.a_id),
        "blur() then focus()"
    );
    type_char(&mut p.app, KeyCode::KeyZ, "Z");
    assert_eq!(first_line(&p.a), "line 0Z");

    let a = p.a.clone();
    *p.on_click.borrow_mut() = Some(Box::new(move || a.blur()));
    click_node(&mut p.app, p.button);
    assert_eq!(p.app.focus_target, FocusTarget::None, "blur() alone");
    type_char(&mut p.app, KeyCode::KeyY, "Y");
    assert_eq!(
        first_line(&p.a),
        "line 0Z",
        "the next key reaches no editor"
    );
}

#[test]
fn two_editors_blurred_in_one_turn_are_both_released() {
    // The one holding the keyboard is the second blurred: the first release
    // (of an editor without it) must not swallow it.
    let mut p = page();
    focused_a(&mut p);
    p.b.blur();
    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
}

#[test]
fn the_selection_survives_blur_and_focus_brings_it_back() {
    let mut p = page();
    let range = Selection::text(Pos(1), end_of(0));
    p.a.set_selection(range.clone());
    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.a_id), "control");

    p.a.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::None);
    assert_eq!(p.a.selection(), range, "blur keeps the selection");

    p.a.focus();
    idle(&mut p.app);
    assert_eq!(p.a.selection(), range, "and focus brings it back as it was");
    type_char(&mut p.app, KeyCode::KeyW, "W");
    assert_eq!(first_line(&p.a), "W", "typing replaces the kept selection");
}

#[test]
fn blur_does_not_discard_a_focus_request_for_another_node() {
    // A field asks for the keyboard and an editor without it is blurred in
    // the same turn: the field still gets it.
    let mut p = page();
    focused_a(&mut p);
    rinch_core::request_focus(p.app.doc_key(), p.input);
    p.b.blur();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
}
