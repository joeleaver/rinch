//! The `#[once]` opt-out on an rsx `if let` / `match` (issue #2105).
//!
//! By default a branch whose pattern binds a value is rebuilt when the value
//! changes (see `rsx_if_let_rebinding_2105.rs`). `#[once]` on an `if` (and its
//! `else if` chain), on a `match`, or on one match arm keeps the old
//! behaviour: the branch is built once each time it is selected, and keeps the
//! value it bound then. It is for a branch whose own controls write the
//! scrutinee (a form), where a rebuild per keystroke would drop focus.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

/// The rendered text of a subtree, skipping comment markers.
fn text(node: &NodeHandle) -> String {
    if node.node_type() == Some(8) {
        return String::new();
    }
    let children = node.children();
    if children.is_empty() {
        return node.text_content().unwrap_or_default();
    }
    children.iter().map(text).collect()
}

thread_local! {
    /// How many times a branch body ran. Each test runs on its own thread.
    static RENDERS: Cell<u32> = const { Cell::new(0) };
}

fn renders() -> u32 {
    RENDERS.with(Cell::get)
}

/// A branch body that counts its own builds.
fn mark(__scope: &mut RenderScope, label: &str) -> NodeHandle {
    RENDERS.with(|r| r.set(r.get() + 1));
    let p = __scope.create_element("p");
    let t = __scope.create_text(label);
    p.append_child(&t);
    p
}

// ============================================================
// The opt-out: `#[once]`
// ============================================================

#[component]
fn once_if(user: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            #[once]
            if let Some(name) = user.get() {
                { mark(__scope, &name) }
            } else {
                { mark(__scope, "nobody") }
            }
        }
    }
}

#[test]
fn once_keeps_an_if_let_built_once_per_show() {
    let user = Signal::new(Some("Alice".to_string()));
    let (_doc, _scope, root) = mount(|s| once_if(s, user));
    assert_eq!(text(&root), "Alice");
    user.set(Some("Bob".into()));
    assert_eq!(text(&root), "Alice", "`#[once]` keeps the first binding");
    assert_eq!(renders(), 1);
    // A flip still rebuilds.
    user.set(None);
    assert_eq!(text(&root), "nobody");
    user.set(Some("Carol".into()));
    assert_eq!(text(&root), "Carol");
    assert_eq!(renders(), 3);
}

#[component]
fn once_chain(a: Signal<Option<u32>>, b: Signal<Option<u32>>) -> NodeHandle {
    rsx! {
        div {
            #[once]
            if let Some(x) = a.get() {
                { mark(__scope, &format!("a={x}")) }
            } else if let Some(y) = b.get() {
                { mark(__scope, &format!("b={y}")) }
            }
        }
    }
}

#[test]
fn once_on_an_if_covers_its_else_if_chain() {
    let a = Signal::new(None::<u32>);
    let b = Signal::new(Some(1u32));
    let (_doc, _scope, root) = mount(|s| once_chain(s, a, b));
    assert_eq!(text(&root), "b=1");
    b.set(Some(2));
    assert_eq!(text(&root), "b=1");
    assert_eq!(renders(), 1);
}

#[component]
fn once_match(sel: Signal<Option<u32>>, limit: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            #[once]
            match sel.get() {
                Some(n) if n > limit.get() => { mark(__scope, &format!("big {n}")) },
                Some(n) => { mark(__scope, &format!("item {n}")) },
                None => { mark(__scope, "none") },
            }
        }
    }
}

#[test]
fn once_keeps_every_arm_of_a_match_built_once() {
    let sel = Signal::new(Some(5u32));
    let limit = Signal::new(10u32);
    let (_doc, _scope, root) = mount(|s| once_match(s, sel, limit));
    limit.set(20);
    sel.set(Some(6));
    assert_eq!(text(&root), "item 5", "`#[once]` keeps the first binding");
    assert_eq!(renders(), 1);
    sel.set(Some(50));
    assert_eq!(text(&root), "big 50", "changing arm still rebuilds");
}

#[component]
fn once_arm(sel: Signal<Result<u32, String>>) -> NodeHandle {
    rsx! {
        div {
            match sel.get() {
                #[once]
                Ok(n) => { mark(__scope, &format!("ok {n}")) },
                Err(e) => { mark(__scope, &format!("err {e}")) },
            }
        }
    }
}

#[test]
fn once_on_one_arm_leaves_the_others_following() {
    let sel = Signal::new(Ok::<u32, String>(1));
    let (_doc, _scope, root) = mount(|s| once_arm(s, sel));
    sel.set(Ok(2));
    assert_eq!(text(&root), "ok 1", "the `#[once]` arm keeps its first binding");
    assert_eq!(renders(), 1);
    sel.set(Err("x".into()));
    sel.set(Err("y".into()));
    assert_eq!(text(&root), "err y", "the other arm follows");
    assert_eq!(renders(), 3);
}

// ============================================================
// A rebuilt input is a new node (what loses focus); `#[once]` keeps it
// ============================================================

#[component]
fn form(draft: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(text) = draft.get() {
                input { class: "follow", value: {text} }
            }
            #[once]
            if let Some(text) = draft.get() {
                input { class: "once", value: {text} }
            }
        }
    }
}

fn find_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some(class) {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_class(c, class))
}

#[test]
fn a_rebuild_replaces_an_input_and_once_keeps_it() {
    let draft = Signal::new(Some("h".to_string()));
    let (_doc, _scope, root) = mount(|s| form(s, draft));
    let follow = find_class(&root, "follow").unwrap().node_id();
    let once = find_class(&root, "once").unwrap().node_id();
    // What the input's own `oninput` would write.
    draft.set(Some("he".into()));
    assert_ne!(
        find_class(&root, "follow").unwrap().node_id(),
        follow,
        "the default rebuild makes a new <input>, which drops focus"
    );
    assert_eq!(
        find_class(&root, "once").unwrap().node_id(),
        once,
        "`#[once]` keeps the same <input>"
    );
}

