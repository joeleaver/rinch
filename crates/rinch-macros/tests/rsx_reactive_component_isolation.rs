//! A reactive-prop component re-renders on its *prop* signals and nothing else
//! (issue #390).
//!
//! `reactive_component_dom` runs the macro's render closure tracked — that is
//! its re-render mechanism: the prop closures are called inside it, and their
//! signal reads schedule the next render. The defect was that the *subtree*
//! render ran in the same tracked region, so a signal read anywhere inside the
//! component body or its children — a nested `match`'s scrutinee, most
//! visibly — also subscribed the re-render effect. A change to it rebuilt the
//! whole subtree instead of swapping one arm, and the rebuild disposed the
//! subtree's scopes, resetting component-local state.
//!
//! The DOM ends up correct either way (the full rebuild produces the right
//! tree), which is exactly why this survived: the assertions here are a render
//! counter and state survival, not final DOM shape. Both tests fail against
//! the unfixed codegen (subtree render not wrapped in `untracked`); the
//! prop-reactivity assertions fail against the opposite mutant (the *whole*
//! render closure wrapped in `untracked`), which is what pins the tracked/
//! untracked line where it belongs.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Mount a component into a headless document.
///
/// Returns the document and the scope alongside the root: a [`NodeHandle`]
/// holds only a `Weak` to the document, so letting either go out of scope
/// would leave every handle answering as if the tree were empty.
fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

/// The rendered text of a subtree, skipping comment markers (`<!-- component -->`,
/// `<!-- match -->`) that `text_content` would otherwise splice in.
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

/// A component that counts its own `render` calls and prints its `tone` prop
/// as text, ahead of its children.
#[derive(Debug, Default)]
struct Probe {
    tone: String,
    renders: Rc<Cell<u32>>,
}

impl Component for Probe {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        self.renders.set(self.renders.get() + 1);
        let root = scope.create_element("div");
        let tone = scope.create_text(&self.tone);
        root.append_child(&tone);
        for child in children {
            root.append_child(child);
        }
        root
    }
}

// ============================================================
// Render count: an inner scrutinee swaps its arm, not the component
// ============================================================

#[component]
fn count_fixture(
    variant: Signal<bool>,
    scrutinee: Signal<u32>,
    renders: Rc<Cell<u32>>,
) -> NodeHandle {
    rsx! {
        div {
            Probe {
                // The closure prop is what routes this component through
                // `reactive_component_dom` — and its `variant` read is the only
                // signal the re-render effect is meant to track.
                tone: {move || if variant.get() { "warm".to_string() } else { "cold".to_string() }},
                renders: renders.clone(),
                match scrutinee.get() {
                    0 => span { "zero" },
                    _ => span { "more" },
                }
            }
        }
    }
}

#[test]
fn an_inner_scrutinee_change_swaps_the_arm_without_rerendering_the_component() {
    let variant = Signal::new(false);
    let scrutinee = Signal::new(0u32);
    let renders = Rc::new(Cell::new(0u32));

    let (_doc, _scope, root) = mount(|s| count_fixture(s, variant, scrutinee, renders.clone()));

    assert_eq!(renders.get(), 1, "one initial render");
    assert_eq!(text(&root), "coldzero");

    // The inner change: the nested match must swap its own arm...
    scrutinee.set(5);
    assert_eq!(text(&root), "coldmore", "the inner match swapped its arm");
    // ...without the component's re-render effect having subscribed to its
    // scrutinee. The DOM is identical either way — the counter is the test.
    assert_eq!(
        renders.get(),
        1,
        "an inner scrutinee change must not re-render the whole component"
    );

    // The prop signal still drives a re-render: this is what a blanket
    // `untracked` around the whole render closure would break, since the prop
    // closures are called inside it and their reads are the re-render trigger.
    variant.set(true);
    assert_eq!(
        renders.get(),
        2,
        "a prop signal change still re-renders the component"
    );
    assert_eq!(text(&root), "warmmore");
}

// ============================================================
// State survival: what a user actually notices
// ============================================================

/// A child that creates component-local state and hands the signal out so the
/// test can tell a surviving subtree from a rebuilt one: a rebuild disposes
/// the old scope (freeing the signal) and logs a fresh one.
#[component]
fn stateful_child(log: Rc<RefCell<Vec<Signal<i32>>>>) -> NodeHandle {
    let local = Signal::new(0i32);
    log.borrow_mut().push(local);
    rsx! {
        span { {move || local.get().to_string()} }
    }
}

#[component]
fn state_fixture(
    variant: Signal<bool>,
    scrutinee: Signal<u32>,
    renders: Rc<Cell<u32>>,
    log: Rc<RefCell<Vec<Signal<i32>>>>,
) -> NodeHandle {
    rsx! {
        div {
            Probe {
                tone: {move || if variant.get() { "warm".to_string() } else { "cold".to_string() }},
                renders: renders.clone(),
                { stateful_child(__scope, log.clone()) }
                match scrutinee.get() {
                    0 => span { "zero" },
                    _ => span { "more" },
                }
            }
        }
    }
}

#[test]
fn component_local_state_survives_an_inner_scrutinee_change() {
    let variant = Signal::new(false);
    let scrutinee = Signal::new(0u32);
    let renders = Rc::new(Cell::new(0u32));
    let log: Rc<RefCell<Vec<Signal<i32>>>> = Rc::new(RefCell::new(Vec::new()));

    let (_doc, _scope, root) =
        mount(|s| state_fixture(s, variant, scrutinee, renders.clone(), log.clone()));

    assert_eq!(renders.get(), 1);
    assert_eq!(log.borrow().len(), 1, "the stateful child rendered once");

    // Mutate the component-local state, then change the inner scrutinee.
    let local = log.borrow()[0];
    local.set(42);
    assert_eq!(text(&root), "cold42zero");

    scrutinee.set(5);

    // Length first: in the broken codegen the component re-renders here, which
    // disposes the old subtree scope (freeing `local` — reading it would
    // panic) and logs a fresh signal.
    assert_eq!(
        log.borrow().len(),
        1,
        "the subtree was not rebuilt: the stateful child still ran exactly once"
    );
    assert_eq!(
        local.get(),
        42,
        "component-local state survived the inner change"
    );
    assert_eq!(renders.get(), 1);
    assert_eq!(text(&root), "cold42more");
}
