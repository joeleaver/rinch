//! An element's focus can be observed, a parked focus is applied before the
//! next key, and `autofocus` is applied at mount (issue #2731).
//!
//! The handlers are written as their attributes (`data-onfocus`,
//! `data-onblur`, `data-onfocusin`, `data-onfocusout`) with
//! `register_handler`, which is exactly what `rsx!`'s `onfocus:` & co. emit
//! (pinned by the macro's own codegen tests), so the runtime half is tested
//! without depending on the macro half. One test at the end goes through
//! `rsx!` itself.
//!
//! The rule is Chrome's: `blur` on the old element, then `focusout` (which
//! bubbles), then `focus` on the new one, then `focusin` (which bubbles); an
//! element's `focus` fires only while its window has the keyboard — a window
//! losing it fires `blur` on the active element and regaining it fires
//! `focus` again, the active element unchanged throughout.
//!
//! Every geometry is declared: each control has an explicit box.

use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use rinch_core::Signal;
use rinch_core::dom::FocusIntoPolicy;
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

fn on(node: &NodeHandle, attr: &str, log: &Log, entry: &str) {
    let (log, entry) = (log.clone(), entry.to_string());
    let id = rinch_core::register_handler(Rc::new(move || log.borrow_mut().push(entry.clone())));
    node.set_attribute(attr, &id.0.to_string());
}

/// Every one of the four events on `node`, logged as `<event>:<name>`.
fn listen(node: &NodeHandle, name: &str, log: &Log) {
    for (attr, ev) in [
        ("data-onfocus", "focus"),
        ("data-onblur", "blur"),
        ("data-onfocusin", "focusin"),
        ("data-onfocusout", "focusout"),
    ] {
        on(node, attr, log, &format!("{ev}:{name}"));
    }
}

struct Page {
    app: RinchApp,
    a: NodeHandle,
    b: NodeHandle,
    button: NodeHandle,
    log: Log,
}

/// A `form` (a plain `div` listening to all four events) holding two text
/// fields `a` and `b` and a `button`, each listening to all four.
fn page() -> Page {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let handles: Rc<RefCell<Option<[NodeHandle; 4]>>> = Rc::new(RefCell::new(None));
    let (log_in, out) = (log.clone(), handles.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let form = scope.create_element("div");
        let mk = |scope: &mut RenderScope, tag: &str| {
            let n = scope.create_element(tag);
            n.set_attribute(
                "style",
                "display: block; width: 120px; height: 28px; border: 0; padding: 0",
            );
            form.append_child(&n);
            n
        };
        let a = mk(scope, "input");
        let b = mk(scope, "input");
        let button = mk(scope, "button");
        listen(&form, "form", &log_in);
        listen(&a, "a", &log_in);
        listen(&b, "b", &log_in);
        listen(&button, "button", &log_in);
        *out.borrow_mut() = Some([form.clone(), a, b, button]);
        form
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let [_form, a, b, button] = handles.borrow_mut().take().unwrap();
    Page {
        app,
        a,
        b,
        button,
        log,
    }
}

impl Page {
    fn event(&mut self, event: PlatformEvent) {
        self.app.handle_event(event, VP, 1.0);
    }

    /// One turn of the desktop loop with nothing else to do.
    fn turn(&mut self) {
        self.event(PlatformEvent::AboutToWait);
    }

    fn key(&mut self, key: KeyCode, text: Option<&str>, shift: bool) {
        let modifiers = Modifiers {
            shift,
            ..Modifiers::default()
        };
        self.event(PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        });
        self.event(PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        });
    }

    fn click(&mut self, node: &NodeHandle) {
        let (x, y) = {
            let d = self.app.doc.as_ref().unwrap().borrow();
            let (x, y, w, h) = painted_element_box(&d.tree, node.node_id().0);
            (x + w / 2.0, y + h / 2.0)
        };
        self.event(PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        });
        self.event(PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        });
    }

    fn active(&self) -> Option<u64> {
        self.a.active_element().map(|n| n.node_id().0)
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

// ── Part 1: the events ──────────────────────────────────────────────────────

/// A programmatic `focus()` fires `focus` on the element and `focusin` on it
/// and its ancestors; moving it on fires `blur`/`focusout` on the old element
/// first, then `focus`/`focusin` on the new — Chrome's order, measured in its
/// twin (`.scratch` page in the PR). `focus` and `blur` do not bubble: the
/// form hears only `focusin`/`focusout`.
///
/// **Mutant: firing the new element's `focus` before the old one's `blur`.**
#[test]
fn programmatic_focus_fires_blur_then_focus_in_chromes_order() {
    let mut p = page();
    p.take();
    p.a.focus();
    p.turn();
    assert_eq!(p.take(), ["focus:a", "focusin:a", "focusin:form"]);

    p.b.focus();
    p.turn();
    assert_eq!(
        p.take(),
        [
            "blur:a",
            "focusout:a",
            "focusout:form",
            "focus:b",
            "focusin:b",
            "focusin:form"
        ]
    );
    // Re-focusing the focused element fires nothing.
    p.b.focus();
    p.turn();
    assert_eq!(p.take(), Vec::<String>::new());
}

/// Tab moves the focus, and the events follow it.
#[test]
fn tab_fires_the_events() {
    let mut p = page();
    p.take();
    p.key(KeyCode::Tab, None, false);
    assert_eq!(p.active(), Some(p.a.node_id().0), "precondition: Tab to a");
    assert_eq!(p.take(), ["focus:a", "focusin:a", "focusin:form"]);
    p.key(KeyCode::Tab, None, false);
    assert_eq!(p.active(), Some(p.b.node_id().0));
    assert_eq!(
        p.take(),
        [
            "blur:a",
            "focusout:a",
            "focusout:form",
            "focus:b",
            "focusin:b",
            "focusin:form"
        ]
    );
}

/// A press moves the focus, and the events fire with the press — before its
/// release, as a browser's `mousedown` focuses before `click`.
#[test]
fn a_click_fires_the_events() {
    let mut p = page();
    p.take();
    let a = p.a.clone();
    p.click(&a);
    assert_eq!(p.take(), ["focus:a", "focusin:a", "focusin:form"]);
    let button = p.button.clone();
    p.click(&button);
    assert_eq!(p.active(), Some(p.button.node_id().0));
    assert_eq!(
        p.take(),
        [
            "blur:a",
            "focusout:a",
            "focusout:form",
            "focus:button",
            "focusin:button",
            "focusin:form"
        ]
    );
}

/// The window losing the keyboard fires `blur` on the active element and
/// regaining it fires `focus` again; the active element does not change.
///
/// **Mutant: not gating the events on the window's focus.**
#[test]
fn the_window_losing_and_regaining_focus_fires_blur_and_focus() {
    let mut p = page();
    p.a.focus();
    p.turn();
    p.take();
    p.event(PlatformEvent::WindowFocus(false));
    assert_eq!(p.take(), ["blur:a", "focusout:a", "focusout:form"]);
    assert_eq!(p.active(), Some(p.a.node_id().0), "the claim is kept");
    p.event(PlatformEvent::WindowFocus(true));
    assert_eq!(p.take(), ["focus:a", "focusin:a", "focusin:form"]);
}

/// Nothing fires while the window lacks the keyboard: a focus moved then is
/// announced when the window gets it back, once, for the element that has it.
#[test]
fn no_event_fires_while_the_window_lacks_focus() {
    let mut p = page();
    p.a.focus();
    p.turn();
    p.event(PlatformEvent::WindowFocus(false));
    p.take();
    p.b.focus();
    p.turn();
    let button = p.button.clone();
    p.click(&button);
    p.key(KeyCode::Tab, None, true);
    assert_eq!(p.take(), Vec::<String>::new(), "the window is not focused");
    let now = p.active().expect("something holds the claim");
    p.event(PlatformEvent::WindowFocus(true));
    let name = if now == p.b.node_id().0 { "b" } else { "?" };
    assert_eq!(now, p.b.node_id().0, "Shift+Tab from the button is b");
    assert_eq!(
        p.take(),
        [
            format!("focus:{name}"),
            format!("focusin:{name}"),
            "focusin:form".to_string()
        ]
    );
}

/// The shell starts a window unfocused: its element's `focus` waits for the
/// system's keyboard enter.
#[test]
fn a_window_not_yet_focused_fires_focus_at_its_keyboard_enter() {
    let mut p = page();
    p.app.window_focused = false;
    p.take();
    p.a.focus();
    p.turn();
    assert_eq!(p.take(), Vec::<String>::new());
    p.event(PlatformEvent::WindowFocus(true));
    assert_eq!(p.take(), ["focus:a", "focusin:a", "focusin:form"]);
}

/// Released focus fires `blur` with no `focus` after it.
#[test]
fn a_release_fires_blur_only() {
    let mut p = page();
    p.a.focus();
    p.turn();
    p.take();
    rinch_core::post_release_request(p.app.doc_key(), p.a.node_id().0);
    p.turn();
    assert_eq!(p.take(), ["blur:a", "focusout:a", "focusout:form"]);
    assert_eq!(p.active(), None);
}

/// The focused element leaving the document is sent `blur`, as Chrome 157
/// does on removal (measured in its twin), and nothing is focused after. (Its
/// `focusout` reaches only what is still its ancestor: rinch announces after
/// the removal, Chrome during it, so the form is not told.)
#[test]
fn removing_the_focused_element_fires_its_blur() {
    let mut p = page();
    p.a.focus();
    p.turn();
    p.take();
    p.a.remove();
    p.turn();
    assert_eq!(p.take(), ["blur:a", "focusout:a"]);
    p.b.focus();
    p.turn();
    assert_eq!(p.take(), ["focus:b", "focusin:b", "focusin:form"]);
}

/// `onfocus` / `onblur` / `onfocusin` / `onfocusout` written in `rsx!`.
#[test]
fn the_rsx_props_fire() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let field: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (l, out) = (log.clone(), field.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let (l1, l2, l3, l4) = (l.clone(), l.clone(), l.clone(), l.clone());
        let root = rsx! {
            div {
                onfocusin: move || l3.borrow_mut().push("focusin:div".into()),
                onfocusout: move || l4.borrow_mut().push("focusout:div".into()),
                input {
                    style: "display: block; width: 100px; height: 20px",
                    onfocus: move || l1.borrow_mut().push("focus:input".into()),
                    onblur: move || l2.borrow_mut().push("blur:input".into()),
                }
            }
        };
        *out.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    let input = field.borrow_mut().take().unwrap();
    input.focus();
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    rinch_core::post_release_request(app.doc_key(), input.node_id().0);
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert_eq!(
        *log.borrow(),
        ["focus:input", "focusin:div", "blur:input", "focusout:div"]
    );
}

// ── Part 2: a parked focus is applied before the window or a key hears of it ──

/// A `focus()` parked while the window had no keyboard is applied before the
/// app is told the window has it: an effect following `window_focused()`
/// sees the field focused at the moment it turns true. It saw nothing focused
/// until the turn's end (labwc, the issue's report).
#[test]
fn a_parked_focus_is_applied_before_window_focused_turns_true() {
    type Seen = Rc<RefCell<Vec<(bool, Option<u64>)>>>;
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let field: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (s, out) = (seen.clone(), field.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("style", "display: block; width: 100px; height: 20px");
        let probe = input.clone();
        scope.create_effect(move || {
            let focused = crate::window::window_focused();
            let active = probe.active_element().map(|n| n.node_id().0);
            s.borrow_mut().push((focused, active));
        });
        *out.borrow_mut() = Some(input.clone());
        input
    });
    app.set_publishes_window_state(true);
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.handle_event(PlatformEvent::WindowFocus(false), VP, 1.0);
    let input = field.borrow_mut().take().unwrap();
    input.focus();
    seen.borrow_mut().clear();
    app.handle_event(PlatformEvent::WindowFocus(true), VP, 1.0);
    assert_eq!(
        seen.borrow().first().copied(),
        Some((true, Some(input.node_id().0))),
        "{:?}",
        seen.borrow()
    );
}

/// A `focus()` made while the component is built is applied at mount, before
/// the first frame: the window cannot be given the keyboard before its field
/// has it.
#[test]
fn a_focus_made_while_building_is_applied_at_mount() {
    let field: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out = field.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let input = scope.create_element("input");
        input.set_attribute("style", "display: block; width: 100px; height: 20px");
        input.focus();
        *out.borrow_mut() = Some(input.clone());
        input
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    let input = field.borrow_mut().take().unwrap();
    assert_eq!(
        input.active_element().map(|n| n.node_id().0),
        Some(input.node_id().0)
    );
}

/// A key pressed right after an overlay opened, in the same turn — before the
/// loop's `AboutToWait` laid it out — lands in the field the overlay focused.
/// It went nowhere: `focus_into` waits for a layout, and the key's own drain
/// left it parked.
///
/// **Mutant: the key's drain not laying out first** (`drain_focus_requests(false)`).
#[test]
fn a_key_right_after_an_overlay_opens_is_not_lost() {
    let open = Signal::new(false);
    let typed: Log = Rc::new(RefCell::new(Vec::new()));
    let handles: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (t, out) = (typed.clone(), handles.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let t = t.clone();
        let root = rsx! {
            div {
                style: {move || if open.get() { "display: block" } else { "display: none" }},
                input {
                    style: "display: block; width: 100px; height: 20px",
                    oninput: move |v: String| t.borrow_mut().push(v),
                }
            }
        };
        *out.borrow_mut() = Some(root.clone());
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    let overlay = handles.borrow_mut().take().unwrap();
    // The overlay opens (a signal write, as from a timer or another window),
    // and asks for the focus as `Modal` does.
    open.set(true);
    overlay.focus_into(FocusIntoPolicy::FirstFocusable);
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyX,
            logical_key: None,
            text: Some("x".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    assert_eq!(*typed.borrow(), ["x"], "the key reached the field");
}

// ── Part 3: autofocus at mount ──────────────────────────────────────────────

fn autofocus_page(which: &[usize], explicit: Option<usize>) -> (RinchApp, Vec<u64>) {
    let ids: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let out = ids.clone();
    let which = which.to_vec();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let mut nodes = Vec::new();
        for i in 0..3 {
            let input = scope.create_element("input");
            input.set_attribute("style", "display: block; width: 100px; height: 20px");
            if which.contains(&i) {
                input.set_attribute("autofocus", "");
            }
            root.append_child(&input);
            out.borrow_mut().push(input.node_id().0);
            nodes.push(input);
        }
        if let Some(i) = explicit {
            nodes[i].focus();
        }
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    // The first turn of the loop, as before the first frame.
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    let ids = ids.borrow().clone();
    (app, ids)
}

fn active_of(app: &RinchApp) -> Option<u64> {
    let doc = app.doc.as_ref().unwrap().borrow();
    rinch_core::dom::DomDocument::active_element(&*doc).map(|n| n.0)
}

/// The first `autofocus` element in tree order takes the focus when the
/// document mounts, as Chrome's does when the document loads.
///
/// Off the fixed point: the autofocus elements are the second and third, not
/// the first focusable.
#[test]
fn the_first_autofocus_element_is_focused_at_mount() {
    let (app, ids) = autofocus_page(&[1, 2], None);
    assert_eq!(active_of(&app), Some(ids[1]));
}

/// No `autofocus`, no focus.
#[test]
fn without_autofocus_nothing_is_focused_at_mount() {
    let (app, _) = autofocus_page(&[], None);
    assert_eq!(active_of(&app), None);
}

/// A `focus()` the app made while building wins over `autofocus`, as in
/// Chrome, where autofocus is skipped once something has the focus.
#[test]
fn an_explicit_focus_wins_over_autofocus() {
    let (app, ids) = autofocus_page(&[1], Some(2));
    assert_eq!(active_of(&app), Some(ids[2]));
}

/// A key typed before the loop's first turn still finds the autofocus field.
#[test]
fn a_key_before_the_first_turn_reaches_the_autofocus_field() {
    let typed: Log = Rc::new(RefCell::new(Vec::new()));
    let t = typed.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let t = t.clone();
        rsx! {
            div {
                input { style: "display: block; width: 100px; height: 20px" }
                input {
                    style: "display: block; width: 100px; height: 20px",
                    autofocus: true,
                    oninput: move |v: String| t.borrow_mut().push(v),
                }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyQ,
            logical_key: None,
            text: Some("q".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    assert_eq!(*typed.borrow(), ["q"]);
}

/// A registered focus target focused before its window has the keyboard
/// hears `on_focus_gained` once, at the keyboard enter — not once from the
/// drain and again from the window's arm (review of #2731) — and nothing for
/// a move made while the window is in the background.
#[test]
fn a_registered_target_hears_one_gain_at_the_keyboard_enter() {
    use crate::focus_registry::{FocusEntry, register_focus_target};
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let handles: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let (l, out) = (log.clone(), handles.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let mut nodes = Vec::new();
        for name in ["one", "two"] {
            let n = scope.create_element("div");
            n.set_attribute("tabindex", "0");
            n.set_attribute("style", "display: block; width: 100px; height: 20px");
            let (g, lo) = (l.clone(), l.clone());
            register_focus_target(
                &n,
                FocusEntry::new()
                    .on_focus_gained(move || g.borrow_mut().push(format!("gained:{name}")))
                    .on_focus_lost(move || lo.borrow_mut().push(format!("lost:{name}"))),
            );
            root.append_child(&n);
            nodes.push(n);
        }
        nodes[0].focus();
        *out.borrow_mut() = Some((nodes[0].clone(), nodes[1].clone()));
        root
    });
    // As the shell builds it: not focused until the system says so.
    app.window_focused = false;
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert_eq!(
        *log.borrow(),
        Vec::<String>::new(),
        "no gain before the enter"
    );
    app.handle_event(PlatformEvent::WindowFocus(true), VP, 1.0);
    assert_eq!(*log.borrow(), ["gained:one"]);
    log.borrow_mut().clear();
    app.handle_event(PlatformEvent::WindowFocus(false), VP, 1.0);
    let (_, two) = handles.borrow_mut().take().unwrap();
    two.focus();
    app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    app.handle_event(PlatformEvent::WindowFocus(true), VP, 1.0);
    assert_eq!(*log.borrow(), ["lost:one", "gained:two"]);
}
