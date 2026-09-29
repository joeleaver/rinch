//! More shapes for issue #452 (a right or middle press runs the press focus
//! claim), from the review of PR #1140.
//!
//! The claim must not reach a press that another owner places: an editor's
//! (whatever the button — a middle press used to hand a focused editor's
//! keyboard to a focusable wrapper around it) or an open `<select>` popup's
//! (modal: a right press on an option moved `:focus` into the popup while the
//! arbiter stayed on the select). And on the `data-oncontextmenu` path the
//! claim is the only release there is: `handle_click_with_button`'s release
//! check never runs there.
use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use rinch_core::Component;
use rinch_core::events::{InputCallback, register_input_handler};
use std::cell::{Cell, RefCell};

const VP: (u32, u32) = (800, 600);

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}
fn press_at(app: &mut RinchApp, x: f32, y: f32, button: MouseButton) {
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}
fn center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, w, h) = painted_element_box(&d.tree, id);
    (ax + w / 2.0, ay + h / 2.0)
}
fn dom_focus(app: &RinchApp) -> Option<usize> {
    app.doc.as_ref().unwrap().borrow().tree.focused_node
}

#[cfg(feature = "desktop")]
type Slot = Option<(usize, crate::editor::EditorHandle)>;

#[cfg(feature = "desktop")]
/// An editor, optionally inside a registered `tabindex="0"` card.
fn editor_app(
    wrapped: bool,
) -> (
    RinchApp,
    usize,
    crate::editor::EditorHandle,
    Rc<RefCell<Vec<String>>>,
) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let (log_in, slot_in) = (log.clone(), slot.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let card = scope.create_element("div");
        card.set_attribute("style", "margin-left: 23px; margin-top: 17px; width: 450px");
        if wrapped {
            card.set_attribute("tabindex", "0");
            register_focus_target(
                &card,
                FocusEntry::new()
                    .on_focus_gained({
                        let log = log_in.clone();
                        move || log.borrow_mut().push("card:gained".into())
                    })
                    .on_focus_lost({
                        let log = log_in.clone();
                        move || log.borrow_mut().push("card:lost".into())
                    }),
            );
        }
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>hello world</p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        card.append_child(&container);
        root.append_child(&card);
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().unwrap();
    (app, container, handle, log)
}

#[cfg(feature = "desktop")]
fn editor_point(app: &mut RinchApp, handle: &crate::editor::EditorHandle) -> (f32, f32) {
    let (cx, cy, ch) = app
        .editor_caret_point(handle, rinch_editor_core::Pos(4))
        .expect("caret");
    (cx + 1.0, cy + ch / 2.0)
}

/// S1: middle press in a focused editor inside a focusable card.
#[cfg(feature = "desktop")]
#[test]
fn s1_middle_press_in_editor_inside_card() {
    let (mut app, container, handle, log) = editor_app(true);
    let (x, y) = editor_point(&mut app, &handle);
    press_at(&mut app, x, y, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    log.borrow_mut().clear();
    press_at(&mut app, x, y, MouseButton::Middle);
    assert_eq!(
        app.focus_target,
        FocusTarget::Editor(container),
        "S1 target"
    );
    assert!(log.borrow().is_empty(), "S1 log {:?}", log.borrow());
}

/// S2: middle press in a focused bare editor.
#[cfg(feature = "desktop")]
#[test]
fn s2_middle_press_in_bare_editor() {
    let (mut app, container, handle, _log) = editor_app(false);
    let (x, y) = editor_point(&mut app, &handle);
    press_at(&mut app, x, y, MouseButton::Left);
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    let before = dom_focus(&app);
    press_at(&mut app, x, y, MouseButton::Middle);
    assert_eq!(
        app.focus_target,
        FocusTarget::Editor(container),
        "S2 target"
    );
    assert_eq!(
        dom_focus(&app),
        before,
        "S2: :focus did not move into the editor's own nodes"
    );
}

/// S1b: a right press in a focused editor inside a focusable card stays the
/// editor's.
#[cfg(feature = "desktop")]
#[test]
fn s1b_right_press_in_editor_inside_card() {
    let (mut app, container, handle, log) = editor_app(true);
    let (x, y) = editor_point(&mut app, &handle);
    press_at(&mut app, x, y, MouseButton::Left);
    log.borrow_mut().clear();
    press_at(&mut app, x, y, MouseButton::Right);
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    assert!(
        log.borrow().is_empty(),
        "the card was never focused: {:?}",
        log.borrow()
    );
}

struct Page {
    app: RinchApp,
    input: usize,
    plain: usize,
    listbox: usize,
    disabled_btn: usize,
    log: Rc<RefCell<Vec<String>>>,
}

fn page() -> Page {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<[usize; 4]>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let changed = register_input_handler(InputCallback::new({
        let log = log.clone();
        move |v: String| log.borrow_mut().push(format!("change:{v}"))
    }));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let input = scope.create_element("input");
        input.set_attribute(
            "style",
            "position: absolute; left: 31px; top: 27px; width: 200px; height: 30px; font-size: 16px; line-height: 20px",
        );
        input.set_attribute("value", "abcdef");
        input.set_attribute("data-onchange", &changed.0.to_string());
        input.set_attribute("data-oninput", &changed.0.to_string());
        let plain = scope.create_element("div");
        plain.set_attribute(
            "style",
            "position: absolute; left: 413px; top: 257px; width: 130px; height: 37px",
        );
        let listbox = scope.create_element("div");
        listbox.set_attribute(
            "style",
            "position: absolute; left: 37px; top: 123px; width: 310px; height: 90px",
        );
        listbox.set_attribute("tabindex", "0");
        register_focus_target(
            &listbox,
            FocusEntry::new()
                .on_focus_gained({
                    let log = log_in.clone();
                    move || log.borrow_mut().push("listbox:gained".into())
                })
                .on_focus_lost({
                    let log = log_in.clone();
                    move || log.borrow_mut().push("listbox:lost".into())
                }),
        );
        let btn = scope.create_element("button");
        btn.set_attribute("style", "margin-left: 13px; width: 120px; height: 31px");
        btn.set_attribute("disabled", "");
        listbox.append_child(&btn);
        root.append_child(&input);
        root.append_child(&plain);
        root.append_child(&listbox);
        ids_in.set(Some([
            input.node_id().0,
            plain.node_id().0,
            listbox.node_id().0,
            btn.node_id().0,
        ]));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let [input, plain, listbox, disabled_btn] = ids.get().unwrap();
    Page {
        app,
        input,
        plain,
        listbox,
        disabled_btn,
        log,
    }
}

/// S3: a focused input, then a right press on a plain area outside it.
#[test]
fn s3_right_press_outside_a_focused_input() {
    let mut p = page();
    let (x, y) = center(&p.app, p.input);
    press_at(&mut p.app, x, y, MouseButton::Left);
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
    let (px, py) = center(&p.app, p.plain);
    press_at(&mut p.app, px, py, MouseButton::Right);
    // Arbiter and :focus should agree about who owns the keyboard.
    let arbiter_on_input = p.app.focus_target == FocusTarget::Input(p.input);
    let dom_on_input = dom_focus(&p.app) == Some(p.input);
    assert_eq!(
        arbiter_on_input, dom_on_input,
        "S3: arbiter and :focus disagree"
    );
}

/// S3m: the same with a middle press.
#[test]
fn s3m_middle_press_outside_a_focused_input() {
    let mut p = page();
    let (x, y) = center(&p.app, p.input);
    press_at(&mut p.app, x, y, MouseButton::Left);
    let (px, py) = center(&p.app, p.plain);
    press_at(&mut p.app, px, py, MouseButton::Middle);
    let arbiter_on_input = p.app.focus_target == FocusTarget::Input(p.input);
    let dom_on_input = dom_focus(&p.app) == Some(p.input);
    assert_eq!(
        arbiter_on_input, dom_on_input,
        "S3m: arbiter and :focus disagree"
    );
}

/// S4: listbox focused, right press on the text input → menu, input claim, one lost.
#[test]
fn s4_right_press_on_input_from_a_focused_node() {
    let mut p = page();
    let (lx, ly) = center(&p.app, p.listbox);
    press_at(&mut p.app, lx + 100.0, ly, MouseButton::Left);
    assert_eq!(p.app.focus_target, FocusTarget::Node(p.listbox));
    p.log.borrow_mut().clear();
    let (x, y) = center(&p.app, p.input);
    ev(
        &mut p.app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Right,
        },
    );
    assert_eq!(p.app.focus_target, FocusTarget::Input(p.input));
    assert!(p.app.is_text_context_menu_open());
    assert_eq!(*p.log.borrow(), vec!["listbox:lost".to_string()]);
}

/// S5: right press on a disabled button inside the listbox.
#[test]
fn s5_right_press_on_a_disabled_child() {
    let mut p = page();
    let (x, y) = center(&p.app, p.disabled_btn);
    press_at(&mut p.app, x, y, MouseButton::Right);
    assert_eq!(p.app.focus_target, FocusTarget::Node(p.listbox));
    assert_eq!(dom_focus(&p.app), Some(p.listbox));
}

/// S6: a render surface inside a focusable wrapper; right vs left announce.
fn surface_app() -> (RinchApp, usize, Rc<RefCell<Vec<String>>>) {
    use crate::render_surface::create_render_surface;
    let surface = create_render_surface();
    let sid = surface.id();
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log_in = log.clone();
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrap = scope.create_element("div");
        wrap.set_attribute("tabindex", "0");
        wrap.set_attribute(
            "style",
            "margin-left: 19px; margin-top: 23px; width: 300px; height: 200px",
        );
        register_focus_target(
            &wrap,
            FocusEntry::new()
                .on_focus_gained({
                    let log = log_in.clone();
                    move || log.borrow_mut().push("wrap:gained".into())
                })
                .on_focus_lost({
                    let log = log_in.clone();
                    move || log.borrow_mut().push("wrap:lost".into())
                }),
        );
        let child = crate::render_surface::RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        child.set_attribute("style", "width: 200px; height: 100px");
        wrap.append_child(&child);
        root.append_child(&wrap);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let _ = surface;
    (app, sid, log)
}

#[test]
fn s6_right_press_on_a_surface_in_a_focusable() {
    // A right press takes the surface exactly as a left press does, wrapper
    // announcements included (both run the same claim before phase 0).
    let mut logs = Vec::new();
    for button in [MouseButton::Left, MouseButton::Right] {
        let (mut app, sid, log) = surface_app();
        press_at(&mut app, 60.0, 60.0, button);
        assert_eq!(app.focus_target, FocusTarget::Surface(sid), "{button:?}");
        logs.push(log.borrow().clone());
    }
    assert_eq!(logs[0], logs[1], "left and right announce alike");
}

/// S7: a focused node, then a right press on a non-focusable node that carries
/// a `data-oncontextmenu`: the claim must be released (Chrome: body), and the
/// release check in `handle_click_with_button` never runs on this path.
#[test]
fn s7_right_press_on_oncontextmenu_outside_releases() {
    let mut p = page();
    let ran = Rc::new(Cell::new(0));
    let rid = rinch_core::register_handler(Rc::new({
        let ran = ran.clone();
        move || ran.set(ran.get() + 1)
    }));
    {
        let doc = p.app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        rinch_core::dom::DomDocument::set_attribute(
            &mut *d,
            rinch_core::dom::NodeId(p.plain),
            "data-oncontextmenu",
            &rid.0.to_string(),
        );
    }
    p.app.resolve_and_repaint(801.0, 600.0);
    let (lx, ly) = center(&p.app, p.listbox);
    press_at(&mut p.app, lx + 100.0, ly, MouseButton::Left);
    assert_eq!(p.app.focus_target, FocusTarget::Node(p.listbox));
    let (x, y) = center(&p.app, p.plain);
    press_at(&mut p.app, x, y, MouseButton::Right);
    assert_eq!(
        ran.get(),
        1,
        "positive control: the contextmenu handler ran"
    );
    assert_eq!(p.app.focus_target, FocusTarget::None, "S7");
}

/// S8: a `<select>` popup open, a right press on an option.
#[test]
fn s8_right_press_in_an_open_select_popup() {
    let ids: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let sel = scope.create_element("select");
        sel.set_attribute("style", "margin-left: 31px; margin-top: 19px; width: 150px; height: 28px; font-size: 16px; line-height: 20px");
        for v in ["alpha", "bravo", "charlie"] {
            let o = scope.create_element("option");
            o.set_attribute("value", v);
            let t = scope.create_text(v);
            o.append_child(&t);
            sel.append_child(&o);
        }
        root.append_child(&sel);
        ids_in.set(Some(sel.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let sel = ids.get().unwrap();
    let (x, y) = center(&app, sel);
    press_at(&mut app, x, y, MouseButton::Left);
    assert!(app.is_select_open());
    let before_focus = dom_focus(&app);
    let pt = {
        let open = app.open_select.as_ref().unwrap();
        let b = open.option_ids[1];
        center(&app, b)
    };
    press_at(&mut app, pt.0, pt.1, MouseButton::Right);
    assert!(
        app.is_select_open(),
        "a right press picks nothing and leaves the list open (#1111)"
    );
    assert_eq!(app.focus_target, FocusTarget::Select(sel));
    assert_eq!(
        dom_focus(&app),
        before_focus,
        "S8: :focus stays on the select, not in its popup"
    );
}
