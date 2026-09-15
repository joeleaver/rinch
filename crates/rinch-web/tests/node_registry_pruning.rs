//! Browser-driven tests for the node-handle leak, issue #184.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: a node the backend no longer owns is dropped from
//! **both** node maps — the page-global `NODE_REGISTRY` and the document's own
//! `nodes` — so neither map grows without bound. Both hold a *strong*
//! `web_sys::Node`, so a stranded entry pins the detached browser node against
//! GC for the life of the wasm module.
//!
//! # Which verb releases, since #719
//!
//! `remove_node` used to be the release point. It is not any more: it is a
//! **detach**, and the node stays in both maps so a handle can put it back —
//! the post-condition desktop always had, and the one a reactive branch
//! re-showing a *captured* handle rests on (issue #719). What releases is
//! [`DomDocument::discard_node`], which every caller that throws a subtree away
//! calls instead.
//!
//! So this file now tests a **pair** at each site: `discard_node` prunes and
//! `remove_node` does not. Asserting only the prune would be passed by a
//! backend that retires everything, which is exactly what #719 was.
//!
//! Every releasing path is covered, because each one strands entries on its own:
//! `discard_node` (the `for`/`virtual_list`/component-re-render churn paths and
//! the editor's `ViewDesc` removals), `set_inner_html` (which destroys its
//! element's existing children), and `Drop for WebDocument` (the root/body
//! wrappers, which no removal ever reaches). `replace_node` is no longer one of
//! them — it detaches, and its callers discard afterwards.
//!
//! The end-to-end guard is `churning_a_for_loop_does_not_grow_the_registry`:
//! the primitive tests below would all stay green if a reactive helper called
//! `remove` where it meant `discard`, and that fixture is what would not.
//!
//! **Measure deltas and specific ids, never an absolute registry length.** The
//! whole file runs in one wasm module on one page and `NEXT_NODE_ID` is
//! process-global, so every test here shares the counter and the registry.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::for_each_dom_typed;
use rinch_core::reactive::Signal;
use rinch_web::web_document::{__node_registry_contains, __node_registry_len, WebDocument};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A host element attached to the real `document.body`, so nodes built under it
/// are `is_connected()` — otherwise a "still registered" precondition could be
/// satisfied for the wrong reason.
///
/// A `WebDocument` adopts its host and never detaches it, so each host is torn
/// down when the next one is built: the whole file shares one page, and leaving
/// eight orphans under `<body>` would be visible to any later test that reads it.
fn host() -> web_sys::Element {
    let stale = browser_document()
        .query_selector_all("[data-test-host]")
        .unwrap();
    for i in 0..stale.length() {
        if let Some(node) = stale.item(i)
            && let Some(parent) = node.parent_node()
        {
            parent.remove_child(&node).ok();
        }
    }
    let el = browser_document().create_element("div").unwrap();
    el.set_attribute("data-test-host", "true").unwrap();
    browser_document()
        .body()
        .unwrap()
        .append_child(&el)
        .unwrap();
    el
}

fn doc() -> WebDocument {
    WebDocument::new_into(browser_document(), host())
}

/// A `div` appended under `parent`.
fn child_div(doc: &mut WebDocument, parent: NodeId) -> NodeId {
    let id = doc.create_element("div");
    doc.append_child(parent, id);
    id
}

#[wasm_bindgen_test]
fn discarding_a_node_drops_it_from_the_registry() {
    let mut doc = doc();
    let body = doc.body();
    let id = child_div(&mut doc, body);

    assert!(
        __node_registry_contains(id.0),
        "precondition: a freshly created node is in the registry"
    );

    doc.discard_node(id);

    assert!(
        !__node_registry_contains(id.0),
        "#184: discard_node must drop the node's NODE_REGISTRY entry"
    );
}

/// The twin, and the whole of #719: `remove_node` must **keep** both entries,
/// because the caller may put the node back.
#[wasm_bindgen_test]
fn removing_a_node_keeps_it_in_both_maps() {
    let mut doc = doc();
    let body = doc.body();
    let id = child_div(&mut doc, body);

    doc.remove_node(id);

    assert!(
        __node_registry_contains(id.0),
        "#719: remove_node is a detach — the NODE_REGISTRY entry must survive"
    );
    assert!(
        doc.__contains(id.0),
        "#719: and so must the WebDocument::nodes entry"
    );
}

#[wasm_bindgen_test]
fn discarding_a_node_drops_it_from_the_document_map() {
    let mut doc = doc();
    let body = doc.body();
    let before = doc.__node_count();
    let id = child_div(&mut doc, body);

    assert!(doc.__contains(id.0), "precondition: node is in the doc map");
    assert_eq!(doc.__node_count(), before + 1);

    doc.discard_node(id);

    assert!(
        !doc.__contains(id.0),
        "#184: discard_node must drop the node's WebDocument::nodes entry"
    );
    assert_eq!(
        doc.__node_count(),
        before,
        "#184: the document map must be back to its pre-create size"
    );
}

#[wasm_bindgen_test]
fn discarding_a_subtree_drops_every_descendant() {
    let mut doc = doc();
    let body = doc.body();

    let parent = child_div(&mut doc, body);
    let child = child_div(&mut doc, parent);
    let text = doc.create_text("hello");
    doc.append_child(child, text);

    for id in [parent, child, text] {
        assert!(
            __node_registry_contains(id.0) && doc.__contains(id.0),
            "precondition: {id:?} is registered in both maps"
        );
    }

    doc.discard_node(parent);

    // The detach only unlinks the top node from its parent; the descendants
    // stay attached to it, which is exactly why a naive one-line prune of the
    // top id alone is not enough.
    for id in [parent, child, text] {
        assert!(
            !__node_registry_contains(id.0),
            "#184: {id:?} must be gone from NODE_REGISTRY after its ancestor is discarded"
        );
        assert!(
            !doc.__contains(id.0),
            "#184: {id:?} must be gone from the doc map after its ancestor is discarded"
        );
    }
}

#[wasm_bindgen_test]
fn an_unattached_node_is_still_pruned_by_discard() {
    let mut doc = doc();

    // Built and never appended: `discard_node`'s `parent_node()` guard does not
    // hold, so a prune written *inside* that guard would keep leaking this.
    // Every discard that follows a `replace_node` is in this state too, since
    // the replace already unlinked the node.
    let id = doc.create_element("div");
    assert!(__node_registry_contains(id.0));

    doc.discard_node(id);

    assert!(
        !__node_registry_contains(id.0),
        "#184: a node that was never appended must still be pruned from NODE_REGISTRY"
    );
    assert!(
        !doc.__contains(id.0),
        "#184: a node that was never appended must still be pruned from the doc map"
    );
}

#[wasm_bindgen_test]
fn churning_a_list_does_not_grow_the_registry() {
    let mut doc = doc();
    let body = doc.body();

    let registry_baseline = __node_registry_len();
    let doc_baseline = doc.__node_count();

    // 50 rounds of "render a row, then drop it" — the `for`/`virtual_list`
    // churn shape. Without the prune each round strands 3 entries per map,
    // forever.
    for _ in 0..50 {
        let row = child_div(&mut doc, body);
        let label = child_div(&mut doc, row);
        let text = doc.create_text("row");
        doc.append_child(label, text);
        doc.discard_node(row);
    }

    assert_eq!(
        __node_registry_len(),
        registry_baseline,
        "#184: churning a list must not grow NODE_REGISTRY"
    );
    assert_eq!(
        doc.__node_count(),
        doc_baseline,
        "#184: churning a list must not grow WebDocument::nodes"
    );
}

#[wasm_bindgen_test]
fn dropping_a_document_releases_its_root_wrappers() {
    let registry_baseline = __node_registry_len();

    {
        let mut doc = doc();
        let body = doc.body();
        child_div(&mut doc, body);
        assert!(
            __node_registry_len() > registry_baseline,
            "precondition: the document and its content registered nodes"
        );
    }

    // The adopted host an island mounts into is never passed to `remove_node`,
    // so only `Drop for WebDocument` can release it.
    assert_eq!(
        __node_registry_len(),
        registry_baseline,
        "#184: dropping a WebDocument must release its remaining registry entries"
    );

    // The `mount()` shape is the one the leak actually bites: `WebDocument::new`
    // builds *two* wrappers (`#rinch-root`, `#rinch-body`) that no `remove_node`
    // ever reaches. `new_into` above adopts a single host, so it does not cover
    // this on its own.
    {
        let _doc = WebDocument::new(browser_document());
        assert_eq!(
            __node_registry_len(),
            registry_baseline + 2,
            "precondition: WebDocument::new registers its root and body wrappers"
        );
    }
    assert_eq!(
        __node_registry_len(),
        registry_baseline,
        "#184: dropping a WebDocument must release its root *and* body wrappers"
    );
    // `Drop` deliberately leaves `#rinch-root` attached to the real body; take it
    // out so the shared test page does not accumulate one per run.
    if let Ok(Some(root)) = browser_document().query_selector("#rinch-root")
        && let Some(parent) = root.parent_node()
    {
        parent.remove_child(&root).ok();
    }
}

/// `replace_node` **detaches** the node it displaced (issue #719) — and the
/// caller's follow-up `discard_node` is what releases it, on a node that is
/// already unlinked.
///
/// This pair is the editor's `diff_children` shape, the highest-churn user of
/// `replace_node` in the workspace (once per kind-changed block per keystroke).
#[wasm_bindgen_test]
fn replacing_a_node_detaches_it_and_a_later_discard_prunes_it() {
    let mut doc = doc();
    let body = doc.body();

    let old = child_div(&mut doc, body);
    let old_child = child_div(&mut doc, old);
    let new = doc.create_element("span");
    doc.append_child(body, new);

    doc.replace_node(old, new);

    assert!(
        __node_registry_contains(old.0) && doc.__contains(old.0),
        "#719: replace_node must leave the displaced node re-insertable"
    );
    assert!(
        __node_registry_contains(new.0) && doc.__contains(new.0),
        "the replacement must survive"
    );

    doc.discard_node(old);

    assert!(
        !__node_registry_contains(old.0) && !doc.__contains(old.0),
        "#184: discarding a displaced node must prune it"
    );
    assert!(
        !__node_registry_contains(old_child.0) && !doc.__contains(old_child.0),
        "#184: and its descendants too, though the node is already unlinked"
    );
}

/// `replaceChild(n, n)` succeeds — the DOM spec re-inserts `n` before its own
/// next sibling — so a self-replace must not report a node that is still in the
/// tree as displaced (issue #184).
#[wasm_bindgen_test]
fn replacing_a_node_with_itself_keeps_it_registered() {
    let mut doc = doc();
    let body = doc.body();

    let node = child_div(&mut doc, body);
    doc.replace_node(node, node);

    assert!(
        __node_registry_contains(node.0) && doc.__contains(node.0),
        "#184: a self-replace must leave the node live in both maps"
    );
    assert_eq!(
        doc.get_children(body),
        vec![node],
        "the node must still be its parent's child"
    );
}

#[wasm_bindgen_test]
fn set_inner_html_prunes_the_children_it_discards() {
    let mut doc = doc();
    let body = doc.body();

    let container = child_div(&mut doc, body);
    let discarded = child_div(&mut doc, container);
    let discarded_child = doc.create_text("gone");
    doc.append_child(discarded, discarded_child);

    // Blows away every existing child, then registers the parsed replacements.
    doc.set_inner_html(container, "<p>fresh</p>");

    for id in [discarded, discarded_child] {
        assert!(
            !__node_registry_contains(id.0),
            "#184: {id:?} must leave NODE_REGISTRY when set_inner_html discards it"
        );
        assert!(
            !doc.__contains(id.0),
            "#184: {id:?} must leave the doc map when set_inner_html discards it"
        );
    }
    assert!(
        !doc.get_children(container).is_empty(),
        "precondition: the new markup was registered"
    );
}

/// **The end-to-end guard.** Every other fixture here calls `discard_node`
/// itself, so all of them stay green if a *reactive helper* calls `remove`
/// where it meant `discard` — which since #719 is the way the leak comes back.
///
/// This one drives the real `for_each_dom_typed` over a real `WebDocument` and
/// watches the page-global registry across 30 rounds of replacing the whole
/// list. Each round drops three rows of two nodes each, so a helper that merely
/// detached would strand 180 entries.
#[wasm_bindgen_test]
fn churning_a_for_loop_does_not_grow_the_registry() {
    let host = host();
    let doc: Rc<RefCell<WebDocument>> = Rc::new(RefCell::new(WebDocument::new_into(
        browser_document(),
        host,
    )));
    let body = doc.borrow().body();
    let body_handle = NodeHandle::new(body, Rc::downgrade(&doc) as _);
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body);

    let rows = Signal::new(vec![0u32, 1, 2]);
    for_each_dom_typed(
        &mut scope,
        &body_handle,
        move || rows.get(),
        |n: &u32| n.to_string(),
        |n: u32, s: &mut RenderScope| {
            let row = s.create_element("div");
            let label = s.create_text(&n.to_string());
            row.append_child(&label);
            row
        },
    );

    // Measure *after* the first render, so the baseline is a mounted list and
    // the delta is churn alone.
    let registry_baseline = __node_registry_len();
    let doc_baseline = doc.borrow().__node_count();

    for round in 1..=30u32 {
        let base = round * 3;
        rows.set(vec![base, base + 1, base + 2]);
    }

    assert_eq!(
        __node_registry_len(),
        registry_baseline,
        "#184/#719: a churning `for` must discard its dropped rows, not merely remove them"
    );
    assert_eq!(
        doc.borrow().__node_count(),
        doc_baseline,
        "#184/#719: and the document map must not grow either"
    );
}
