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
//! let visible = Signal::new(true);
//!
//! rsx! {
//!     Show {
//!         when: {|| visible.get()},
//!         fallback: |__scope| rsx! { div { "Hidden" } },
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
    W: Fn() -> bool + 'static,
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
            // Resources the branch creates belong to the branch's own scope, so
            // flipping the condition takes them with it (issue #141). Covers
            // both entry paths — initial render and the effect-driven swap.
            let content = {
                let _owner = child_scope.push_owner();
                render_fn(&mut child_scope)
            };
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
            // This ensures all nested effects are cleaned up properly.
            //
            // The `take()` is deliberately a separate statement: an `if let`
            // scrutinee's temporaries live through the then-block, so the
            // obvious one-liner holds `current_scope`'s `RefMut` across the
            // dispose. That used to be harmless, but disposal now runs user code
            // — cleanups, handler-closure drops, signal value drops (issue
            // #141) — and any of it that writes a signal flushes effects
            // synchronously, re-entering this very closure and panicking on the
            // outstanding borrow.
            let old_scope = current_scope_clone.borrow_mut().take();
            if let Some(old_scope) = old_scope {
                old_scope.dispose();
            }

            // Remove old content nodes
            for node in current_content_clone.borrow_mut().drain(..) {
                node.clear_animations();
                node.remove();
            }

            // Render new content after marker. Wrapped in untracked so
            // signal reads during branch rendering don't subscribe the
            // Show's condition-watching Effect. Branch content creates
            // its own effects for reactivity.
            if new_showing {
                crate::reactive::untracked(|| {
                    insert_content_after_marker(
                        &doc_weak_clone,
                        parent_id,
                        &marker_effect,
                        then_fn_clone.as_ref(),
                        &current_content_clone,
                        &current_scope_clone,
                    );
                });
            } else if let Some(ref else_fn) = else_fn_clone {
                crate::reactive::untracked(|| {
                    insert_content_after_marker(
                        &doc_weak_clone,
                        parent_id,
                        &marker_effect,
                        else_fn.as_ref(),
                        &current_content_clone,
                        &current_scope_clone,
                    );
                });
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
/// let visible = Signal::new(true);
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

    /// A branch body's resources are attributed to the branch's own child scope,
    /// not to the parent that hosts the `show` (issue #141).
    ///
    /// Both entry paths are covered by the single push in
    /// `insert_content_after_marker`: the initial render, and the effect-driven
    /// swap when the condition flips.
    #[test]
    fn a_branch_body_is_attributed_to_its_own_child_scope() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Owner, Signal, current_owner};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let showing = Signal::new(true);
        let owners: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let then_owners = owners.clone();
        let else_owners = owners.clone();
        let marker = crate::show::show_dom(
            &mut scope,
            &parent,
            move || showing.get(),
            move |s: &mut RenderScope| {
                then_owners.borrow_mut().push(current_owner());
                Signal::new(0);
                s.create_element("div")
            },
            Some(move |s: &mut RenderScope| {
                else_owners.borrow_mut().push(current_owner());
                Signal::new(0);
                s.create_element("span")
            }),
        );
        let _ = marker;

        showing.set(false); // flip: disposes the then-scope, builds the else-scope

        let owners = owners.borrow();
        assert_eq!(owners.len(), 2, "one initial render plus one swap");

        let then_owner = owners[0].clone().expect("the branch runs under an owner");
        let else_owner = owners[1].clone().expect("the branch runs under an owner");
        assert_ne!(
            then_owner, else_owner,
            "each branch gets its own child scope"
        );
        assert_ne!(
            else_owner,
            scope.owner(),
            "a branch is not attributed to the scope hosting the `show`"
        );
        assert!(
            !then_owner.is_alive(),
            "flipping the condition must have disposed the then-branch's scope"
        );
        assert_eq!(
            else_owner.owned_counts().map(|c| c.signals),
            Some(1),
            "the live branch's signal belongs to that branch"
        );
        assert_eq!(
            scope.owned_counts().signals,
            0,
            "and not to the parent render scope"
        );
    }

    /// A store created inside a branch dies with that branch (issue #141 PR3).
    ///
    /// This is the claim SD2 rests on — "a `create_store` inside a `match` arm
    /// removes its entry when the arm flips" — exercised on a real render path
    /// rather than a bare `Scope`.
    #[test]
    fn a_store_created_in_a_branch_dies_with_the_branch() {
        use crate::context::{clear_context, create_store, try_use_store};
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        #[derive(Clone, Debug, PartialEq)]
        struct BranchStore(&'static str);

        clear_context();
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let showing = Signal::new(true);
        let marker = crate::show::show_dom(
            &mut scope,
            &parent,
            move || showing.get(),
            move |s: &mut RenderScope| {
                create_store(BranchStore("in-branch"));
                s.create_element("div")
            },
            Some(move |s: &mut RenderScope| s.create_element("span")),
        );
        let _ = marker;

        assert_eq!(
            try_use_store::<BranchStore>(),
            Some(BranchStore("in-branch")),
            "the store resolves while its branch is mounted"
        );

        showing.set(false); // flip: the then-branch's scope is disposed

        assert_eq!(
            try_use_store::<BranchStore>(),
            None,
            "flipping the branch must reclaim the store it created"
        );
        clear_context();
    }

    /// A branch cleanup that reads a signal must not subscribe the show's
    /// condition-watching effect (issue #494).
    ///
    /// Flipping the condition disposes the outgoing branch from inside the show
    /// effect, so its `on_cleanup`s run with that effect as the current
    /// observer — a tracked read there would re-run the show on every later
    /// write to that signal, for the rest of the show's life, driven by a scope
    /// that no longer exists. The DOM is identical either way, so the assertion
    /// counts condition evaluations, not final state, with a positive control
    /// proving the counter can see a re-run.
    #[test]
    fn a_branch_cleanup_that_reads_a_signal_does_not_subscribe_the_show_effect() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Signal, on_cleanup};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let cond = Signal::new(true);
        let probe = Signal::new(0u32);
        let passes = Rc::new(Cell::new(0usize));

        let count = passes.clone();
        let _marker = crate::show::show_dom(
            &mut scope,
            &parent,
            move || {
                count.set(count.get() + 1);
                cond.get()
            },
            move |s: &mut RenderScope| {
                // Registered against the branch's own scope (the ambient
                // owner), read by the disposal fixpoint when the condition
                // flips.
                on_cleanup(move || {
                    let _ = probe.get();
                });
                s.create_element("div")
            },
            Some(move |s: &mut RenderScope| s.create_element("span")),
        );

        cond.set(false); // dispose the then-branch, running its cleanup
        let passes_before = passes.get();

        probe.set(1);
        assert_eq!(
            passes.get(),
            passes_before,
            "a write to a signal only the branch cleanup read must not re-run the show effect"
        );

        // Positive control: a write the show effect legitimately tracks.
        cond.set(true);
        assert_eq!(passes.get(), passes_before + 1);
    }

    /// A signal value owned by the outgoing branch runs its `Drop` during the
    /// same disposal (fixpoint step 6), and a read from it must not subscribe
    /// the show effect either (issue #494) — the value-drop half, through a
    /// real reconcile site.
    #[test]
    fn a_branch_signal_values_drop_that_reads_a_signal_does_not_subscribe_the_show_effect() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        /// Reads `probe` on drop — user code at value-teardown time.
        struct ReadsSignalOnDrop {
            probe: Signal<u32>,
        }
        impl Drop for ReadsSignalOnDrop {
            fn drop(&mut self) {
                let _ = self.probe.get();
            }
        }

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let cond = Signal::new(true);
        let probe = Signal::new(0u32);
        let passes = Rc::new(Cell::new(0usize));

        let count = passes.clone();
        let _marker = crate::show::show_dom(
            &mut scope,
            &parent,
            move || {
                count.set(count.get() + 1);
                cond.get()
            },
            move |s: &mut RenderScope| {
                // Owned by the branch scope: freed with the branch, its value
                // dropped by the fixpoint's final step.
                Signal::new(ReadsSignalOnDrop { probe });
                s.create_element("div")
            },
            Some(move |s: &mut RenderScope| s.create_element("span")),
        );

        cond.set(false); // dispose the then-branch, dropping its signal's value
        let passes_before = passes.get();

        probe.set(1);
        assert_eq!(
            passes.get(),
            passes_before,
            "a write to a signal only the freed value's Drop read must not re-run the show effect"
        );

        cond.set(true);
        assert_eq!(passes.get(), passes_before + 1);
    }
}
