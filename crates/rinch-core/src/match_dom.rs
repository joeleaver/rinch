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
            let content = branch_fn(&mut child_scope);
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

            // Dispose old scope BEFORE removing DOM nodes
            if let Some(old_scope) = scope_clone.borrow_mut().take() {
                old_scope.dispose();
            }

            // Remove old content nodes
            for node in content_clone.borrow_mut().drain(..) {
                node.clear_animations();
                node.remove();
            }

            // Render new branch
            if new_idx < branches_clone.len() {
                render_branch(
                    &doc_weak_clone,
                    parent_id,
                    &marker_effect,
                    branches_clone[new_idx].as_ref(),
                    &content_clone,
                    &scope_clone,
                );
            }
        }
    });

    scope.create_effect_from(effect);

    marker
}
