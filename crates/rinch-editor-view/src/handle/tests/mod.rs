//! `EditorHandle`'s unit tests, and the fixtures and helpers the test files
//! share.

use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Pos, default_plugins};

mod anchors;
/// `on_key`, `on_selection_change` and `caret_rect`: what an app needs to
/// drive an autocomplete popup from the editor.
/// The caret-affinity hint (#301) lives beside the selection it was set
/// with and applies only while the selection is still that one.
mod caret_affinity;
mod change_and_batching;
mod clipboard_and_links;
// ── Collaboration (design M9, the `collaboration` feature) ───────────────
//
// Two real `EditorHandle`s (each over its own mock host) wired into a single
// in-process loopback — the exact seam the two-pane demo uses. The convergence
// assertions exercise the whole wiring: a local edit's `record_local` →
// `save_incremental` → `outbound`, and the peer's `collab_receive` →
// `integrate_incremental` → re-projection.
#[cfg(feature = "collaboration")]
mod collab;
mod commands;
/// The image hooks keep the #147/#183 lifetime rules (review of #1501).
mod image_hook_lifetime;
mod ime;
mod loading;
mod node_selection;
/// Issue #943: plugin code the handle runs — `init_state`, `apply`,
/// `decorations`, `handle_paste`, a plugin's command, an input rule —
/// runs untracked, so a plugin that reads an app signal does not subscribe
/// the app effect that happened to drive the editor. The caller's own
/// `update(|state| …)` build closure is the caller's code and stays
/// tracked (the positive control in it).
mod plugin_tracking;
mod popup_hooks;
mod read_only;
/// `EditorHandle::scroll_into_view` and `EditorHandle::focus`, against the
/// mock: which boxes the reveal places, in which order it scrolls to them,
/// when it waits, and what carries or drops it. The platforms' end-to-end
/// pins are `rinch/src/app/editor_focus_and_reveal_tests.rs` and
/// `rinch-web/tests/editor_focus_and_reveal.rs`.
mod reveal;
mod scroll_gate;

struct Harness {
    doc: Rc<RefCell<dyn DomDocument>>,
    /// The *same* allocation as `doc`, typed: the mock's test-only helpers are
    /// inherent methods and unreachable through `dyn DomDocument`.
    mock: Rc<RefCell<MockDomDocument>>,
    container_id: NodeId,
    handle: EditorHandle,
}

fn mount(html_blocks: Node) -> Harness {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    let schema = Rc::new(Schema::starter_kit());
    let handle = EditorHandle::new(
        container,
        Rc::downgrade(&doc),
        schema,
        html_blocks,
        default_plugins(),
    );
    Harness {
        doc,
        mock,
        container_id,
        handle,
    }
}

/// Give the container's block children the stacked `tops`.
fn measure(h: &Harness, tops: &[f32]) {
    let blocks = children(h, h.container_id);
    let mut m = h.mock.borrow_mut();
    for (b, top) in blocks.iter().zip(tops) {
        m.__set_node_layout(*b, 0.0, *top, 200.0, 20.0);
    }
}

fn drain(h: &Harness) -> Vec<NodeId> {
    h.doc.borrow_mut().drain_scroll_into_view_requests()
}

fn schema() -> Schema {
    Schema::starter_kit()
}
fn para(s: &Schema, t: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
        .unwrap()
}
fn doc_node(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}
fn children(h: &Harness, id: NodeId) -> Vec<NodeId> {
    h.doc.borrow().get_children(id)
}
fn tag(h: &Harness, id: NodeId) -> Option<String> {
    h.doc.borrow().tag_name(id)
}
fn text(h: &Harness, id: NodeId) -> Option<String> {
    h.doc.borrow().text_content(id)
}

// ── SelectionAnchor (the asynchronous-insertion point, #149) ─────────────

/// The first block's text as maximal runs with and without the mark `name`.
fn runs_by_mark(doc: &Node, name: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for n in doc.child(0).content().iter() {
        let on = n.marks().iter().any(|m| m.type_name() == name);
        let t = n.text().unwrap_or_default();
        match out.last_mut() {
            Some((run, last)) if *last == on => run.push_str(t),
            _ => out.push((t.to_string(), on)),
        }
    }
    out
}

fn empty_doc(s: &Schema) -> Node {
    s.branch(
        "doc",
        Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
    )
    .unwrap()
}
