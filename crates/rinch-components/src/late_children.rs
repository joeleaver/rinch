//! Giving a container's default to a child that arrives after it rendered —
//! issue #716.
//!
//! A container whose prop is a default for its items — `List::icon`,
//! `RadioGroup::size`, `Stepper::active` — cannot hand it over as a prop: a
//! parent component renders *after* its children, so it finds the rendered items
//! and patches them. That patch used to run once, and a child inserted later by
//! a `for` reconcile, a `show_dom` branch or a hand-rolled `append_child` came
//! out without the default.
//!
//! [`adopt_late_children`] is the second half: the same patch, re-run on the
//! subtree that just landed. The two halves have to agree, and they do by
//! construction — each container passes the *same* function to both.

use rinch_core::dom::{NodeHandle, RenderScope, on_child_inserted};

/// Re-run `patch` on every subtree inserted beneath `root` from now on.
///
/// `boundary` names the classes the container's render-time walk stops
/// descending at — its own item, and any class that marks a nested container of
/// the same kind. A subtree that landed *inside* one of those belongs to
/// whatever is nested there, not to this container, so it is declined. That is
/// the render-time walk's rule read upwards, and the two halves have to name the
/// same classes or a row gets one answer at render and the other one later.
///
/// `patch` gets a **fresh** [`RenderScope`] rooted at `root`, because the
/// container's own scope is a `&mut` borrow that ended when `render` returned.
/// Nothing here creates a reactive resource, so the scope is a place to mint
/// nodes from and nothing more. It holds only a `Weak` to the document, so a
/// registered observer never keeps a document alive.
pub fn adopt_late_children(
    scope: &RenderScope,
    root: &NodeHandle,
    boundary: &'static [&'static str],
    patch: impl Fn(&NodeHandle, &mut RenderScope) + 'static,
) {
    let doc = scope.doc_weak();
    let watched = root.clone();
    let root_id = root.node_id();
    on_child_inserted(root, move |inserted| {
        if crosses(inserted, &watched, boundary) {
            return;
        }
        let Some(doc) = doc.upgrade() else {
            return;
        };
        let mut scope = RenderScope::new(doc, root_id);
        patch(inserted, &mut scope);
    });
}

/// Does the chain from `node` up to `root` cross a node carrying any of
/// `classes`?
///
/// `node` itself is not asked — it may well *be* the item the container is
/// looking for. `root` is reached before it is asked, so a container's own class
/// appearing in `classes` does not make it decline its own children. A chain
/// that never reaches `root` answers `true`: the caller's only use for this is
/// to decline, and declining is the safe answer for a node whose relationship to
/// `root` cannot be established.
fn crosses(node: &NodeHandle, root: &NodeHandle, classes: &[&str]) -> bool {
    let mut current = node.parent_node();
    while let Some(here) = current {
        if here.node_id() == root.node_id() {
            return false;
        }
        if classes.iter().any(|class| has_class(&here, class)) {
            return true;
        }
        current = here.parent_node();
    }
    true
}

/// Does `node` carry `class` as a whole class token?
fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}
