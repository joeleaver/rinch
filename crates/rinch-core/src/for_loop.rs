//! For component for efficient list rendering.
//!
//! The For component enables fine-grained list rendering with keyed reconciliation.
//! When the list changes, only affected items are added, removed, or moved -
//! unchanged items keep their DOM nodes and internal state.
//!
//! # Marker-Based Rendering
//!
//! For uses a comment marker node (`<!-- for -->`) instead of a wrapper
//! `<span>` element. List items are inserted as siblings after the marker in the
//! parent. This avoids polluting the DOM tree with extra elements that would
//! break CSS flex/grid layouts.
//!
//! # How It Works
//!
//! 1. On initial render, For renders each item with its own scope
//! 2. An Effect is created that watches the `each` closure
//! 3. When the list changes:
//!    - Keys are compared to identify what changed
//!    - Removed items have their scopes disposed and nodes removed
//!    - New items are rendered with fresh scopes
//!    - Moved items are repositioned in the DOM
//!    - Unchanged items are left alone
//!
//! # Keyed Reconciliation
//!
//! Keys are essential for efficient updates. Without keys, the entire list
//! would need to be re-rendered on any change. With keys, For can:
//!
//! - Identify which items are new, removed, or moved
//! - Preserve DOM nodes and component state for unchanged items
//! - Minimize DOM operations via LIS-based diffing
//!
//! # Example
//!
//! ```ignore
//! let items = use_signal(|| vec![
//!     Item { id: "1", name: "Alice" },
//!     Item { id: "2", name: "Bob" },
//! ]);
//!
//! rsx! {
//!     For {
//!         each: {|| items.get().into_iter().map(|item| {
//!             ForItem::new(item.id.clone(), item)
//!         }).collect()},
//!         |item| {
//!             let item = item.downcast::<Item>().unwrap();
//!             rsx! {
//!                 div {
//!                     {|| item.name.clone()}
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::dom::{NodeHandle, RenderScope};
use crate::element::ForItem;
use crate::hooks::{HookEntry, pop_hook_scope, push_hook_scope, with_render_context};
use crate::reactive::Effect;
use crate::reconcile::diff_keyed;

/// Create ForItem instances from an iterator with a key function.
///
/// # Example
///
/// ```ignore
/// let items: Vec<ForItem> = to_for_items(
///     my_items.into_iter(),
///     |item| item.id.clone(),
/// );
/// ```
pub fn to_for_items<T, I, K>(iter: I, key_fn: impl Fn(&T) -> K) -> Vec<ForItem>
where
    T: 'static,
    I: Iterator<Item = T>,
    K: ToString,
{
    iter.map(|item| {
        let key = key_fn(&item).to_string();
        ForItem::new(key, item)
    })
    .collect()
}

/// State for a rendered item in the fine-grained For loop.
struct ItemState {
    /// The root node handle for this item.
    node: NodeHandle,
    /// The ForItem data (stored for re-rendering if needed).
    item: ForItem,
    /// Hook state for this item (preserved across re-renders).
    hooks: Vec<HookEntry>,
    /// The render scope for this item (cleaned up on removal).
    scope: Option<RenderScope>,
}

/// Fine-grained list rendering that surgically updates the DOM.
///
/// Uses a comment marker node instead of a wrapper element. List items are
/// inserted as siblings after the marker in the parent, avoiding any
/// interference with CSS flex/grid layouts.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `parent` - The parent node to insert the marker and items into
/// * `each` - A closure that returns the current list of ForItems
/// * `view` - A closure that renders a single item to a NodeHandle
///
/// # Returns
///
/// The comment marker NodeHandle. The caller should NOT append this to
/// the parent — it is already inserted.
pub fn for_each_dom<E, V>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    each: E,
    view: V,
) -> NodeHandle
where
    E: Fn() -> Vec<ForItem> + 'static,
    V: Fn(&ForItem, &mut RenderScope) -> NodeHandle + 'static,
{
    use crate::reconcile::ListOp;

    // Create comment marker and insert into parent
    let marker = scope.create_comment("for");
    parent.append_child(&marker);

    let parent_id = parent.node_id();

    // Get weak doc reference for creating new scopes in Effect
    let doc_weak = scope.doc_weak();

    // Store render function as Rc for sharing with Effect
    let view = Rc::new(view);

    // Track items by key: key -> ItemState
    let items_state: Rc<RefCell<HashMap<String, ItemState>>> =
        Rc::new(RefCell::new(HashMap::new()));
    // Track order of keys
    let keys_order: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let marker_clone = marker.clone();

    // Initial render - insert items as siblings after the marker
    {
        let initial_items = each();
        let mut state = items_state.borrow_mut();
        let mut keys = keys_order.borrow_mut();

        // Track inserted nodes to chain insert_after calls
        let mut initial_nodes: Vec<NodeHandle> = Vec::new();

        for item in initial_items {
            if let Some(doc) = doc_weak.upgrade() {
                let mut child_scope = RenderScope::new(doc, parent_id);
                push_hook_scope(None); // Fresh scope for new item
                let node = with_render_context(|| view(&item, &mut child_scope));
                let hooks = pop_hook_scope();

                if initial_nodes.is_empty() {
                    marker.insert_after(&node);
                } else {
                    initial_nodes.last().unwrap().insert_after(&node);
                }

                keys.push(item.key.clone());
                initial_nodes.push(node.clone());
                state.insert(
                    item.key.clone(),
                    ItemState {
                        node,
                        item,
                        hooks,
                        scope: Some(child_scope),
                    },
                );
            }
        }
    }

    // Create Effect that reconciles list when it changes
    let items_state_clone = items_state.clone();
    let keys_order_clone = keys_order.clone();
    let doc_weak_clone = doc_weak.clone();
    let view_clone = view.clone();
    let marker_effect = marker_clone;

    let effect = Effect::new(move || {
        let new_items = each();

        // Extract current and new keys
        let old_keys: Vec<String> = keys_order_clone.borrow().clone();
        let new_keys: Vec<String> = new_items.iter().map(|i| i.key.clone()).collect();

        // Build a map of new items by key for quick lookup
        let new_items_map: HashMap<String, &ForItem> =
            new_items.iter().map(|i| (i.key.clone(), i)).collect();

        // Keys match — re-render each item with new data
        if old_keys == new_keys {
            let mut state = items_state_clone.borrow_mut();
            for item in &new_items {
                if let Some(old_state) = state.get_mut(&item.key)
                    && let Some(doc) = doc_weak_clone.upgrade()
                {
                    // Dispose old scope (cleans up old effects)
                    if let Some(old_scope) = old_state.scope.take() {
                        old_scope.dispose();
                    }
                    let mut child_scope = RenderScope::new(doc, parent_id);
                    push_hook_scope(Some(std::mem::take(&mut old_state.hooks)));
                    let new_node = with_render_context(|| view_clone(item, &mut child_scope));
                    let hooks = pop_hook_scope();
                    old_state.node.insert_after(&new_node);
                    old_state.node.remove();
                    old_state.node = new_node;
                    old_state.item = item.clone();
                    old_state.hooks = hooks;
                    old_state.scope = Some(child_scope);
                }
            }
            return;
        }

        // Get diff operations
        let ops = diff_keyed(&old_keys, &new_keys);

        // Apply operations
        let mut state = items_state_clone.borrow_mut();
        let mut keys = keys_order_clone.borrow_mut();

        for op in ops {
            match op {
                ListOp::Remove { key, .. } => {
                    // Remove the item's DOM node
                    if let Some(item_state) = state.remove(&key) {
                        if let Some(old_scope) = item_state.scope {
                            old_scope.dispose();
                        }
                        item_state.node.clear_animations();
                        item_state.node.remove();
                    }
                    // Remove from keys order
                    if let Some(pos) = keys.iter().position(|k| k == &key) {
                        keys.remove(pos);
                    }
                }
                ListOp::Insert { key, new_index } => {
                    // Get the item data
                    if let Some(&item) = new_items_map.get(&key)
                        && let Some(doc) = doc_weak_clone.upgrade()
                    {
                        let mut child_scope = RenderScope::new(doc, parent_id);
                        push_hook_scope(None);
                        let node = with_render_context(|| view_clone(item, &mut child_scope));
                        let hooks = pop_hook_scope();

                        // Insert at the correct position as sibling.
                        // Find the node to insert after: either the previous
                        // sibling in the list or the marker itself.
                        if new_index == 0 {
                            marker_effect.insert_after(&node);
                        } else {
                            // Find the key at new_index - 1 in the current keys
                            // (after processing previous ops)
                            let prev_key = if new_index - 1 < keys.len() {
                                Some(keys[new_index - 1].clone())
                            } else if !keys.is_empty() {
                                Some(keys.last().unwrap().clone())
                            } else {
                                None
                            };

                            if let Some(prev_key) = prev_key {
                                if let Some(prev_state) = state.get(&prev_key) {
                                    prev_state.node.insert_after(&node);
                                } else {
                                    marker_effect.insert_after(&node);
                                }
                            } else {
                                marker_effect.insert_after(&node);
                            }
                        }

                        // Update state
                        let insert_pos = new_index.min(keys.len());
                        keys.insert(insert_pos, key.clone());
                        state.insert(
                            key,
                            ItemState {
                                node,
                                item: item.clone(),
                                hooks,
                                scope: Some(child_scope),
                            },
                        );
                    }
                }
                ListOp::Move { key, new_index, .. } => {
                    // Move the existing node to new position
                    if let Some(item_state) = state.get(&key) {
                        let node = &item_state.node;

                        // Find insertion point
                        if new_index == 0 {
                            marker_effect.insert_after(node);
                        } else {
                            // Get the sibling before the target position
                            // We need to look at the current state of keys
                            // after previous operations
                            let prev_key = if new_index - 1 < keys.len() {
                                let k = keys[new_index - 1].clone();
                                // Skip self
                                if k == key {
                                    if new_index >= 2 {
                                        Some(keys[new_index - 2].clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    Some(k)
                                }
                            } else {
                                None
                            };

                            if let Some(prev_key) = prev_key {
                                if let Some(prev_state) = state.get(&prev_key) {
                                    prev_state.node.insert_after(node);
                                } else {
                                    marker_effect.insert_after(node);
                                }
                            } else {
                                marker_effect.insert_after(node);
                            }
                        }

                        // Update keys order
                        if let Some(old_pos) = keys.iter().position(|k| k == &key) {
                            keys.remove(old_pos);
                        }
                        let insert_pos = new_index.min(keys.len());
                        keys.insert(insert_pos, key);
                    }
                }
            }
        }

        // Update keys to match new order
        *keys = new_keys;
    });

    // Attach effect to parent scope
    scope.create_effect_from(effect);

    marker
}

/// Typed list rendering that avoids ForItem boxing at the user level.
///
/// Wraps items into `ForItem` internally and delegates to `for_each_dom`.
/// The user provides a typed collection, key function, and view function
/// without ever seeing `ForItem` or `downcast_ref`.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `parent` - The parent node to insert the marker and items into
/// * `collection` - A closure that returns the current list of items
/// * `key_fn` - A closure that extracts a unique string key from each item
/// * `view` - A closure that renders a single item to a NodeHandle
///
/// # Returns
///
/// The comment marker NodeHandle. Already inserted into parent.
pub fn for_each_dom_typed<T, C, K, V>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    collection: C,
    key_fn: K,
    view: V,
) -> NodeHandle
where
    T: 'static,
    C: Fn() -> Vec<T> + 'static,
    K: Fn(&T) -> String + 'static,
    V: Fn(&T, &mut RenderScope) -> NodeHandle + 'static,
{
    let key_fn = Rc::new(key_fn);
    let view = Rc::new(view);
    let kf = key_fn.clone();

    for_each_dom(
        scope,
        parent,
        move || {
            collection()
                .into_iter()
                .map(|item| {
                    let key = kf(&item);
                    ForItem::new(key, item)
                })
                .collect()
        },
        move |item: &ForItem, scope: &mut RenderScope| {
            let data = item
                .data
                .downcast_ref::<T>()
                .expect("for_each_dom_typed: type mismatch in ForItem downcast");
            view(data, scope)
        },
    )
}

/// Builder for fine-grained For loop rendering.
///
/// Provides a fluent API for building list rendering with RenderScope.
///
/// # Example
///
/// ```ignore
/// let items = use_signal(|| vec![...]);
///
/// FineForBuilder::new(move || items.get())
///     .view(|item, scope| {
///         let data = item.downcast::<MyItem>().unwrap();
///         let div = scope.create_element("div");
///         div.set_text(&data.name);
///         div
///     })
///     .build(scope, &parent)
/// ```
pub struct FineForBuilder<E, V>
where
    E: Fn() -> Vec<ForItem> + Clone + 'static,
    V: Fn(&ForItem, &mut RenderScope) -> NodeHandle + 'static,
{
    each: E,
    view: Option<V>,
}

impl<E> FineForBuilder<E, fn(&ForItem, &mut RenderScope) -> NodeHandle>
where
    E: Fn() -> Vec<ForItem> + Clone + 'static,
{
    /// Create a new FineForBuilder with the given items closure.
    pub fn new(each: E) -> Self {
        FineForBuilder { each, view: None }
    }
}

impl<E, V> FineForBuilder<E, V>
where
    E: Fn() -> Vec<ForItem> + Clone + 'static,
    V: Fn(&ForItem, &mut RenderScope) -> NodeHandle + 'static,
{
    /// Set the view function for rendering items.
    pub fn view<V2>(self, view: V2) -> FineForBuilder<E, V2>
    where
        V2: Fn(&ForItem, &mut RenderScope) -> NodeHandle + 'static,
    {
        FineForBuilder {
            each: self.each,
            view: Some(view),
        }
    }

    /// Build the For loop and return the marker NodeHandle.
    ///
    /// The marker and items are inserted directly into `parent`.
    /// The returned handle should NOT be appended again.
    ///
    /// # Panics
    ///
    /// Panics if `view` was not called.
    pub fn build(self, scope: &mut RenderScope, parent: &NodeHandle) -> NodeHandle {
        let view = self.view.expect("FineForBuilder: view() must be called");
        for_each_dom(scope, parent, self.each, view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Scope;
    use crate::reconcile::{ListOp, diff_keyed};
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Clone, Debug, PartialEq)]
    struct TestItem {
        id: String,
        name: String,
    }

    #[test]
    fn test_for_append_item() {
        // Appending items should only create new nodes
        let old_keys: Vec<&str> = vec!["a", "b", "c"];
        let new_keys: Vec<&str> = vec!["a", "b", "c", "d", "e"];

        let ops = diff_keyed(&old_keys, &new_keys);

        // Should have 2 insertions, no removals or moves
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 2);
        assert!(removes.is_empty());
    }

    #[test]
    fn test_for_remove_item() {
        // Removing items should only remove those nodes
        let old_keys: Vec<&str> = vec!["a", "b", "c", "d"];
        let new_keys: Vec<&str> = vec!["a", "c"];

        let ops = diff_keyed(&old_keys, &new_keys);

        // Should have 2 removals (b and d), no insertions
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();

        assert_eq!(removes.len(), 2);
        assert!(inserts.is_empty());
    }

    #[test]
    fn test_for_reorder_items() {
        // Reordering should move nodes, not recreate
        let old_keys: Vec<&str> = vec!["a", "b", "c"];
        let new_keys: Vec<&str> = vec!["c", "a", "b"];

        let ops = diff_keyed(&old_keys, &new_keys);

        // Should have moves, no insertions or removals
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();
        let moves: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Move { .. }))
            .collect();

        assert!(inserts.is_empty());
        assert!(removes.is_empty());
        assert!(!moves.is_empty());
    }

    #[test]
    fn test_for_mixed_operations() {
        // Mix of add, remove, and reorder
        let old_keys: Vec<&str> = vec!["a", "b", "c", "d"];
        let new_keys: Vec<&str> = vec!["b", "e", "c", "a"];

        let ops = diff_keyed(&old_keys, &new_keys);

        // Should have: remove "d", insert "e", move "a"
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Insert { .. }))
            .collect();
        let removes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, ListOp::Remove { .. }))
            .collect();

        assert_eq!(inserts.len(), 1); // "e"
        assert_eq!(removes.len(), 1); // "d"
    }

    #[test]
    fn test_for_item_downcast() {
        let item = ForItem::new(
            "test",
            TestItem {
                id: "test".to_string(),
                name: "Test Item".to_string(),
            },
        );

        // Successful downcast
        let data = item.downcast::<TestItem>();
        assert!(data.is_some());
        assert_eq!(data.unwrap().id, "test");

        // Failed downcast (wrong type)
        let wrong: Option<&String> = item.downcast::<String>();
        assert!(wrong.is_none());
    }

    #[test]
    fn test_for_scope_cleanup() {
        // Test that item scopes are cleaned up when removed
        let cleanup_count = Rc::new(Cell::new(0));

        // Create scopes for 3 items
        let scopes: Vec<Scope> = (0..3).map(|_| Scope::new()).collect();

        // Add cleanup to each scope
        for scope in &scopes {
            let cleanup_count_clone = cleanup_count.clone();
            scope.on_cleanup(move || {
                cleanup_count_clone.set(cleanup_count_clone.get() + 1);
            });
        }

        // Dispose 2 scopes (simulating item removal)
        scopes[0].dispose();
        scopes[1].dispose();

        assert_eq!(cleanup_count.get(), 2);
    }

    #[test]
    fn test_to_for_items() {
        let data = vec![
            TestItem {
                id: "1".to_string(),
                name: "One".to_string(),
            },
            TestItem {
                id: "2".to_string(),
                name: "Two".to_string(),
            },
        ];

        let items = to_for_items(data.into_iter(), |item| item.id.clone());

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "1");
        assert_eq!(items[1].key, "2");

        // Verify data is preserved
        let item_data = items[0].downcast::<TestItem>().unwrap();
        assert_eq!(item_data.name, "One");
    }
}
