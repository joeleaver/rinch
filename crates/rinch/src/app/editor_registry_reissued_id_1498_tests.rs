//! The mounted-editor registry against `rinch-dom`, where a freed node's slot
//! really is handed to the next node minted (its very id until #723; a new
//! generation of the slot since) (issues #1498 and #1516; the
//! focus registry's twin is `focus_registry_reissued_id_1490_tests`).

use crate::editor::{EditorHandle, create_editor, editor_for, editor_for_doc};
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

type Doc = Rc<RefCell<dyn DomDocument>>;

fn el(doc: &Doc, tag: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_element(tag);
    NodeHandle::new(id, Rc::downgrade(doc))
}

/// Mint elements until one is handed `id`'s slot (rinch-dom pops the most
/// recently freed slab key first).
fn mint_at(doc: &Doc, id: NodeId) -> NodeHandle {
    for _ in 0..256 {
        let n = el(doc, "div");
        // The slot, not the id: since #723 a freed slot comes back under a
        // new generation, so no node is handed a freed id again.
        if n.node_id().index() == id.index() {
            assert_ne!(n.node_id(), id, "#723: the slot comes back under a new id");
            return n;
        }
    }
    panic!("precondition: the freed slot was not re-issued");
}

/// A document with a `host` element in its body.
fn page() -> (Doc, NodeHandle) {
    let doc: Doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc));
    let host = el(&doc, "div");
    body.append_child(&host);
    (doc, host)
}

/// Mount `editor` in a scope of its own, its container appended to `parent`.
fn mount(doc: &Doc, parent: &NodeHandle, editor: &EditorHandle) -> (RenderScope, NodeHandle) {
    let mut scope = RenderScope::new(doc.clone(), parent.node_id());
    let container = editor.mount(&mut scope);
    parent.append_child(&container);
    (scope, container)
}

/// #1498: editor A's container is freed by an ancestor's `set_inner_html`
/// while A's mount scope lives, editor B's container is handed its id, and
/// then A's scope is disposed. Its cleanup must release A's registration
/// only: B stays registered, so the runtime keeps driving its caret and
/// routing its input.
#[test]
fn an_editor_on_a_reissued_container_id_survives_the_old_mounts_cleanup() {
    let (doc, host) = page();
    let a = create_editor();
    let (old, a_container) = mount(&doc, &host, &a);
    let x = a_container.node_id();
    let doc_key = a_container.doc_key();

    host.set_inner_html("");
    // Put `x` on top of the free list on its own, so B's container (the
    // first node its mount mints) is handed it.
    let spare = mint_at(&doc, x);
    host.append_child(&spare);
    host.set_inner_html("");

    // Read-only marks B apart from A: `EditorHandle` has no public identity.
    let b = create_editor();
    b.set_read_only(true);
    let (new, b_container) = mount(&doc, &host, &b);
    let y = b_container.node_id();
    assert_eq!(
        y.index(),
        x.index(),
        "precondition: B's container is in A's old slot"
    );
    assert!(
        editor_for_doc(doc_key, y.0).is_some_and(|h| h.is_read_only()),
        "control: B is registered"
    );
    assert!(editor_for_doc(doc_key, x.0).is_none(), "#723: A's id names nothing");

    old.dispose();
    assert!(
        editor_for_doc(doc_key, y.0).is_some_and(|h| h.is_read_only()),
        "#1498: A's mount cleanup unregistered B"
    );
    new.dispose();
    assert!(
        editor_for_doc(doc_key, y.0).is_none(),
        "B's own cleanup releases it"
    );
}

/// #1516: `set_inner_html` over an ancestor frees a mounted editor's
/// container while the mount scope lives. A node minted on the freed id is
/// not that editor's container, so it must not answer `editor_for_doc` (which
/// is how the desktop runtime maps a node to `FocusTarget::Editor`) or
/// `editor_for` (the web's lookup). The container sits two levels below the
/// replaced element, and an editor outside the replaced subtree keeps its
/// registration.
#[test]
fn a_node_on_a_freed_editor_containers_id_is_not_an_editor() {
    let (doc, host) = page();
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc));
    let wrapper = el(&doc, "div");
    host.append_child(&wrapper);
    let a = create_editor();
    let (scope_a, a_container) = mount(&doc, &wrapper, &a);
    let (scope_o, o_container) = mount(&doc, &body, &create_editor());
    let x = a_container.node_id();
    let doc_key = a_container.doc_key();
    assert!(
        editor_for_doc(doc_key, x.0).is_some(),
        "control: A is registered"
    );

    host.set_inner_html("");
    let plain = mint_at(&doc, x);
    host.append_child(&plain);
    for id in [plain.node_id().0, x.0] {
        assert!(
            editor_for_doc(doc_key, id).is_none(),
            "#1516: a plain <div> in the freed container's slot answers as the dead editor"
        );
        assert!(
            editor_for(id).is_none(),
            "#1516: and through the web's lookup"
        );
    }
    let o = o_container.node_id().0;
    assert!(
        editor_for_doc(doc_key, o).is_some(),
        "an editor outside the replaced subtree keeps its registration"
    );

    scope_a.dispose();
    assert!(
        editor_for_doc(doc_key, o).is_some(),
        "A's cleanup, finding nothing of its own, leaves the other editor"
    );
    scope_o.dispose();
    assert!(editor_for_doc(doc_key, o).is_none());
}
