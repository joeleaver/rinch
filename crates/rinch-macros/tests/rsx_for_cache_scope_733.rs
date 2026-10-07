//! `rsx!`'s `for` over a cache filled from inside its body, issue #733 — the
//! recipe the guide gives (`docs/src/guide/rsx-syntax.md`, "A cache filled from
//! inside a `for`"), end to end through the macro.
//!
//! A `for` body is only ever handed its row's scope, so a row built there and
//! cached is the row's: discarded when its key leaves, with every effect,
//! signal and handler it made. The body asks for a scope that is not the
//! row's — [`RenderScope::cache_scope`] — builds through it with
//! [`RenderScope::build`], and keeps it beside the node.
//!
//! Over `MockDomDocument`, which retires a discarded node as `rinch-web` does.
//! The helper-level fixtures are `rinch_core::reinsertion_tests::lazy_memo_733`
//! and, in Chrome, `rinch-web/tests/for_memo_733.rs`; what this adds is that
//! the recipe as written compiles and that `rsx!` inside `build` is owned by
//! the cache scope (its reactive text, and a `Signal` made beside it).

use rinch::prelude::*;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{__retired_view_returns, DomDocument};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Rendered text of a subtree, skipping the control-flow comment markers.
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

type Cache = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;

thread_local! {
    /// The `Signal` each cached tab made for itself while it was built.
    static LOCAL: Cell<Option<Signal<u32>>> = const { Cell::new(None) };
}

/// The guide's helper: hand back the cached tab, or build it once through a
/// cache scope and keep that scope with it.
fn cached_tab(
    scope: &mut RenderScope,
    cache: &Cache,
    id: u32,
    label: Signal<String>,
) -> NodeHandle {
    if let Some((row, _keep)) = cache.borrow().get(&id) {
        return row.clone();
    }
    let mut keep = scope.cache_scope();
    let row = keep.build(|__scope| {
        let clicks = Signal::new(0u32);
        LOCAL.with(|l| l.set(Some(clicks)));
        rsx! {
            article {
                {id.to_string()} ":" {|| label.get()} ":" {|| clicks.get().to_string()}
            }
        }
    });
    cache.borrow_mut().insert(id, (row.clone(), keep));
    row
}

#[component]
fn tabs(open: Signal<Vec<u32>>, cache: Cache, label: Signal<String>) -> NodeHandle {
    rsx! {
        div {
            for id in open.get() {
                {cached_tab(__scope, &cache, id, label)}
            }
        }
    }
}

#[test]
fn a_for_over_a_cache_scoped_row_keeps_the_row_and_its_bindings() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);
    let cache: Cache = Rc::default();
    let open = Signal::new(vec![1u32, 2]);
    let label = Signal::new(String::from("a"));
    let root = tabs(&mut outer, open, cache.clone(), label);
    assert_eq!(text(&root), "1:a:02:a:0", "precondition: both tabs shown");
    let first = cache.borrow()[&1].0.node_id();
    let clicks = LOCAL.with(Cell::get).expect("tab 2 made its signal");

    open.set(vec![1]);
    assert_eq!(text(&root), "1:a:0", "precondition: tab 2 closed");
    // Both bindings of the closed tab are written while it is out.
    label.set("b".into());
    clicks.set(5);

    open.set(vec![1, 2]);
    assert_eq!(
        text(&root),
        "1:b:02:b:5",
        "#733: the cached tab comes back, and its reactive text and its own \
         signal lived through the row leaving"
    );
    assert_eq!(cache.borrow()[&1].0.node_id(), first);
    label.set("c".into());
    assert_eq!(
        text(&root),
        "1:c:02:c:5",
        "#733: and still update afterwards"
    );

    let count = doc.borrow().__node_count();
    for i in 0..200 {
        open.set(if i % 2 == 0 { vec![1] } else { vec![1, 2] });
    }
    assert_eq!(
        doc.borrow().__node_count(),
        count,
        "#733: a cached row is one row, however often its key comes and goes"
    );
    assert_eq!(
        __retired_view_returns(),
        (0, 0),
        "nothing retired came back"
    );
}

// ── the guide's recipe, whole: the component owns the cache and evicts ──────
//
// This is the code block of `docs/src/guide/rsx-syntax.md`, "A cache filled
// from inside a `for`", with `label` standing in for the tab's content. Keep
// the two in step: the guide is not compiled, this is.

#[component]
fn tab_strip(open: Signal<Vec<u32>>, label: Signal<String>) -> NodeHandle {
    let cache = Cache::default();
    // Dropping the cache is not evicting: it stops the tabs' effects and
    // leaves their nodes in the backend. Let go of each one when the strip
    // unmounts.
    let evict = cache.clone();
    __scope.on_cleanup(move || {
        for (_, (tab, keep)) in evict.borrow_mut().drain() {
            keep.dispose();
            tab.discard();
        }
    });
    rsx! {
        div {
            for id in open.get() {
                {cached_tab(__scope, &cache, id, label)}
            }
        }
    }
}

#[component]
fn page(shown: Signal<bool>, open: Signal<Vec<u32>>, label: Signal<String>) -> NodeHandle {
    rsx! {
        main {
            if shown.get() {
                {tab_strip(__scope, open, label)}
            }
        }
    }
}

/// The owner of the cache unmounts and mounts again, 50 times. With the
/// `on_cleanup` drain nothing is left behind; without it every mount strands
/// its cached tabs (`rinch_core::cache_scope_733_tests::r4_…`: +200 nodes for
/// this shape).
#[test]
fn the_recipes_cache_owner_can_unmount_without_leaking() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);
    let shown = Signal::new(true);
    let open = Signal::new(vec![1u32, 2]);
    let label = Signal::new(String::from("a"));
    let root = page(&mut outer, shown, open, label);
    assert_eq!(text(&root), "1:a:02:a:0", "precondition: mounted");

    // A tab closed and reopened inside one mount is the same tab.
    open.set(vec![1]);
    label.set("b".into());
    open.set(vec![1, 2]);
    assert_eq!(text(&root), "1:b:02:b:0");

    shown.set(false);
    assert_eq!(text(&root), "", "precondition: unmounted");
    let count = doc.borrow().__node_count();
    for i in 0..100 {
        shown.set(i % 2 == 0);
    }
    assert_eq!(
        doc.borrow().__node_count(),
        count,
        "#733: 50 mounts of a list of cached tabs leave no node behind"
    );
    shown.set(true);
    assert_eq!(text(&root), "1:b:02:b:0", "and a fresh mount builds afresh");
    assert_eq!(__retired_view_returns(), (0, 0));
}
