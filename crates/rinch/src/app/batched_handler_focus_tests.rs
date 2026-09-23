//! A handler that writes an overlay's open signal and then focuses a node
//! itself (PR #882 review, D1).
//!
//! Every handler is a batch, so the overlay's `trap_focus` effect — which
//! parks a `focus_into` / `restore_focus` request in the single focus slot —
//! runs after the handler's body. Without the flush-on-DOM-access rule
//! (`NodeHandle::accessed_doc` → `flush_pending_effects`) it ran *after* the
//! handler's own `focus()` and overwrote it. With it, `focus()` first runs the
//! queued overlay effect and then writes its request, which is program order.
//! The two `control_unbatched` fixtures are the pre-batching behaviour.
use super::*;
use std::cell::RefCell;

use rinch_components::Modal;
use rinch_core::{Component, Signal};

const W: f32 = 800.0;
const H: f32 = 600.0;

fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(W, H);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(W, H);
    app
}

fn settle(app: &mut RinchApp) {
    app.handle_event(
        PlatformEvent::UserEvent(UserEvent::ReRender),
        (W as u32, H as u32),
        1.0,
    );
}

fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

fn button(scope: &mut RenderScope, id: &str) -> NodeHandle {
    let b = scope.create_element("button");
    b.set_attribute("id", id);
    b.set_attribute("style", "display: block; width: 120px; height: 28px");
    b
}

struct Handles {
    opener: NodeHandle,
    elsewhere: NodeHandle,
    in_first: NodeHandle,
    in_last: NodeHandle,
}

fn page(open: Signal<bool>) -> (RinchApp, Rc<RefCell<Option<Handles>>>) {
    let out: Rc<RefCell<Option<Handles>>> = Rc::new(RefCell::new(None));
    let o = out.clone();
    let app = mount(move |scope: &mut RenderScope| {
        let page = scope.create_element("div");
        let opener = button(scope, "opener");
        page.append_child(&opener);
        let in_first = button(scope, "in-first");
        let in_last = button(scope, "in-last");
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[in_first.clone(), in_last.clone()]);
        page.append_child(&modal);
        let elsewhere = button(scope, "elsewhere");
        page.append_child(&elsewhere);
        *o.borrow_mut() = Some(Handles {
            opener,
            elsewhere,
            in_first,
            in_last,
        });
        page
    });
    (app, out)
}

/// "Open the dialog and focus its second field" from one click handler.
#[test]
fn open_then_focus_a_specific_field_in_one_handler() {
    let open = Signal::new(false);
    let (mut app, h) = page(open);
    let hb = h.borrow();
    let h = hb.as_ref().unwrap();
    app.focus_element(h.opener.node_id().0);
    let target = h.in_last.clone();
    let id = rinch_core::events::register_handler(Rc::new(move || {
        open.set(true);
        target.focus();
    }));
    assert!(rinch_core::events::dispatch_event(id));
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(h.in_last.node_id().0),
        "the handler's explicit focus() must win; in_first={} opener={}",
        h.in_first.node_id().0,
        h.opener.node_id().0
    );
}

/// "Close the dialog and focus the search box" from one click handler.
#[test]
fn close_then_focus_elsewhere_in_one_handler() {
    let open = Signal::new(false);
    let (mut app, h) = page(open);
    let hb = h.borrow();
    let h = hb.as_ref().unwrap();
    app.focus_element(h.opener.node_id().0);
    open.set(true);
    settle(&mut app);
    assert_eq!(focused(&app), Some(h.in_first.node_id().0), "precondition");
    let target = h.elsewhere.clone();
    let id = rinch_core::events::register_handler(Rc::new(move || {
        open.set(false);
        target.focus();
    }));
    assert!(rinch_core::events::dispatch_event(id));
    settle(&mut app);
    assert_eq!(
        focused(&app),
        Some(h.elsewhere.node_id().0),
        "the handler's explicit focus() must win over the close's restore; opener={}",
        h.opener.node_id().0
    );
}

/// Control: the same handler body run unbatched (what `dispatch_event` did
/// before #882).
#[test]
fn control_unbatched_open_then_focus() {
    let open = Signal::new(false);
    let (mut app, h) = page(open);
    let hb = h.borrow();
    let h = hb.as_ref().unwrap();
    app.focus_element(h.opener.node_id().0);
    open.set(true);
    h.in_last.focus();
    settle(&mut app);
    assert_eq!(focused(&app), Some(h.in_last.node_id().0));
}

#[test]
fn control_unbatched_close_then_focus() {
    let open = Signal::new(false);
    let (mut app, h) = page(open);
    let hb = h.borrow();
    let h = hb.as_ref().unwrap();
    app.focus_element(h.opener.node_id().0);
    open.set(true);
    settle(&mut app);
    open.set(false);
    h.elsewhere.focus();
    settle(&mut app);
    assert_eq!(focused(&app), Some(h.elsewhere.node_id().0));
}
