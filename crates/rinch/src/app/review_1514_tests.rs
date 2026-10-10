//! Review fixtures for PR #1514 (#1509): two documents, the host node, and a
//! focused target freed while its component is alive or disposed.

use super::*;
use crate::focus_registry::{FocusEntry, register_focus_target, wants_key_routing};
use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_core::reactive::Scope;
use rinch_dom::RinchDocument;
use std::cell::Cell;

fn el(doc: &Rc<RefCell<dyn DomDocument>>, tag: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_element(tag);
    NodeHandle::new(id, Rc::downgrade(doc))
}

fn new_doc() -> (Rc<RefCell<dyn DomDocument>>, NodeHandle) {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc));
    (doc, body)
}

/// R1 (#134): two documents on one thread with the same numeric node ids.
/// Document A's `set_inner_html` frees A's id N; document B's target on its own
/// id N must stay registered.
#[test]
fn r1_another_documents_set_inner_html_keeps_this_documents_target() {
    let (da, ba) = new_doc();
    let (db, bb) = new_doc();
    let ha = el(&da, "div");
    ba.append_child(&ha);
    let a = el(&da, "div");
    ha.append_child(&a);
    let hb = el(&db, "div");
    bb.append_child(&hb);
    let b = el(&db, "div");
    hb.append_child(&b);
    assert_eq!(a.node_id(), b.node_id(), "precondition: same numeric id");
    assert_ne!(a.doc_key(), b.doc_key(), "precondition: two documents");
    let s = Scope::new();
    s.run(|| register_focus_target(&b, FocusEntry::new().on_key(|_| true)));
    ha.set_inner_html("");
    assert!(
        wants_key_routing(b.doc_key(), b.node_id().0),
        "A's set_inner_html dropped B's target"
    );
    s.dispose();
}

/// R2: the node `set_inner_html` is called on survives; its registration must too.
#[test]
fn r2_the_node_whose_children_are_replaced_stays_registered() {
    let (d, body) = new_doc();
    let host = el(&d, "div");
    body.append_child(&host);
    let c = el(&d, "span");
    host.append_child(&c);
    let s = Scope::new();
    s.run(|| register_focus_target(&host, FocusEntry::new().on_key(|_| true)));
    host.set_inner_html("<p>x</p>");
    assert!(
        wants_key_routing(host.doc_key(), host.node_id().0),
        "the host itself lost its registration"
    );
    s.dispose();
}

/// R3: the freed node is the FOCUSED target. Pin what the arbiter does: the
/// claim is released at the next key, and no key reaches the old `on_key`.
/// Records whether `on_focus_lost` fires. With `dispose`, the target is
/// registered in a scope of its own that is disposed after the free and
/// before the key.
fn focused_target_freed(markup: &str, dispose: bool) -> (Vec<String>, FocusTarget, usize, usize) {
    let own: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
    let own_in = own.clone();
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let host = scope.create_element("div");
        let w = scope.create_element("div");
        w.set_attribute("style", "width: 200px; height: 40px");
        w.set_attribute("tabindex", "0");
        let l1 = log_in.clone();
        let l2 = log_in.clone();
        let l3 = log_in.clone();
        let entry = FocusEntry::new()
            .on_focus_gained(move || l1.borrow_mut().push("gained".into()))
            .on_focus_lost(move || l2.borrow_mut().push("lost".into()))
            .on_key(move |k| {
                l3.borrow_mut().push(format!("key:{}", k.key));
                true
            });
        if dispose {
            let s = Scope::new();
            s.run(|| register_focus_target(&w, entry));
            *own_in.borrow_mut() = Some(s);
        } else {
            register_focus_target(&w, entry);
        }
        host.append_child(&w);
        root.append_child(&host);
        ids_in.set(Some((host.node_id().0, w.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (host_id, w_id) = ids.get().unwrap();
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, aw, ah) = painted_element_box(&d.tree, w_id);
        (ax + aw / 2.0, ay + ah / 2.0)
    };
    for down in [true, false] {
        let ev = if down {
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            }
        } else {
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            }
        };
        app.handle_event(ev, (800, 600), 1.0);
    }
    assert_eq!(
        app.focus_target,
        FocusTarget::Node(w_id),
        "precondition: focused"
    );
    let doc: Rc<RefCell<RinchDocument>> = app.doc.clone().unwrap();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc;
    let host = NodeHandle::new(rinch_core::dom::NodeId(host_id), Rc::downgrade(&dyn_doc));
    host.set_inner_html(markup);
    let first = dyn_doc
        .borrow()
        .get_children(host.node_id())
        .first()
        .map(|n| n.0)
        .unwrap_or(0);
    app.resolve_and_repaint(800.0, 600.0);
    if let Some(s) = own.borrow_mut().take() {
        s.dispose();
    }
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Enter,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    let l = log.borrow().clone();
    (l, app.focus_target, w_id, first)
}

/// R3a (#1509, review finding 1): the component is alive, so it is still told
/// it lost the keyboard when the arbiter releases the claim — as on main,
/// where the entry was still registered at the release.
#[test]
fn r3a_a_freed_focused_target_with_a_live_component_hears_focus_lost() {
    let (log, target, _w, _) = focused_target_freed("", false);
    assert_eq!(target, FocusTarget::None, "claim released: {log:?}");
    assert!(!log.iter().any(|e| e.starts_with("key:")), "{log:?}");
    assert_eq!(
        log,
        ["gained", "lost"],
        "on_focus_lost owed to a live component"
    );
}

/// R3a': the component is disposed before the claim is released: silent, as
/// any unmount (#141 PR4) — its callback would read freed state.
#[test]
fn r3a_a_freed_focused_target_whose_component_is_disposed_is_silent() {
    let (log, target, _w, _) = focused_target_freed("", true);
    assert_eq!(target, FocusTarget::None, "claim released: {log:?}");
    assert_eq!(log, ["gained"], "no callback after disposal");
}

/// R3b: the markup re-mints the focused id as a focusable element. The old
/// `on_key` must not reach it, and the claim is released rather than handed
/// to an element nobody focused; the live component hears `on_focus_lost`.
#[test]
fn r3b_a_freed_focused_target_reminted_by_the_markup() {
    let (log, target, w, first) = focused_target_freed(r#"<div tabindex="0">n</div>"#, false);
    assert_eq!(
        first, w,
        "precondition: the markup re-minted the focused id"
    );
    assert!(
        !log.iter().any(|e| e.starts_with("key:")),
        "old on_key reached the new node: {log:?}"
    );
    assert_eq!(target, FocusTarget::None, "claim released: {log:?}");
    assert_eq!(log, ["gained", "lost"]);
}
