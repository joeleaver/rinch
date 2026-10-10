//! Round-2 review fixtures for PR #1514 (#1509).

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target};
use rinch_core::dom::{DomDocument, NodeHandle};
use std::cell::Cell;

type Log = Rc<RefCell<Vec<String>>>;

/// A registered `tabindex` widget under a host; `lost`/`ime` choose which
/// callbacks the entry carries. Clicked so it holds the claim.
fn app_with_focused_widget(lost: bool, ime: bool) -> (RinchApp, Log, usize, usize) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let host = scope.create_element("div");
        let w = scope.create_element("div");
        w.set_attribute("style", "width: 200px; height: 40px");
        w.set_attribute("tabindex", "0");
        let other = scope.create_element("div");
        other.set_attribute("style", "width: 200px; height: 40px");
        other.set_attribute("tabindex", "0");
        let (l1, l2, l3) = (log_in.clone(), log_in.clone(), log_in.clone());
        let mut e = FocusEntry::new()
            .on_focus_gained(move || l1.borrow_mut().push("gained".into()))
            .on_key(move |k| {
                l3.borrow_mut().push(format!("key:{}", k.key));
                true
            });
        if lost {
            e = e.on_focus_lost(move || l2.borrow_mut().push("lost".into()));
        }
        if ime {
            e = e.on_ime(|_| {});
        }
        register_focus_target(&w, e);
        host.append_child(&w);
        root.append_child(&host);
        root.append_child(&other);
        ids_in.set(Some((host.node_id().0, w.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (host_id, w_id) = ids.get().unwrap();
    click_node(&mut app, w_id);
    assert_eq!(app.focus_target, FocusTarget::Node(w_id), "precondition");
    (app, log, host_id, w_id)
}

fn center(app: &RinchApp, id: usize) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (ax, ay, aw, ah) = painted_element_box(&d.tree, id);
    (ax + aw / 2.0, ay + ah / 2.0)
}

fn click_node(app: &mut RinchApp, id: usize) {
    let (x, y) = center(app, id);
    for ev in [
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
        app.handle_event(ev, (800, 600), 1.0);
    }
}

fn free(app: &mut RinchApp, host_id: usize, markup: &str) -> usize {
    let doc: Rc<RefCell<RinchDocument>> = app.doc.clone().unwrap();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc;
    let host = NodeHandle::new(rinch_core::dom::NodeId(host_id), Rc::downgrade(&dyn_doc));
    host.set_inner_html(markup);
    app.resolve_and_repaint(800.0, 600.0);
    dyn_doc
        .borrow()
        .get_children(host.node_id())
        .first()
        .map(|n| n.0)
        .unwrap_or(0)
}

fn key(app: &mut RinchApp, k: KeyCode) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key: k,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

/// S1: R3b with a target that registered no `on_focus_lost` (on_key + on_ime
/// only). The claim must still be dropped, not handed to the re-minted node.
#[test]
fn s1_reminted_claim_dropped_without_on_focus_lost() {
    let (mut app, log, host, w) = app_with_focused_widget(false, true);
    let first = free(&mut app, host, r#"<div tabindex="0">n</div>"#);
    assert_eq!(first, w, "precondition: the markup re-minted the id");
    key(&mut app, KeyCode::Enter);
    let l = log.borrow().clone();
    assert!(!l.iter().any(|e| e.starts_with("key:")), "{l:?}");
    assert_ne!(
        app.focus_target,
        FocusTarget::Node(w),
        "claim handed to the re-minted node: {l:?}"
    );
}

/// S2: no input ever comes. The next `AboutToWait` releases the freed claim,
/// so a widget's `focused` state does not stay set until a key, Tab or click.
#[test]
fn s2_about_to_wait_releases_a_freed_claim() {
    let (mut app, log, host, _w) = app_with_focused_widget(true, true);
    free(&mut app, host, "");
    assert!(
        app.has_focused_node(),
        "precondition: the claim outlives the free"
    );
    let _ = app.handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    assert!(
        !app.has_focused_node(),
        "claim still held after AboutToWait"
    );
    assert_eq!(app.focus_target, FocusTarget::None);
    assert_eq!(*log.borrow(), ["gained", "lost"]);
}

/// S3: an IME text target freed while focused: the platform IME is off.
#[test]
fn s3_ime_off_for_a_freed_text_target() {
    let (mut app, _log, host, _w) = app_with_focused_widget(true, true);
    assert!(app.ime_state().enabled, "precondition: IME on");
    free(&mut app, host, "");
    assert!(!app.ime_state().enabled, "IME left on for a freed target");
}

/// S4: two documents. App A's widget (id N) holds A's claim; app B frees its own
/// registered, focused widget on the same id N. A's claim is untouched and keys
/// reach A's on_key; B's claim is released with on_focus_lost.
#[test]
fn s4_two_documents() {
    let (mut a, la, _ha, wa) = app_with_focused_widget(true, false);
    let (mut b, lb, hb, wb) = app_with_focused_widget(true, false);
    assert_eq!(wa, wb, "precondition: same numeric id");
    assert_ne!(a.doc_key(), b.doc_key());
    free(&mut b, hb, "");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.focus_target, FocusTarget::Node(wa));
    assert!(
        la.borrow().iter().any(|e| e == "key:Enter"),
        "{:?}",
        la.borrow()
    );
    assert!(!la.borrow().iter().any(|e| e == "lost"));
    key(&mut b, KeyCode::Enter);
    assert_eq!(b.focus_target, FocusTarget::None);
    assert!(lb.borrow().iter().any(|e| e == "lost"), "{:?}", lb.borrow());
}

/// S5: the app (document) goes away while a freed claim is parked: its claim
/// record and the parked entry (with its captures) go with it.
#[test]
fn s5_parked_entry_after_the_app_is_dropped() {
    let (mut app, _log, host, w) = app_with_focused_widget(true, false);
    let dk = app.doc_key();
    free(&mut app, host, "");
    assert!(
        crate::focus_registry::was_freed(dk, w),
        "precondition: parked"
    );
    assert_eq!(crate::focus_registry::claimed_for_tests(dk), Some(w));
    drop(app);
    assert!(
        !crate::focus_registry::was_freed(dk, w),
        "parked entry outlived its app"
    );
    assert_eq!(
        crate::focus_registry::claimed_for_tests(dk),
        None,
        "claim record outlived its app"
    );
}

/// S6 (round-2 finding 4): only the freed target holding the claim is parked.
/// A registered target freed while unfocused is dropped outright, so a node
/// the markup re-mints on its id and the user then focuses keeps its claim,
/// and the old component hears nothing.
#[test]
fn s6_an_unfocused_freed_target_is_not_parked() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let host = scope.create_element("div");
        let w = scope.create_element("div");
        w.set_attribute("style", "width: 200px; height: 40px");
        w.set_attribute("tabindex", "0");
        let l = log_in.clone();
        register_focus_target(
            &w,
            FocusEntry::new().on_focus_lost(move || l.borrow_mut().push("old-lost".into())),
        );
        host.append_child(&w);
        root.append_child(&host);
        ids_in.set(Some((host.node_id().0, w.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (host, w) = ids.get().unwrap();
    assert_eq!(
        app.focus_target,
        FocusTarget::None,
        "precondition: unfocused"
    );
    let first = free(
        &mut app,
        host,
        r#"<div tabindex="0" style="width: 200px; height: 40px">n</div>"#,
    );
    assert_eq!(first, w, "precondition: the markup re-minted the id");
    click_node(&mut app, w);
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(w),
        "precondition: new node focused"
    );
    let _ = app.handle_event(PlatformEvent::AboutToWait, (800, 600), 1.0);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(w),
        "the new node's claim was dropped"
    );
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
}
