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
//! **Keys must be unique within one list.** The whole reconcile rests on one
//! key naming one `ItemState` and one mounted sibling, with `keys_order` in step
//! with both. An item repeating a key already seen in the same pass is
//! therefore **not rendered**, and a warning is logged — the first occurrence
//! wins, as in React. Note the rsx fallback key when no `key:` prop is given is
//! `format!("{:?}", item)`, so two `Debug`-equal items collide: `for n in
//! vec![1, 1, 2]` renders two rows, not three (issue #185).
//!
//! # Example
//!
//! ```ignore
//! let items = Signal::new(vec![
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

/// Drop every repeat of a key already seen in this batch, keeping the first.
///
/// One key, one item (issue #185). Rendering a repeat used to displace the
/// first occurrence's [`ItemState`] out of `items_state`; dropping the
/// displaced state disposed its [`RenderScope`] — handlers deregistered,
/// signals freed — while its DOM node stayed a sibling of the marker, because
/// [`NodeHandle`] has no `Drop` and nothing else held the node. The result was
/// a row that renders, swallows clicks and never updates again, unreachable
/// from every data structure and so beyond the reach of any later
/// `ListOp::Remove`.
///
/// Dropping the repeat instead keeps the invariant the reconcile rests on: one
/// key, one `ItemState`, one mounted sibling, `keys_order` in step with all
/// three. React does the same ("only the first child will be used"). The
/// tradeoff is that a duplicate now shows up as a missing row — an obviously
/// wrong render that gets reported — rather than as a row that looks right and
/// is silently dead.
///
/// `warned` latches the diagnostic to once per list: [`Effect::new`] runs its
/// body immediately, so `each` is called twice before the user does anything,
/// and every later list change would warn again.
fn dedup_by_key(mut items: Vec<ForItem>, warned: &std::cell::Cell<bool>) -> Vec<ForItem> {
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(items.len());
    items.retain(|item| {
        if seen.insert(item.key.clone()) {
            return true;
        }
        if !warned.replace(true) {
            tracing::warn!(
                "duplicate `for` key {:?}: this item is not rendered. \
                 Give each item a unique `key:`.",
                item.key
            );
        }
        false
    });
    items
}

/// State for a rendered item in the fine-grained For loop.
struct ItemState {
    /// The root node handle for this item.
    node: NodeHandle,
    /// The ForItem data (stored for re-rendering if needed).
    item: ForItem,
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
/// * `eq_fn` - Optional equality function to compare ForItem data.
///   When provided, surviving items whose data changed are re-rendered.
///   When None, surviving items are never re-rendered (old behavior).
///
/// # Duplicate keys
///
/// Keys must be unique. An item whose key repeats one already seen in the same
/// batch is dropped by [`dedup_by_key`] before anything is rendered — first
/// occurrence wins, warning logged (issue #185).
///
/// # Returns
///
/// The comment marker NodeHandle. The caller should NOT append this to
/// the parent — it is already inserted.
#[allow(clippy::type_complexity)]
pub fn for_each_dom<E, V>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    each: E,
    view: V,
    eq_fn: Option<Rc<dyn Fn(&ForItem, &ForItem) -> bool>>,
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

    // One key, one item (issue #185). Wrapping `each` here — rather than
    // patching the two `state.insert` sites — is what makes the guarantee
    // total: `new_keys`, `new_items_map`, the data-comparison pass and the
    // `*keys = new_keys` assignment at the end of the reconcile all read the
    // raw item list, so every one of them would otherwise reintroduce a
    // duplicate that the insert sites had just rejected.
    //
    // This must stay at the `each()` call and never move below the
    // `items_state`/`keys_order` borrows: dropping the skipped `ForItem`s drops
    // their `Rc<dyn Any>` payload, which runs the user type's `Drop` — user
    // code, under a `RefMut` this closure re-enters (issue #141).
    let each = {
        let warned = std::cell::Cell::new(false);
        move || dedup_by_key(each(), &warned)
    };

    // Initial render - insert items as siblings after the marker
    {
        let initial_items = each();
        let mut state = items_state.borrow_mut();
        let mut keys = keys_order.borrow_mut();

        // Track inserted nodes to chain insert_after calls
        let mut initial_nodes: Vec<NodeHandle> = Vec::new();
        // Item states displaced by a duplicate key, dropped after `state` is
        // released — see the note on `state.insert` below.
        let mut displaced: Vec<ItemState> = Vec::new();

        for item in initial_items {
            if let Some(doc) = doc_weak.upgrade() {
                let mut child_scope = RenderScope::new(doc, parent_id);
                // Each item owns what its view creates (issue #141). The guard
                // ends before `state.insert` below, which can displace — and so
                // dispose — a live item scope on a duplicate key.
                let node = {
                    let _owner = child_scope.push_owner();
                    view(&item, &mut child_scope)
                };

                if initial_nodes.is_empty() {
                    marker.insert_after(&node);
                } else {
                    initial_nodes.last().unwrap().insert_after(&node);
                }

                keys.push(item.key.clone());
                initial_nodes.push(node.clone());
                // `each` is deduplicated (issue #185), so nothing can be
                // displaced here. The parking stays as insurance: dropping a
                // displaced `ItemState` inline would dispose its scope, running
                // user code under `state`'s `RefMut` (issue #141).
                let clobbered = state.insert(
                    item.key.clone(),
                    ItemState {
                        node,
                        item,
                        scope: Some(child_scope),
                    },
                );
                debug_assert!(
                    clobbered.is_none(),
                    "for_each_dom: `dedup_by_key` guarantees one `ItemState` per key"
                );
                displaced.extend(clobbered);
            }
        }

        drop(state);
        drop(keys);
        drop(displaced);
    }

    // Create Effect that reconciles list when it changes
    let items_state_clone = items_state.clone();
    let keys_order_clone = keys_order.clone();
    let doc_weak_clone = doc_weak.clone();
    let view_clone = view.clone();
    let eq_fn_clone = eq_fn;
    let marker_effect = marker_clone;

    let effect = Effect::new(move || {
        let new_items = each();

        // Extract current and new keys
        let old_keys: Vec<String> = keys_order_clone.borrow().clone();
        let new_keys: Vec<String> = new_items.iter().map(|i| i.key.clone()).collect();

        // Build a map of new items by key for quick lookup
        let new_items_map: HashMap<String, &ForItem> =
            new_items.iter().map(|i| (i.key.clone(), i)).collect();

        // Always use diff_keyed to compute minimal operations
        let ops = diff_keyed(&old_keys, &new_keys);

        // Scopes displaced by this reconcile, torn down at the very end of the
        // closure once `state` and `keys` are no longer borrowed.
        //
        // Disposal runs user code — cleanups, handler-closure drops, signal
        // value drops (issue #141) — and any of it that writes a signal flushes
        // effects synchronously, re-entering this very closure. Disposing under
        // the `RefMut`s below would make that a `BorrowMutError` rather than a
        // reconcile. The cost is that a removed item's effects stay live for the
        // remainder of this pass; they have no signal to wake them in that
        // window, since nothing re-enters the flush before the drop.
        let mut doomed: Vec<RenderScope> = Vec::new();

        // Apply operations
        let mut state = items_state_clone.borrow_mut();
        let mut keys = keys_order_clone.borrow_mut();

        // Track which keys were freshly inserted (skip them in data comparison)
        let mut inserted_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

        for op in ops {
            match op {
                ListOp::Remove { key, .. } => {
                    // Remove the item's DOM node
                    if let Some(item_state) = state.remove(&key) {
                        doomed.extend(item_state.scope);
                        item_state.node.clear_animations();
                        item_state.node.remove();
                    }
                    // Remove from keys order
                    if let Some(pos) = keys.iter().position(|k| k == &key) {
                        keys.remove(pos);
                    }
                }
                ListOp::Insert { key, new_index } => {
                    inserted_keys.insert(key.clone());

                    // Get the item data
                    if let Some(&item) = new_items_map.get(&key)
                        && let Some(doc) = doc_weak_clone.upgrade()
                    {
                        let mut child_scope = RenderScope::new(doc, parent_id);
                        // Wrap in untracked so signal reads during view rendering
                        // don't subscribe the for-loop's parent effect. Items create
                        // their own effects for reactivity via {|| expr} closures.
                        //
                        // The owner guard is a separate stack from `untracked`'s
                        // observer stack (issue #141). It must stay inside this
                        // match arm: the `Remove` arm disposes scopes, and that
                        // must run under the reconcile effect's owner.
                        let node = {
                            let _owner = child_scope.push_owner();
                            crate::reactive::untracked(|| view_clone(item, &mut child_scope))
                        };

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

                        // Update state. `each` is deduplicated (issue #185), so
                        // nothing can be displaced here; park anything that is
                        // rather than letting it drop — and so dispose — under
                        // the `state` borrow (#141).
                        let insert_pos = new_index.min(keys.len());
                        keys.insert(insert_pos, key.clone());
                        let clobbered = state.insert(
                            key,
                            ItemState {
                                node,
                                item: item.clone(),
                                scope: Some(child_scope),
                            },
                        );
                        debug_assert!(
                            clobbered.is_none(),
                            "for_each_dom: `dedup_by_key` guarantees one `ItemState` per key"
                        );
                        doomed.extend(clobbered.and_then(|c| c.scope));
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

        // Data comparison pass: re-render surviving items whose data changed.
        // This ensures that when a todo's `completed` field changes, the item
        // gets fresh closures with the new value.
        if let Some(ref eq_fn) = eq_fn_clone {
            for item in &new_items {
                // Skip freshly inserted items — they were just rendered
                if inserted_keys.contains(&item.key) {
                    continue;
                }
                if let Some(old_state) = state.get_mut(&item.key) {
                    // Compare old data vs new data
                    if !eq_fn(&old_state.item, item) {
                        // Data changed — re-render this item
                        if let Some(doc) = doc_weak_clone.upgrade() {
                            doomed.extend(old_state.scope.take());
                            let mut child_scope = RenderScope::new(doc, parent_id);
                            // The re-rendered item owns its new resources
                            // (issue #141); the old scope was disposed above,
                            // under the reconcile effect's owner.
                            let new_node = {
                                let _owner = child_scope.push_owner();
                                crate::reactive::untracked(|| view_clone(item, &mut child_scope))
                            };
                            old_state.node.insert_after(&new_node);
                            old_state.node.remove();
                            old_state.node = new_node;
                            old_state.item = item.clone();
                            old_state.scope = Some(child_scope);
                        }
                    }
                }
            }
        }

        // Update keys to match new order
        *keys = new_keys;

        // Borrows released before the parked scopes are torn down.
        drop(state);
        drop(keys);
        for scope in doomed {
            scope.dispose();
        }
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
/// Items with matching keys whose data has changed (via `PartialEq`) are
/// automatically re-rendered with fresh closures. Items whose data is
/// unchanged keep their existing DOM nodes and state.
///
/// The view function receives an **owned** `T`, so loop variables can be
/// captured directly in `move` closures without manual extraction.
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
    T: Clone + PartialEq + 'static,
    C: Fn() -> Vec<T> + 'static,
    K: Fn(&T) -> String + 'static,
    V: Fn(T, &mut RenderScope) -> NodeHandle + 'static,
{
    let key_fn = Rc::new(key_fn);
    let view = Rc::new(view);
    let kf = key_fn.clone();

    // Build PartialEq-based equality function for data comparison
    #[allow(clippy::type_complexity)]
    let eq_fn: Rc<dyn Fn(&ForItem, &ForItem) -> bool> =
        Rc::new(|a: &ForItem, b: &ForItem| {
            match (a.data.downcast_ref::<T>(), b.data.downcast_ref::<T>()) {
                (Some(a_data), Some(b_data)) => a_data == b_data,
                _ => false,
            }
        });

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
            view(data.clone(), scope)
        },
        Some(eq_fn),
    )
}

/// Builder for fine-grained For loop rendering.
///
/// Provides a fluent API for building list rendering with RenderScope.
///
/// # Example
///
/// ```ignore
/// let items = Signal::new(vec![...]);
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
        for_each_dom(scope, parent, self.each, view, None)
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

    /// Each item body is attributed to that item's own child scope, across all
    /// three of the `for` loop's render sites: the initial build, an `Insert`
    /// during reconcile, and a data-change re-render (issue #141).
    ///
    /// Also proves the item guards nest correctly *inside* `run_effect`'s push —
    /// the reconcile effect is itself running with its own owner ambient when
    /// sites 7 and 8 fire.
    #[test]
    fn an_item_body_is_attributed_to_its_own_item_scope() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Owner, Signal, current_owner};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec![TestItem {
            id: "a".into(),
            name: "A".into(),
        }]);
        #[allow(clippy::type_complexity)]
        let seen: Rc<RefCell<Vec<(String, Option<Owner>)>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |item: &TestItem| item.id.clone(),
            move |item: TestItem, s: &mut RenderScope| {
                log.borrow_mut().push((item.name.clone(), current_owner()));
                Signal::new(0);
                s.create_element("div")
            },
        );
        let _ = marker;

        // Site 7: reconcile inserts a new key.
        items.update(|v| {
            v.push(TestItem {
                id: "b".into(),
                name: "B".into(),
            })
        });
        // Site 8: an existing key's data changes, forcing a re-render.
        items.update(|v| v[0].name = "A2".into());

        let seen = seen.borrow();
        let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["A", "B", "A2"], "all three render sites fired");

        let owners: Vec<Owner> = seen
            .iter()
            .map(|(n, o)| o.clone().unwrap_or_else(|| panic!("{n} ran with no owner")))
            .collect();

        for (i, owner) in owners.iter().enumerate() {
            assert_ne!(
                *owner,
                scope.owner(),
                "render {i} must not be attributed to the parent scope"
            );
            for (j, other) in owners.iter().enumerate().skip(i + 1) {
                assert_ne!(*owner, *other, "renders {i} and {j} must have own scopes");
            }
        }

        assert!(
            !owners[0].is_alive(),
            "the re-rendered item's original scope was disposed"
        );
        assert_eq!(
            owners[1].owned_counts().map(|c| c.signals),
            Some(1),
            "the inserted item owns its own signal"
        );
        assert_eq!(
            owners[2].owned_counts().map(|c| c.signals),
            Some(1),
            "the re-rendered item owns its own signal"
        );
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

    /// Removing an item whose teardown writes a signal must not deadlock the
    /// reconcile on its own `RefCell`s (issue #141).
    ///
    /// Disposal now runs user code — cleanups, handler-closure drops, signal
    /// value drops — and a write from any of it flushes effects synchronously,
    /// re-entering this very reconcile closure. Disposing under the
    /// `items_state`/`keys_order` borrows makes that a `BorrowMutError`; parking
    /// the doomed scopes until both are released makes it a no-op re-entry.
    ///
    /// Counterfactual: replace the `doomed` vec at the `ListOp::Remove` arm with
    /// an inline `old_scope.dispose()` and this panics with
    /// "already mutably borrowed".
    #[test]
    fn removing_an_item_whose_cleanup_writes_a_signal_does_not_panic() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec![
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ]);
        // Written from each item's cleanup, and read by the list itself — so the
        // write re-enters the reconcile effect rather than merely waking a
        // bystander.
        let churn = Signal::new(0);

        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || {
                churn.get();
                items.get()
            },
            |item: &TestItem| item.id.clone(),
            move |_item: TestItem, s: &mut RenderScope| {
                s.on_cleanup(move || churn.update(|n| *n += 1));
                s.create_element("div")
            },
        );
        let _ = marker;

        // Drops item "a": its cleanup writes `churn`, whose flush re-enters this
        // reconcile.
        items.update(|v| v.retain(|i| i.id == "b"));

        assert_eq!(churn.get(), 1, "exactly the removed item's cleanup ran");
    }

    // -----------------------------------------------------------------
    // Duplicate keys (issue #185)
    // -----------------------------------------------------------------

    /// The `data-name` of every mounted row, in sibling order.
    ///
    /// Reads an attribute rather than the text because
    /// `MockDomDocument::text_content` ignores an element's own `text` field and
    /// concatenates its descendants, so `set_text` on a `<div>` reads back as
    /// `""`. The `for` marker is a comment node (`node_type() == Some(8)`) and is
    /// filtered out.
    fn mounted_row_names(parent: &NodeHandle) -> Vec<String> {
        parent
            .children()
            .into_iter()
            .filter(|child| child.node_type() != Some(8))
            .map(|child| child.get_attribute("data-name").unwrap_or_default())
            .collect()
    }

    /// The node id of every mounted row, in sibling order.
    fn mounted_row_ids(parent: &NodeHandle) -> Vec<usize> {
        parent
            .children()
            .into_iter()
            .filter(|child| child.node_type() != Some(8))
            .map(|child| child.node_id().0)
            .collect()
    }

    /// One key, one row: a repeated key is not rendered at all, and the *first*
    /// occurrence is the one that survives (issue #185).
    ///
    /// Keeping the first matches React ("only the first child will be used") and
    /// is the opposite of what displacing the earlier `ItemState` used to do —
    /// that kept the last.
    #[test]
    fn duplicate_key_renders_one_row_per_key() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = vec![
            TestItem {
                id: "a".into(),
                name: "A1".into(),
            },
            TestItem {
                id: "a".into(),
                name: "A2".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ];

        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.clone(),
            |item: &TestItem| item.id.clone(),
            |item: TestItem, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
        );
        let _ = marker;

        assert_eq!(
            mounted_row_names(&parent),
            ["A1", "B"],
            "one row per key, first occurrence wins"
        );
    }

    /// The title of issue #185: a duplicate key must never leave a mounted DOM
    /// node whose scope has been disposed.
    ///
    /// Rendering the repeat used to displace the first item's `ItemState` out of
    /// `items_state`; dropping the displaced state disposed its `RenderScope`
    /// (handlers deregistered, signals freed) while its node stayed a sibling of
    /// the marker — a row that renders, swallows clicks and never updates again.
    /// `NodeHandle` has no `Drop`, so nothing unmounted it, and no later
    /// `ListOp::Remove` could ever reach it.
    ///
    /// The fix skips the repeat before a scope is ever created, so this asserts
    /// **two** view invocations. Under the rejected "render it, then unmount the
    /// displaced node" variant it would be three, with the first owner dead and
    /// its node gone; if that variant is ever adopted this test has to change.
    #[test]
    fn a_duplicate_key_never_leaves_a_dead_scope_mounted() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Owner, Signal, current_owner};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = vec![
            TestItem {
                id: "a".into(),
                name: "A1".into(),
            },
            TestItem {
                id: "a".into(),
                name: "A2".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ];
        #[allow(clippy::type_complexity)]
        let seen: Rc<RefCell<Vec<(String, Option<Owner>, usize)>>> =
            Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.clone(),
            |item: &TestItem| item.id.clone(),
            move |item: TestItem, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                // A resource owned by the item's scope: freed if it is disposed.
                Signal::new(0);
                log.borrow_mut()
                    .push((item.name.clone(), current_owner(), node.node_id().0));
                node
            },
        );
        let _ = marker;

        let seen = seen.borrow();
        let names: Vec<&str> = seen.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, ["A1", "B"], "the repeated key is never rendered");

        for (name, owner, _) in seen.iter() {
            let owner = owner
                .clone()
                .unwrap_or_else(|| panic!("{name} ran with no owner"));
            assert!(
                owner.is_alive(),
                "{name}'s scope must still be live while its node is mounted"
            );
        }

        let mut mounted = mounted_row_ids(&parent);
        let mut rendered: Vec<usize> = seen.iter().map(|(_, _, id)| *id).collect();
        mounted.sort_unstable();
        rendered.sort_unstable();
        assert_eq!(
            mounted, rendered,
            "every mounted node belongs to a live view invocation"
        );
    }

    /// A duplicate must not desynchronise `keys_order` from `items_state` and the
    /// DOM, because both insert paths index `keys_order` *positionally* to find
    /// the sibling to insert after (issue #185).
    ///
    /// This is the test that distinguishes a real fix from "unmount the displaced
    /// node but leave `keys_order` alone": with `keys == ["a", "a", "b"]` the
    /// later `Insert { "c", new_index: 2 }` reads `prev_key = keys[1] == "a"` and
    /// puts `C` after `A` instead of after `B`. Nothing ever repairs that — a
    /// later diff only emits a `Move` when the *key* order changes, and the key
    /// order is already correct.
    #[test]
    fn duplicate_keys_do_not_desynchronise_keys_order() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec![
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ]);

        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |item: &TestItem| item.id.clone(),
            |item: TestItem, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
        );
        let _ = marker;

        // The duplicate goes away and a fresh key arrives at the end.
        items.set(vec![
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
            TestItem {
                id: "c".into(),
                name: "C".into(),
            },
        ]);

        assert_eq!(
            mounted_row_names(&parent),
            ["A", "B", "C"],
            "an insert after a duplicate pass still lands in the right slot"
        );
    }

    /// A duplicate introduced by an update must not make every later reconcile
    /// re-render the shared slot (issue #185).
    ///
    /// The data-comparison pass walks the raw item list, duplicates included, and
    /// looks each one up by key. Two same-key items with different data therefore
    /// disagree with the stored item in turn, so the slot is re-rendered — and its
    /// scope disposed — twice per pass, forever. No `state.insert` collision
    /// happens on that path, so neither of the old `tracing::warn!`s ever fired
    /// for it.
    #[test]
    fn a_duplicate_key_introduced_by_an_update_does_not_thrash() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec![
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ]);
        // Read by the list, so bumping it forces another reconcile pass.
        let churn = Signal::new(0);
        let renders = Rc::new(Cell::new(0usize));

        let count = renders.clone();
        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || {
                churn.get();
                items.get()
            },
            |item: &TestItem| item.id.clone(),
            move |item: TestItem, s: &mut RenderScope| {
                count.set(count.get() + 1);
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
        );
        let _ = marker;

        assert_eq!(renders.get(), 2, "one render per item on the initial build");

        // Introduce a second item keyed "b" with different data.
        items.update(|v| {
            v.push(TestItem {
                id: "b".into(),
                name: "B2".into(),
            })
        });
        assert_eq!(
            renders.get(),
            2,
            "the repeat is dropped, so nothing is re-rendered"
        );

        // Any later pass must not re-render either.
        churn.update(|n| *n += 1);
        assert_eq!(renders.get(), 2, "and it stays dropped on every later pass");

        assert_eq!(mounted_row_names(&parent), ["A", "B"]);
    }

    /// The `key:`-less case: two `Debug`-equal items collide (issue #185).
    ///
    /// With no `key:` prop the rsx macro keys items by `format!("{:?}", item)`
    /// (`crates/rinch-macros/src/dom_codegen/control_flow.rs`), which this
    /// mirrors directly — the macro expands to `rinch::core::…` paths and cannot
    /// be invoked from inside `rinch-core`. So `for n in vec![1, 1, 2]` renders
    /// two rows, not three, and neither of them is dead.
    #[test]
    fn an_unkeyed_for_drops_a_duplicate_debug_representation() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Owner, current_owner};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = vec![
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "a".into(),
                name: "A".into(),
            },
            TestItem {
                id: "b".into(),
                name: "B".into(),
            },
        ];
        let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.clone(),
            // Exactly the macro's fallback key.
            |item: &TestItem| format!("{item:?}"),
            move |item: TestItem, s: &mut RenderScope| {
                log.borrow_mut().push(current_owner());
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
        );
        let _ = marker;

        assert_eq!(mounted_row_names(&parent), ["A", "B"]);

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "the Debug-equal repeat is never rendered");
        for (i, owner) in seen.iter().enumerate() {
            let owner = owner
                .clone()
                .unwrap_or_else(|| panic!("row {i} had no owner"));
            assert!(owner.is_alive(), "row {i}'s scope must still be live");
        }
    }

    /// The dedup lives in `for_each_dom`, not in `for_each_dom_typed`, so the
    /// public direct callers (`FineForBuilder::build`, and hand-built
    /// `Vec<ForItem>` call sites) are covered too (issue #185).
    ///
    /// `eq_fn: None` is the `FineForBuilder` shape — no data-comparison pass at
    /// all, so this pins the initial-render site on its own.
    #[test]
    fn for_each_dom_deduplicates_hand_built_for_items() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let marker = super::for_each_dom(
            &mut scope,
            &parent,
            || {
                vec![
                    ForItem::new("a", "A1".to_string()),
                    ForItem::new("a", "A2".to_string()),
                    ForItem::new("b", "B".to_string()),
                ]
            },
            |item: &ForItem, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", item.downcast::<String>().unwrap());
                node
            },
            None,
        );
        let _ = marker;

        assert_eq!(mounted_row_names(&parent), ["A1", "B"]);
    }
}
