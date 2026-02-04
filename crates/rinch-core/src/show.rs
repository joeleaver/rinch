//! Show component for conditional rendering.
//!
//! The Show component enables fine-grained conditional rendering in Rinch.
//! Unlike full re-renders, Show surgically updates only the affected DOM nodes
//! when the condition changes.
//!
//! # How It Works
//!
//! 1. On initial render, Show evaluates the condition and renders either
//!    children (true) or fallback (false)
//! 2. An Effect is created that watches the condition
//! 3. When the condition changes:
//!    - The old content's scope is disposed (cleaning up nested effects)
//!    - Old DOM nodes are removed
//!    - New content is rendered with a fresh scope
//!
//! # Marker-Based Rendering
//!
//! Show uses a comment marker node (`<!-- show -->`) instead of a wrapper
//! `<span>` element. Content is inserted as siblings after the marker in the
//! parent. This avoids polluting the DOM tree with extra elements that would
//! break CSS flex/grid layouts.
//!
//! # Example
//!
//! ```ignore
//! let visible = use_signal(|| true);
//!
//! rsx! {
//!     Show {
//!         when: {|| visible.get()},
//!         fallback: |scope| rsx! { div { "Hidden" } },
//!         div { "Shown!" }
//!     }
//! }
//! ```
//!
//! # Nested Shows
//!
//! Shows can be nested. Each Show manages its own scope, so disposing
//! a parent Show also disposes all nested Shows.
//!
//! # Scope Lifecycle
//!
//! When the condition changes, the old content's scope is disposed BEFORE
//! removing DOM nodes. This ensures all nested effects are cleaned up
//! properly, preventing:
//! - Memory leaks from accumulated effects
//! - Wasted CPU from effects running on removed content
//! - Potential crashes from effects updating removed nodes

use std::cell::RefCell;
use std::rc::Rc;

use crate::dom::{NodeHandle, RenderScope};
use crate::hooks::with_render_context;
use crate::reactive::Effect;

/// Fine-grained conditional rendering that surgically updates the DOM.
///
/// Uses a comment marker node instead of a wrapper element. Content is
/// inserted as siblings after the marker in the parent, avoiding any
/// interference with CSS flex/grid layouts.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `parent` - The parent node to insert the marker and content into
/// * `when` - A closure that returns the condition to evaluate
/// * `then_fn` - A closure that builds content when condition is true
/// * `else_fn` - Optional closure that builds content when condition is false
///
/// # Returns
///
/// The comment marker NodeHandle. The caller should NOT append this to
/// the parent — it is already inserted.
///
/// # How It Works
///
/// 1. Creates a `<!-- show -->` comment marker and appends it to the parent
/// 2. Evaluates the condition and renders initial content as sibling after marker
/// 3. Creates an Effect that watches the condition
/// 4. When condition changes:
///    - Disposes the old child scope (cleaning up nested effects)
///    - Removes old DOM content nodes
///    - Creates new child scope and renders new content after the marker
pub fn show_dom<W, T, E>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    when: W,
    then_fn: T,
    else_fn: Option<E>,
) -> NodeHandle
where
    W: Fn() -> bool + Clone + 'static,
    T: Fn(&mut RenderScope) -> NodeHandle + 'static,
    E: Fn(&mut RenderScope) -> NodeHandle + 'static,
{
    // Create comment marker and insert into parent
    let marker = scope.create_comment("show");
    parent.append_child(&marker);

    let parent_id = parent.node_id();

    // Store render functions as Rc for sharing with Effect
    let then_fn = Rc::new(then_fn);
    let else_fn = else_fn.map(Rc::new);

    // Track current state
    let showing = Rc::new(RefCell::new(when()));
    // Track owned content nodes (siblings after marker)
    let current_content: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    // Track the current child scope for proper disposal
    let current_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));

    // Get weak doc reference for creating new scopes in Effect
    let doc_weak = scope.doc_weak();
    let marker_clone = marker.clone();

    // Helper: render content and insert after marker
    fn insert_content_after_marker(
        doc_weak: &std::rc::Weak<RefCell<dyn crate::dom::DomDocument>>,
        parent_id: crate::dom::NodeId,
        marker: &NodeHandle,
        render_fn: &dyn Fn(&mut RenderScope) -> NodeHandle,
        current_content: &Rc<RefCell<Vec<NodeHandle>>>,
        current_scope: &Rc<RefCell<Option<RenderScope>>>,
    ) {
        if let Some(doc) = doc_weak.upgrade() {
            let mut child_scope = RenderScope::new(doc, parent_id);
            // Enable hook context for render functions called during Effects
            let content = with_render_context(|| render_fn(&mut child_scope));
            marker.insert_after(&content);
            current_content.borrow_mut().push(content);
            *current_scope.borrow_mut() = Some(child_scope);
        }
    }

    // Initial render
    {
        let is_showing = *showing.borrow();
        if is_showing {
            insert_content_after_marker(
                &doc_weak,
                parent_id,
                &marker,
                then_fn.as_ref(),
                &current_content,
                &current_scope,
            );
        } else if let Some(ref else_fn) = else_fn {
            insert_content_after_marker(
                &doc_weak,
                parent_id,
                &marker,
                else_fn.as_ref(),
                &current_content,
                &current_scope,
            );
        }
    }

    // Create Effect that swaps content when condition changes
    let showing_clone = showing.clone();
    let current_content_clone = current_content.clone();
    let current_scope_clone = current_scope.clone();
    let doc_weak_clone = doc_weak.clone();
    let then_fn_clone = then_fn;
    let else_fn_clone = else_fn;
    let marker_effect = marker_clone.clone();

    let effect = Effect::new(move || {
        let new_showing = when();
        let old_showing = *showing_clone.borrow();

        // Only update if condition actually changed
        if new_showing != old_showing {
            *showing_clone.borrow_mut() = new_showing;

            // DISPOSE old scope BEFORE removing DOM nodes
            // This ensures all nested effects are cleaned up properly
            if let Some(old_scope) = current_scope_clone.borrow_mut().take() {
                old_scope.dispose();
            }

            // Remove old content nodes
            for node in current_content_clone.borrow_mut().drain(..) {
                node.clear_animations();
                node.remove();
            }

            // Render new content after marker
            if new_showing {
                insert_content_after_marker(
                    &doc_weak_clone,
                    parent_id,
                    &marker_effect,
                    then_fn_clone.as_ref(),
                    &current_content_clone,
                    &current_scope_clone,
                );
            } else if let Some(ref else_fn) = else_fn_clone {
                insert_content_after_marker(
                    &doc_weak_clone,
                    parent_id,
                    &marker_effect,
                    else_fn.as_ref(),
                    &current_content_clone,
                    &current_scope_clone,
                );
            }
        }
    });

    // Attach the toggle effect to the parent scope so it lives as long as
    // the Show's container node. Previously stored globally which leaked.
    scope.create_effect_from(effect);

    marker
}

/// Builder for fine-grained Show rendering.
///
/// Provides a fluent API for building conditional content with RenderScope.
///
/// # Example
///
/// ```ignore
/// let visible = use_signal(|| true);
///
/// FineShowBuilder::new(move || visible.get())
///     .then(|scope| {
///         let div = scope.create_element("div");
///         div.set_text("Shown!");
///         div
///     })
///     .fallback(|scope| {
///         let div = scope.create_element("div");
///         div.set_text("Hidden");
///         div
///     })
///     .build(scope, &parent)
/// ```
pub struct FineShowBuilder<W, T, E>
where
    W: Fn() -> bool + Clone + 'static,
    T: Fn(&mut RenderScope) -> NodeHandle + 'static,
    E: Fn(&mut RenderScope) -> NodeHandle + 'static,
{
    when: W,
    then_fn: Option<T>,
    else_fn: Option<E>,
}

impl<W> FineShowBuilder<W, fn(&mut RenderScope) -> NodeHandle, fn(&mut RenderScope) -> NodeHandle>
where
    W: Fn() -> bool + Clone + 'static,
{
    /// Create a new FineShowBuilder with the given condition.
    pub fn new(when: W) -> Self {
        FineShowBuilder {
            when,
            then_fn: None,
            else_fn: None,
        }
    }
}

impl<W, T, E> FineShowBuilder<W, T, E>
where
    W: Fn() -> bool + Clone + 'static,
    T: Fn(&mut RenderScope) -> NodeHandle + 'static,
    E: Fn(&mut RenderScope) -> NodeHandle + 'static,
{
    /// Set the content to render when condition is true.
    pub fn then<T2>(self, then_fn: T2) -> FineShowBuilder<W, T2, E>
    where
        T2: Fn(&mut RenderScope) -> NodeHandle + 'static,
    {
        FineShowBuilder {
            when: self.when,
            then_fn: Some(then_fn),
            else_fn: self.else_fn,
        }
    }

    /// Set the content to render when condition is false.
    pub fn fallback<E2>(self, else_fn: E2) -> FineShowBuilder<W, T, E2>
    where
        E2: Fn(&mut RenderScope) -> NodeHandle + 'static,
    {
        FineShowBuilder {
            when: self.when,
            then_fn: self.then_fn,
            else_fn: Some(else_fn),
        }
    }

    /// Build the Show and return the marker NodeHandle.
    ///
    /// The marker and content are inserted directly into `parent`.
    /// The returned handle should NOT be appended again.
    ///
    /// # Panics
    ///
    /// Panics if `then` was not called.
    pub fn build(self, scope: &mut RenderScope, parent: &NodeHandle) -> NodeHandle {
        let then_fn = self
            .then_fn
            .expect("FineShowBuilder: then() must be called");
        show_dom(scope, parent, self.when, then_fn, self.else_fn)
    }
}

#[cfg(test)]
mod tests {
    use crate::reactive::{Effect, Scope};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn test_show_cleanup() {
        // Test that nested effects are disposed when Show toggles
        let effect_ran = Rc::new(Cell::new(0));
        let cleanup_ran = Rc::new(Cell::new(false));

        // Create a scope that tracks disposal
        let scope = Scope::new();

        let effect_ran_clone = effect_ran.clone();
        scope.add_effect(Effect::new(move || {
            effect_ran_clone.set(effect_ran_clone.get() + 1);
        }));

        let cleanup_ran_clone = cleanup_ran.clone();
        scope.on_cleanup(move || {
            cleanup_ran_clone.set(true);
        });

        // Effect ran once on creation
        assert_eq!(effect_ran.get(), 1);
        assert!(!cleanup_ran.get());

        // Dispose scope (simulating Show toggle)
        scope.dispose();

        // Cleanup ran
        assert!(cleanup_ran.get());
    }
}
