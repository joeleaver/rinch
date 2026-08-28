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
//! Every removal path is covered, because each one strands entries on its own:
//! `remove_node` (the `for`/`show`/`match`/`virtual_list` churn path and the
//! editor's `ViewDesc` removals), `replace_node` (a per-keystroke editor path),
//! `set_inner_html` (which discards its element's existing children), and
//! `Drop for WebDocument` (the root/body wrappers, which no `remove_node` ever
//! reaches).
//!
//! **Measure deltas and specific ids, never an absolute registry length.** The
//! whole file runs in one wasm module on one page and `NEXT_NODE_ID` is
//! process-global, so every test here shares the counter and the registry.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_web::web_document::{__node_registry_contains, __node_registry_len, WebDocument};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A host element attached to the real `document.body`, so nodes built under it
/// are `is_connected()` — otherwise a "still registered" precondition could be
/// satisfied for the wrong reason.
fn host() -> web_sys::Element {
    let el = browser_document().create_element("div").unwrap();
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
fn removing_a_node_drops_it_from_the_registry() {
    let mut doc = doc();
    let body = doc.body();
    let id = child_div(&mut doc, body);

    assert!(
        __node_registry_contains(id.0),
        "precondition: a freshly created node is in the registry"
    );

    doc.remove_node(id);

    assert!(
        !__node_registry_contains(id.0),
        "#184: remove_node must drop the node's NODE_REGISTRY entry"
    );
}

#[wasm_bindgen_test]
fn removing_a_node_drops_it_from_the_document_map() {
    let mut doc = doc();
    let body = doc.body();
    let before = doc.__node_count();
    let id = child_div(&mut doc, body);

    assert!(doc.__contains(id.0), "precondition: node is in the doc map");
    assert_eq!(doc.__node_count(), before + 1);

    doc.remove_node(id);

    assert!(
        !doc.__contains(id.0),
        "#184: remove_node must drop the node's WebDocument::nodes entry"
    );
    assert_eq!(
        doc.__node_count(),
        before,
        "#184: the document map must be back to its pre-create size"
    );
}

#[wasm_bindgen_test]
fn removing_a_subtree_drops_every_descendant() {
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

    doc.remove_node(parent);

    // remove_node only detaches the top node from its parent; the descendants
    // stay attached to it, which is exactly why a naive one-line prune of the
    // top id alone is not enough.
    for id in [parent, child, text] {
        assert!(
            !__node_registry_contains(id.0),
            "#184: {id:?} must be gone from NODE_REGISTRY after its ancestor is removed"
        );
        assert!(
            !doc.__contains(id.0),
            "#184: {id:?} must be gone from the doc map after its ancestor is removed"
        );
    }
}

#[wasm_bindgen_test]
fn an_unattached_node_is_still_pruned_by_remove() {
    let mut doc = doc();

    // Built and never appended: `remove_node`'s `parent_node()` guard does not
    // hold, so a prune written *inside* that guard would keep leaking this.
    let id = doc.create_element("div");
    assert!(__node_registry_contains(id.0));

    doc.remove_node(id);

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

    // 50 rounds of "render a row, then drop it" — the `for`/`show`/`match` churn
    // shape. Today each round strands 3 entries per map, forever.
    for _ in 0..50 {
        let row = child_div(&mut doc, body);
        let label = child_div(&mut doc, row);
        let text = doc.create_text("row");
        doc.append_child(label, text);
        doc.remove_node(row);
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

    // The document's own root/body wrappers are never passed to `remove_node`,
    // so only `Drop for WebDocument` can release them.
    assert_eq!(
        __node_registry_len(),
        registry_baseline,
        "#184: dropping a WebDocument must release its remaining registry entries"
    );
}

#[wasm_bindgen_test]
fn replacing_a_node_prunes_the_replaced_subtree() {
    let mut doc = doc();
    let body = doc.body();

    let old = child_div(&mut doc, body);
    let old_child = child_div(&mut doc, old);
    let new = doc.create_element("span");
    doc.append_child(body, new);

    doc.replace_node(old, new);

    assert!(
        !__node_registry_contains(old.0) && !doc.__contains(old.0),
        "#184: replace_node must prune the node it orphaned"
    );
    assert!(
        !__node_registry_contains(old_child.0) && !doc.__contains(old_child.0),
        "#184: replace_node must prune the orphaned node's descendants too"
    );
    assert!(
        __node_registry_contains(new.0) && doc.__contains(new.0),
        "the replacement must survive"
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
