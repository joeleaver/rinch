//! `data-nofocus`: a control can take the click without taking the keyboard
//! (issue #312).
//!
//! Browsers solve this with `preventDefault()` on `mousedown`, which suppresses
//! the focus change while still delivering the click. Desktop had no
//! equivalent, so once the mousedown claim started focusing every `tabindex`
//! node (#147, decision 2) an editor toolbar button carrying one took the
//! keyboard away from the editor it acts on — Bold would apply to a selection
//! that no longer had focus. The claim runs *before* `handle_click`'s
//! `data-rid` carve-out, so that carve-out could not save it.
//!
//! The attribute is read anywhere on the hit's ancestor chain, so a whole
//! toolbar carries it once rather than every button in it — which is what the
//! ~18 `ActionIcon`s over the ui-zoo editor actually need.

use super::*;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Ids {
    /// A `tabindex` button inside a `data-nofocus` toolbar.
    guarded_btn: usize,
    /// The toolbar's own blank chrome, outside any button.
    toolbar: usize,
    /// The same button shape, with no `data-nofocus` anywhere above it.
    plain_btn: usize,
    /// `data-nofocus` written directly on the focusable.
    self_guarded_btn: usize,
    /// `data-nofocus="false"` — the documented opt-out.
    opted_out_btn: usize,
    /// A text field inside the `data-nofocus` toolbar.
    toolbar_input: usize,
    /// An ordinary text field outside it.
    plain_input: usize,
    /// The editor's container.
    editor: usize,
}

fn mount_fixture() -> (RinchApp, Ids, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let click = |tag: &'static str, log: &Rc<RefCell<Vec<String>>>| {
        let log = log.clone();
        rinch_core::register_handler(Rc::new(move || {
            log.borrow_mut().push(tag.to_string());
        }))
    };
    let guarded_rid = click("guarded", &log);
    let plain_rid = click("plain", &log);
    let self_rid = click("self", &log);
    let opted_rid = click("opted", &log);
    let input_a = register_input_handler(InputCallback::new(|_| {}));
    let input_b = register_input_handler(InputCallback::new(|_| {}));

    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");

        // A button-shaped focusable with a click handler.
        let button = |scope: &mut RenderScope, rid: usize| {
            let n = scope.create_element("div");
            n.set_attribute("style", "width: 60px; height: 30px");
            n.set_attribute("tabindex", "0");
            n.set_attribute("data-rid", &rid.to_string());
            n
        };

        // The toolbar: `data-nofocus` once, on the container.
        let toolbar = scope.create_element("div");
        toolbar.set_attribute("style", "width: 400px; height: 60px");
        toolbar.set_attribute("data-nofocus", "");
        let guarded_btn = button(scope, guarded_rid.0);
        let toolbar_input = scope.create_element("input");
        toolbar_input.set_attribute("style", "width: 100px; height: 30px");
        toolbar_input.set_attribute("data-oninput", &input_a.0.to_string());
        toolbar.append_child(&guarded_btn);
        toolbar.append_child(&toolbar_input);

        let plain_btn = button(scope, plain_rid.0);
        let self_guarded_btn = button(scope, self_rid.0);
        self_guarded_btn.set_attribute("data-nofocus", "");
        let opted_out_btn = button(scope, opted_rid.0);
        opted_out_btn.set_attribute("data-nofocus", "false");
        let plain_input = scope.create_element("input");
        plain_input.set_attribute("style", "width: 200px; height: 30px");
        plain_input.set_attribute("data-oninput", &input_b.0.to_string());

        let (editor_container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>hello world</p>");
        editor_container.set_attribute("style", "width: 400px; height: 100px");

        root.append_child(&toolbar);
        root.append_child(&plain_btn);
        root.append_child(&self_guarded_btn);
        root.append_child(&opted_out_btn);
        root.append_child(&plain_input);
        root.append_child(&editor_container);

        ids_in.set(Some(Ids {
            guarded_btn: guarded_btn.node_id().0,
            toolbar: toolbar.node_id().0,
            plain_btn: plain_btn.node_id().0,
            self_guarded_btn: self_guarded_btn.node_id().0,
            opted_out_btn: opted_out_btn.node_id().0,
            toolbar_input: toolbar_input.node_id().0,
            plain_input: plain_input.node_id().0,
            editor: editor_container.node_id().0,
        }));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let ids = ids.get().expect("node ids captured at mount");
    (app, ids, log)
}

fn abs_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, id)
}

fn press_at(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

fn press(app: &mut RinchApp, id: usize) {
    let (x, y, w, h) = abs_box(app, id);
    press_at(app, x + w / 2.0, y + h / 2.0);
}

/// Put the arbiter on the editor by pressing inside it, the way a user would.
fn focus_editor(app: &mut RinchApp, ids: Ids) {
    press(app, ids.editor);
    assert_eq!(
        app.focus_target,
        FocusTarget::Editor(ids.editor),
        "the fixture's editor must actually take focus from a press"
    );
}

// ── 1. the reported defect ──────────────────────────────────────────────────

/// The issue's own test: editor focused, press a `tabindex` toolbar button →
/// the editor keeps the keyboard **and** the handler still fires.
#[test]
fn a_nofocus_button_takes_the_click_without_the_keyboard() {
    let (mut app, ids, log) = mount_fixture();
    focus_editor(&mut app, ids);

    press(&mut app, ids.guarded_btn);

    assert_eq!(
        app.focus_target,
        FocusTarget::Editor(ids.editor),
        "the editor still owns the keyboard, so Bold has a selection to act on"
    );
    assert_eq!(*log.borrow(), vec!["guarded".to_string()], "and it clicked");
}

/// The contrast that makes the test above meaningful: the identical button
/// with no `data-nofocus` above it does steal the editor's focus. This is the
/// default, and it is what every toolbar button will do once `<button>`
/// becomes focusable by tag (#252).
#[test]
fn without_the_attribute_the_same_button_steals_the_editor() {
    let (mut app, ids, log) = mount_fixture();
    focus_editor(&mut app, ids);

    press(&mut app, ids.plain_btn);

    assert_eq!(app.focus_target, FocusTarget::Node(ids.plain_btn));
    assert_eq!(*log.borrow(), vec!["plain".to_string()]);
}

/// The attribute works on the focusable itself, not only on a container.
#[test]
fn the_attribute_works_on_the_button_itself() {
    let (mut app, ids, log) = mount_fixture();
    focus_editor(&mut app, ids);

    press(&mut app, ids.self_guarded_btn);

    assert_eq!(app.focus_target, FocusTarget::Editor(ids.editor));
    assert_eq!(*log.borrow(), vec!["self".to_string()]);
}

/// Boolean-attribute rule, same as `disabled`: only the explicit `"false"`
/// opts out.
#[test]
fn the_false_value_opts_out() {
    let (mut app, ids, _log) = mount_fixture();
    focus_editor(&mut app, ids);

    press(&mut app, ids.opted_out_btn);

    assert_eq!(app.focus_target, FocusTarget::Node(ids.opted_out_btn));
}

/// The toolbar's blank chrome — between and around the buttons — must not blur
/// the editor either. A press there carries no `data-rid`, so before this the
/// editor-blur phase would have released it.
#[test]
fn the_blank_area_of_a_nofocus_toolbar_does_not_blur_the_editor() {
    let (mut app, ids, _log) = mount_fixture();
    focus_editor(&mut app, ids);

    // The toolbar's own right-hand edge, past every child.
    let (tx, ty, tw, th) = abs_box(&app, ids.toolbar);
    press_at(&mut app, tx + tw - 2.0, ty + th / 2.0);

    assert_eq!(app.focus_target, FocusTarget::Editor(ids.editor));
}

// ── 2. it protects every kind of claim, not just the editor's ───────────────

#[test]
fn it_protects_an_input_claim() {
    let (mut app, ids, _log) = mount_fixture();

    press(&mut app, ids.plain_input);
    assert_eq!(app.focused_input_node_id, Some(ids.plain_input));

    press(&mut app, ids.guarded_btn);

    assert_eq!(
        app.focus_target,
        FocusTarget::Input(ids.plain_input),
        "a text field keeps the caret while a toolbar button is pressed"
    );
    assert_eq!(app.focused_input_node_id, Some(ids.plain_input));
}

#[test]
fn it_protects_a_generic_node_claim() {
    let (mut app, ids, _log) = mount_fixture();

    press(&mut app, ids.plain_btn);
    assert_eq!(app.focus_target, FocusTarget::Node(ids.plain_btn));

    press(&mut app, ids.guarded_btn);

    assert_eq!(app.focus_target, FocusTarget::Node(ids.plain_btn));
}

// ── 3. it does not shield a text field inside it ────────────────────────────

/// An `<input>` *inside* a `data-nofocus` region still focuses normally — a
/// link-URL field in an editor toolbar has to be usable. The walk reaches the
/// field's own `data-oninput` before it can reach the toolbar's attribute, so
/// the more specific intent wins.
#[test]
fn a_text_field_inside_a_nofocus_region_still_focuses() {
    let (mut app, ids, _log) = mount_fixture();
    focus_editor(&mut app, ids);

    press(&mut app, ids.toolbar_input);

    assert_eq!(app.focus_target, FocusTarget::Input(ids.toolbar_input));
}

// ── 4. the resolution itself ────────────────────────────────────────────────

#[test]
fn the_press_resolution_reports_preserve() {
    let (app, ids, _log) = mount_fixture();
    let d = app.doc.as_ref().unwrap().borrow();

    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(ids.guarded_btn)),
        PressFocus::Preserve,
        "the attribute wins over the focusable it sits above"
    );
    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(ids.self_guarded_btn)),
        PressFocus::Preserve,
        "and over the focusable it sits on"
    );
    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(ids.plain_btn)),
        PressFocus::Node(ids.plain_btn)
    );
    assert_eq!(
        RinchApp::resolve_click_focus(&d.tree, Some(ids.toolbar_input)),
        PressFocus::Release,
        "a field is the text engine's, claimed on the click path"
    );
}
