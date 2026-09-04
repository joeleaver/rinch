//! Match component for multi-branch conditional rendering.
//!
//! The match_dom function enables fine-grained multi-branch conditional rendering
//! in Rinch. It generalizes `show_dom` (2 branches) to N branches selected by index.
//!
//! # How It Works
//!
//! 1. On initial render, match_dom evaluates the discriminant and renders the
//!    corresponding branch
//! 2. An Effect is created that watches the discriminant
//! 3. When the discriminant changes:
//!    - The old content's scope is disposed (cleaning up nested effects)
//!    - Old DOM nodes are removed
//!    - New content is rendered with a fresh scope
//!
//! # Marker-Based Rendering
//!
//! Like `show_dom`, match_dom uses a comment marker node (`<!-- match -->`)
//! instead of a wrapper element. Content is inserted as siblings after the marker
//! in the parent, avoiding interference with CSS flex/grid layouts.

use std::cell::RefCell;
use std::rc::Rc;

use crate::dom::{NodeHandle, RenderScope};
use crate::reactive::Effect;

/// A boxed render closure for a single match branch.
type BranchFn = Box<dyn Fn(&mut RenderScope) -> NodeHandle>;

/// Multi-branch conditional rendering that surgically updates the DOM.
///
/// Uses a comment marker node instead of a wrapper element. Content is
/// inserted as siblings after the marker in the parent.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `parent` - The parent node to insert the marker and content into
/// * `discriminant` - A closure that returns the index of the active branch (0-based)
/// * `branches` - A vec of render closures, one per branch
///
/// # Returns
///
/// The comment marker NodeHandle. Already inserted into parent.
/// The caller should NOT append this to the parent again.
pub fn match_dom<D>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    discriminant: D,
    branches: Vec<BranchFn>,
) -> NodeHandle
where
    D: Fn() -> usize + 'static,
{
    // Create comment marker and insert into parent
    let marker = scope.create_comment("match");
    parent.append_child(&marker);

    let parent_id = parent.node_id();
    let doc_weak = scope.doc_weak();
    let branches = Rc::new(branches);

    // Track current state
    let current_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(usize::MAX)); // sentinel
    let current_content: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let current_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));

    let marker_clone = marker.clone();

    // Helper: render branch content after marker
    fn render_branch(
        doc_weak: &std::rc::Weak<RefCell<dyn crate::dom::DomDocument>>,
        parent_id: crate::dom::NodeId,
        marker: &NodeHandle,
        branch_fn: &dyn Fn(&mut RenderScope) -> NodeHandle,
        current_content: &Rc<RefCell<Vec<NodeHandle>>>,
        current_scope: &Rc<RefCell<Option<RenderScope>>>,
    ) {
        if let Some(doc) = doc_weak.upgrade() {
            let mut child_scope = RenderScope::new(doc, parent_id);
            // Attribute the branch's resources to the branch's own scope
            // (issue #141). Covers the initial render and the arm swap alike.
            let content = {
                let _owner = child_scope.push_owner();
                branch_fn(&mut child_scope)
            };
            marker.insert_after(&content);
            current_content.borrow_mut().push(content);
            *current_scope.borrow_mut() = Some(child_scope);
        }
    }

    // Initial render
    let initial_idx = discriminant();
    if initial_idx < branches.len() {
        *current_index.borrow_mut() = initial_idx;
        render_branch(
            &doc_weak,
            parent_id,
            &marker,
            branches[initial_idx].as_ref(),
            &current_content,
            &current_scope,
        );
    }

    // Create Effect that swaps branches when discriminant changes
    let idx_clone = current_index.clone();
    let content_clone = current_content.clone();
    let scope_clone = current_scope.clone();
    let doc_weak_clone = doc_weak.clone();
    let branches_clone = branches;
    let marker_effect = marker_clone;

    let effect = Effect::new(move || {
        let new_idx = discriminant();
        let old_idx = *idx_clone.borrow();

        if new_idx != old_idx {
            *idx_clone.borrow_mut() = new_idx;

            // Dispose old scope BEFORE removing DOM nodes.
            //
            // `take()` on its own line so the `RefMut` is not held across the
            // dispose — see the matching note in `show_dom` (issue #141).
            let old_scope = scope_clone.borrow_mut().take();
            if let Some(old_scope) = old_scope {
                old_scope.dispose();
            }

            // Remove old content nodes
            for node in content_clone.borrow_mut().drain(..) {
                node.clear_animations();
                node.remove();
            }

            // Render new branch. Wrapped in untracked so signal reads
            // during branch rendering don't subscribe the match Effect.
            if new_idx < branches_clone.len() {
                crate::reactive::untracked(|| {
                    render_branch(
                        &doc_weak_clone,
                        parent_id,
                        &marker_effect,
                        branches_clone[new_idx].as_ref(),
                        &content_clone,
                        &scope_clone,
                    );
                });
            }
        }
    });

    scope.create_effect_from(effect);

    marker
}

#[cfg(test)]
mod tests {
    use crate::dom::traits::DomDocument;
    use crate::dom::{RenderScope, mock::MockDomDocument};
    use crate::reactive::{Owner, Signal, current_owner};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Each `match` arm body is attributed to that arm's own child scope, and
    /// switching arms disposes the outgoing one (issue #141).
    ///
    /// Both entry paths — the initial render and the effect-driven arm swap —
    /// run through the same `render_branch` helper, so one guard covers both.
    #[test]
    fn an_arm_body_is_attributed_to_its_own_child_scope() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let which = Signal::new(0usize);
        let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let branches: Vec<super::BranchFn> = (0..2)
            .map(|_| {
                let log = seen.clone();
                Box::new(move |s: &mut RenderScope| {
                    log.borrow_mut().push(current_owner());
                    Signal::new(0);
                    s.create_element("div")
                }) as super::BranchFn
            })
            .collect();

        let marker = super::match_dom(&mut scope, &parent, move || which.get(), branches);
        let _ = marker;

        which.set(1); // swap arms

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "initial render plus one arm swap");

        let first = seen[0].clone().expect("an arm runs under an owner");
        let second = seen[1].clone().expect("an arm runs under an owner");
        assert_ne!(first, second, "each arm gets its own child scope");
        assert_ne!(
            second,
            scope.owner(),
            "an arm is not attributed to the scope hosting the `match`"
        );
        assert!(
            !first.is_alive(),
            "switching arms disposed the outgoing one"
        );
        assert_eq!(
            second.owned_counts().map(|c| c.signals),
            Some(1),
            "the live arm owns the signal its body created"
        );
        assert_eq!(scope.owned_counts().signals, 0);
    }

    /// An arm cleanup that reads a signal must not subscribe the match's
    /// discriminant-watching effect (issue #494).
    ///
    /// Swapping arms disposes the outgoing arm from inside the match effect, so
    /// a tracked cleanup read would re-run the match — re-evaluating the
    /// discriminant — on every later write to that signal. Counts discriminant
    /// evaluations, since the DOM is identical either way.
    #[test]
    fn an_arm_cleanup_that_reads_a_signal_does_not_subscribe_the_match_effect() {
        use crate::reactive::on_cleanup;
        use std::cell::Cell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let which = Signal::new(0usize);
        let probe = Signal::new(0u32);
        let passes = Rc::new(Cell::new(0usize));

        let branches: Vec<super::BranchFn> = vec![
            Box::new(move |s: &mut RenderScope| {
                on_cleanup(move || {
                    let _ = probe.get();
                });
                s.create_element("div")
            }),
            Box::new(|s: &mut RenderScope| s.create_element("span")),
        ];

        let count = passes.clone();
        let _marker = super::match_dom(
            &mut scope,
            &parent,
            move || {
                count.set(count.get() + 1);
                which.get()
            },
            branches,
        );

        which.set(1); // swap arms: disposes arm 0, running its cleanup
        let passes_before = passes.get();

        probe.set(1);
        assert_eq!(
            passes.get(),
            passes_before,
            "a write to a signal only the arm's cleanup read must not re-run the match effect"
        );

        // Positive control: a write the match effect legitimately tracks.
        which.set(0);
        assert_eq!(passes.get(), passes_before + 1);
    }
}
