//! A request effect that writes the signal a branch renders from is not
//! subscribed to that branch's reads (issue #3047).
//!
//! re-ofscp's admin console: an effect reads the request it was asked for,
//! sets the result signal to "loading" and spawns the request; the screen
//! shows the result through `if let` / `match`. The effect's write swapped the
//! branch inside the effect's own run, and the branch render — which re-reads
//! the scrutinee to bind its names, under `untracked` — subscribed the
//! *effect* to the result signal. When the answer arrived, the effect ran
//! again and sent the request a second time.
//!
//! Each fixture counts requests, the effect's runs: one per ask.

use rinch::prelude::*;
use rinch::reactive::Effect;
use rinch_core::dom::DomTree;
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

#[derive(Clone, Debug)]
enum Load {
    Loading,
    Ready(Vec<String>),
}

thread_local! {
    /// Requests sent. Each test runs on its own thread.
    static REQUESTS: Cell<u32> = const { Cell::new(0) };
}

fn requests() -> u32 {
    REQUESTS.with(Cell::get)
}

/// The issue's audit screen: `wanted` asks, `list` answers, `match` shows.
#[component]
fn listing(wanted: Signal<u32>, list: Signal<Load>) -> NodeHandle {
    Effect::new(move || {
        wanted.get();
        list.set(Load::Loading);
        REQUESTS.with(|r| r.set(r.get() + 1));
    });
    rsx! {
        div {
            match list.get() {
                Load::Loading => p { "Loading" },
                Load::Ready(found) => div {
                    for e in found.clone() {
                        p { {e} }
                    }
                },
            }
        }
    }
}

#[test]
fn a_match_on_the_result_does_not_resend_the_request_when_it_arrives() {
    let wanted = Signal::new(0u32);
    let list = Signal::new(Load::Loading);
    let (_doc, _scope, root) = mount(|__scope| listing(__scope, wanted, list));
    assert_eq!(requests(), 1);

    // The first answer.
    list.set(Load::Ready(vec!["a".into()]));
    assert_eq!(text(&root), "a");

    // A click asks again; the effect sets "loading" inside its run.
    batch(|| wanted.set(1));
    assert_eq!(
        text(&root),
        "Loading",
        "control: the click swapped the branch"
    );
    assert_eq!(requests(), 2);

    // The answer arrives: shown, and not asked for a third time.
    list.set(Load::Ready(vec!["b".into(), "c".into()]));
    assert_eq!(text(&root), "bc");
    assert_eq!(requests(), 2, "the answer must not resend the request");
    list.set(Load::Ready(vec!["d".into()]));
    assert_eq!(requests(), 2);
}

/// The accounts screen's detail panel: `if let Some(detail) = shown.get()`
/// around a `match detail.clone()`.
#[component]
fn detail_panel(viewing: Signal<Option<u32>>, shown: Signal<Option<Load>>) -> NodeHandle {
    Effect::new(move || match viewing.get() {
        Some(_) => {
            shown.set(Some(Load::Loading));
            REQUESTS.with(|r| r.set(r.get() + 1));
        }
        None => shown.set(None),
    });
    rsx! {
        div {
            if let Some(detail) = shown.get() {
                div {
                    match detail.clone() {
                        Load::Loading => p { "Loading" },
                        Load::Ready(lines) => p { {lines.join(",")} },
                    }
                }
            }
        }
    }
}

#[test]
fn an_if_let_on_the_result_does_not_resend_the_request_when_it_arrives() {
    let viewing = Signal::new(None::<u32>);
    let shown = Signal::new(None::<Load>);
    let (_doc, _scope, root) = mount(|__scope| detail_panel(__scope, viewing, shown));
    assert_eq!((requests(), text(&root)), (0, String::new()));

    // A row is clicked: the effect sets "loading" inside its run.
    batch(|| viewing.set(Some(4)));
    assert_eq!(
        text(&root),
        "Loading",
        "control: the click showed the panel"
    );
    assert_eq!(requests(), 1);

    // The answer arrives.
    shown.set(Some(Load::Ready(vec!["x".into()])));
    assert_eq!(text(&root), "x");
    assert_eq!(requests(), 1, "the answer must not resend the request");

    // Positive control: a second ask is a second request.
    batch(|| viewing.set(Some(5)));
    assert_eq!(requests(), 2);
    shown.set(Some(Load::Ready(vec!["y".into()])));
    assert_eq!(requests(), 2);
}
