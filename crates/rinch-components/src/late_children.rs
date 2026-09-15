//! Giving a container's default to a child that arrives after it rendered, and
//! re-deriving when one leaves — issues #716 and #745.
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
//!
//! [`adopt_child_removals`] is for the container whose derivation depends on a
//! sibling's *position* rather than only on the container's own prop. A step
//! that leaves renumbers every step behind it exactly as one that arrives does
//! (issue #745), so `Stepper` takes both; `List` and `RadioGroup` take only the
//! first, since neither of their defaults can be changed by a row going away.

use rinch_core::dom::{NodeHandle, RenderScope, on_child_inserted, on_child_removed};

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

/// Re-run `patch` over `root` whenever a subtree is removed from beneath it
/// (issue #745).
///
/// `boundary` means what it means above, read against the node the subtree
/// **left** — the former parent, which is what the removal half of the registry
/// hands over, because a removed node is detached and has no chain to test. The
/// set of nodes checked is exactly the set [`adopt_late_children`] checks for an
/// insertion into that same parent, so a container that declines a late arrival
/// inside a nested container of its own kind declines a departure from there too.
///
/// `patch` is handed only a scope: there is no *subject* to pass. A container
/// that reaches for this is re-deriving from the children it has left — which is
/// what the insertion half's consumer does with the node it is handed anyway
/// (`Stepper::settle_steps` ignores it by name).
pub fn adopt_child_removals(
    scope: &RenderScope,
    root: &NodeHandle,
    boundary: &'static [&'static str],
    patch: impl Fn(&mut RenderScope) + 'static,
) {
    let doc = scope.doc_weak();
    let watched = root.clone();
    let root_id = root.node_id();
    on_child_removed(root, move |vacated| {
        if crosses_from(vacated, &watched, boundary) {
            return;
        }
        let Some(doc) = doc.upgrade() else {
            return;
        };
        let mut scope = RenderScope::new(doc, root_id);
        patch(&mut scope);
    });
}

/// Does the chain from `node` up to `root` cross a node carrying any of
/// `classes`?
///
/// `node` itself is not asked — it may well *be* the item the container is
/// looking for. See [`crosses_from`], which this defers to, for the rest.
fn crosses(node: &NodeHandle, root: &NodeHandle, classes: &[&str]) -> bool {
    match node.parent_node() {
        Some(parent) => crosses_from(&parent, root, classes),
        None => true,
    }
}

/// [`crosses`], starting **at** `node` rather than at its parent.
///
/// The removal half is handed the parent the change happened under, where the
/// insertion half is handed the child, so the two reach the same chain from
/// different ends — and the chain that has to be tested is the same one either
/// way, the parent included.
///
/// `root` is reached before it is asked, so a container's own class appearing in
/// `classes` does not make it decline its own children. A chain that never
/// reaches `root` answers `true`: the caller's only use for this is to decline,
/// and declining is the safe answer for a node whose relationship to `root`
/// cannot be established.
fn crosses_from(node: &NodeHandle, root: &NodeHandle, classes: &[&str]) -> bool {
    let mut current = Some(node.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_core::dom::traits::DomDocument;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const BOUNDARY: &[&str] = &["item"];

    struct Fixture {
        _doc: Rc<RefCell<MockDomDocument>>,
        scope: RenderScope,
        root: NodeHandle,
    }

    impl Fixture {
        fn new() -> Self {
            let doc = Rc::new(RefCell::new(MockDomDocument::new()));
            let body = doc.borrow().body();
            let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
            let root = scope.create_element("div");
            scope.body_handle().append_child(&root);
            Self {
                _doc: doc,
                scope,
                root,
            }
        }

        fn child(&mut self, parent: &NodeHandle, class: &str) -> NodeHandle {
            let node = self.scope.create_element("div");
            node.set_attribute("class", class);
            parent.append_child(&node);
            node
        }

        /// Watch `root` for removals, and hand back the call count.
        fn watch(&self) -> Rc<Cell<usize>> {
            let runs = Rc::new(Cell::new(0));
            let sink = runs.clone();
            adopt_child_removals(&self.scope, &self.root, BOUNDARY, move |_| {
                sink.set(sink.get() + 1)
            });
            runs
        }
    }

    #[test]
    fn a_removal_from_the_containers_own_subtree_reaches_the_patch() {
        let mut f = Fixture::new();
        let root = f.root.clone();
        let wrapper = f.child(&root, "wrapper");
        let doomed = f.child(&wrapper, "");
        let runs = f.watch();

        doomed.remove();

        assert_eq!(
            runs.get(),
            1,
            "issue #745: a node that left the container's subtree — through a \
             wrapper a `for` loop put there, so the container is not the direct \
             parent — is the container's business"
        );
    }

    #[test]
    fn a_removal_from_inside_a_boundary_node_is_declined() {
        let mut f = Fixture::new();
        let root = f.root.clone();
        let item = f.child(&root, "item");
        let inner = f.child(&item, "wrapper");
        let doomed = f.child(&inner, "");
        let runs = f.watch();

        doomed.remove();

        assert_eq!(
            runs.get(),
            0,
            "what is inside an item belongs to whatever is nested there, the \
             same rule the insertion half reads downwards"
        );
    }

    #[test]
    fn a_removal_from_a_boundary_node_itself_is_declined() {
        let mut f = Fixture::new();
        let root = f.root.clone();
        let item = f.child(&root, "item");
        let doomed = f.child(&item, "");
        let runs = f.watch();

        doomed.remove();

        assert_eq!(
            runs.get(),
            0,
            "the chain tested for a removal starts *at* the parent, not above \
             it: the removal half is handed the parent where the insertion half \
             is handed the child, so testing `parent.parent_node()` upwards \
             would skip exactly one node — the item itself, which is the \
             commonest boundary of all"
        );
    }

    #[test]
    fn the_container_itself_is_not_a_boundary_against_its_own_children() {
        let mut f = Fixture::new();
        // The container's own class is in the boundary set, which is what lets a
        // `List` decline a nested `List` without declining itself.
        f.root.set_attribute("class", "item");
        let root = f.root.clone();
        let doomed = f.child(&root, "");
        let runs = f.watch();

        doomed.remove();

        assert_eq!(
            runs.get(),
            1,
            "`root` is reached before it is asked, so a container carrying one \
             of its own boundary classes still owns its direct children"
        );
    }
}
