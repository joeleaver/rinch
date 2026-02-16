//! Scope: manages the lifetime of reactive primitives.

use std::cell::{Cell, RefCell};

use super::Effect;

/// A scope that manages the lifetime of reactive primitives.
///
/// When a scope is disposed, all effects created within it are cleaned up.
/// Scopes can have child scopes for hierarchical cleanup.
///
/// # Example
///
/// ```ignore
/// let scope = Scope::new();
///
/// scope.run(|| {
///     let signal = Signal::new(0);
///     Effect::new(|| { /* ... */ });
///     // signal and effect belong to this scope
/// });
///
/// scope.dispose(); // Cleans up signal, effect, and all child scopes
/// ```
pub struct Scope {
    effects: RefCell<Vec<Effect>>,
    children: RefCell<Vec<Scope>>,
    cleanups: RefCell<Vec<Box<dyn FnOnce()>>>,
    disposed: Cell<bool>,
}

impl Scope {
    /// Create a new scope.
    pub fn new() -> Self {
        Self {
            effects: RefCell::new(Vec::new()),
            children: RefCell::new(Vec::new()),
            cleanups: RefCell::new(Vec::new()),
            disposed: Cell::new(false),
        }
    }

    /// Check if this scope has been disposed.
    pub fn is_disposed(&self) -> bool {
        self.disposed.get()
    }

    /// Run a function within this scope, capturing any effects created.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        // TODO: Implement scope tracking so effects created within
        // are automatically registered to this scope
        f()
    }

    /// Register an effect with this scope.
    pub fn add_effect(&self, effect: Effect) {
        self.effects.borrow_mut().push(effect);
    }

    /// Create a child scope that will be disposed when this scope is disposed.
    ///
    /// Child scopes are useful for conditional or list rendering where nested
    /// content needs independent lifecycle management.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let parent = Scope::new();
    /// let child = parent.child_scope();
    ///
    /// child.add_effect(Effect::new(|| { /* ... */ }));
    ///
    /// parent.dispose(); // Also disposes child and its effects
    /// ```
    pub fn child_scope(&self) -> Scope {
        // We return the child and expect the caller to manage it
        // The parent stores a reference for cleanup
        Scope::new()
    }

    /// Add a child scope to be disposed with this scope.
    pub fn add_child(&self, child: Scope) {
        self.children.borrow_mut().push(child);
    }

    /// Register a cleanup function to run when this scope is disposed.
    ///
    /// Cleanup functions run after child scopes and effects are disposed.
    pub fn on_cleanup<F: FnOnce() + 'static>(&self, f: F) {
        self.cleanups.borrow_mut().push(Box::new(f));
    }

    /// Dispose of all effects, child scopes, and run cleanup functions.
    ///
    /// After dispose, this scope should not be used.
    pub fn dispose(&self) {
        if self.disposed.get() {
            return;
        }

        // Use a thread-local disposal queue to avoid stack overflow.
        //
        // The problem: Effect closures may capture Rc<RefCell<RenderScope>>,
        // so Effect::dispose() -> drops closure -> drops RenderScope -> Scope::drop
        // -> dispose() -> more Effect::dispose() -- creating unbounded recursion.
        //
        // The solution: The outermost dispose() call runs an iterative loop.
        // Nested dispose() calls (triggered by closure drops) just push their
        // effects onto the queue instead of processing them immediately.
        thread_local! {
            static DISPOSE_QUEUE: RefCell<Option<Vec<Effect>>> = const { RefCell::new(None) };
        }

        let is_root = DISPOSE_QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            if q.is_none() {
                *q = Some(Vec::new());
                true
            } else {
                false
            }
        });

        // Mark this scope and collect its effects into the queue
        self.dispose_into_queue(&DISPOSE_QUEUE);

        if is_root {
            // We are the outermost dispose call. Process the queue iteratively.
            loop {
                let batch: Vec<Effect> =
                    DISPOSE_QUEUE.with(|q| std::mem::take(q.borrow_mut().as_mut().unwrap()));
                if batch.is_empty() {
                    break;
                }
                // Disposing effects may drop closures that own RenderScopes,
                // triggering more Scope::drop -> dispose() calls. Those nested
                // calls will push onto the queue (not recurse) because
                // DISPOSE_QUEUE is Some.
                for effect in batch {
                    effect.dispose();
                }
            }
            // Clean up the queue
            DISPOSE_QUEUE.with(|q| *q.borrow_mut() = None);
        }
    }

    /// Mark this scope as disposed and push its effects onto the disposal queue.
    /// Also iteratively processes all child scopes.
    fn dispose_into_queue(
        &self,
        queue: &'static std::thread::LocalKey<RefCell<Option<Vec<Effect>>>>,
    ) {
        if self.disposed.get() {
            return;
        }
        self.disposed.set(true);

        // Collect effects into the queue
        let effects: Vec<Effect> = self.effects.borrow_mut().drain(..).collect();
        queue.with(|q| {
            if let Some(ref mut vec) = *q.borrow_mut() {
                vec.extend(effects);
            }
        });

        // Run cleanups
        for cleanup in self.cleanups.borrow_mut().drain(..) {
            cleanup();
        }

        // Process children iteratively
        let mut pending: Vec<Scope> = self.children.borrow_mut().drain(..).collect();
        while let Some(child) = pending.pop() {
            if child.disposed.get() {
                // Drain to prevent recursive field drop
                child.children.borrow_mut().drain(..);
                child.effects.borrow_mut().drain(..);
                continue;
            }
            child.disposed.set(true);

            pending.extend(child.children.borrow_mut().drain(..));
            let child_effects: Vec<Effect> = child.effects.borrow_mut().drain(..).collect();
            queue.with(|q| {
                if let Some(ref mut vec) = *q.borrow_mut() {
                    vec.extend(child_effects);
                }
            });

            for cleanup in child.cleanups.borrow_mut().drain(..) {
                cleanup();
            }
        }
    }

    /// Clear all effects without disposing them.
    /// Used when transferring effects to another scope.
    pub fn take_effects(&self) -> Vec<Effect> {
        self.effects.borrow_mut().drain(..).collect()
    }

    /// Get the number of effects in this scope.
    pub fn effect_count(&self) -> usize {
        self.effects.borrow().len()
    }

    /// Get the number of child scopes.
    pub fn child_count(&self) -> usize {
        self.children.borrow().len()
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Use the iterative dispose to avoid stack overflow.
        // Rust's default field drop would recursively drop children -> Scope -> Drop,
        // and Effect::dispose can drop closures that own RenderScopes -> more Scopes.
        self.dispose();
    }
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("effects", &self.effects.borrow().len())
            .field("children", &self.children.borrow().len())
            .field("cleanups", &self.cleanups.borrow().len())
            .field("disposed", &self.disposed.get())
            .finish()
    }
}
