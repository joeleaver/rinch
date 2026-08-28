//! Virtual list rendering for large datasets.
//!
//! Renders only the visible items in a scrollable container, keeping DOM node
//! count constant regardless of list size. This enables smooth scrolling
//! through 10k–100k+ items without creating a DOM node per item.
//!
//! # DOM Structure
//!
//! ```text
//! <div class="rinch-vlist" style="overflow-y:auto; height:100%; position:relative">
//!   <div class="rinch-vlist__spacer" style="height:{total_height}px">
//!   </div>
//!   <div class="rinch-vlist__window" style="position:absolute; top:0; left:0; right:0;
//!        transform:translateY({offset}px)">
//!     <!-- only visible items rendered here -->
//!   </div>
//! </div>
//! ```
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! let items = Signal::new((0..100_000).map(|i| format!("Item {i}")).collect::<Vec<_>>());
//!
//! rsx! {
//!     div {
//!         style: "height: 400px;",
//!         {virtual_list(
//!             __scope,
//!             36.0,
//!             move || items.get(),
//!             |item: &String| item.clone(),
//!             5,
//!             |item: String, __scope: &mut RenderScope| rsx! {
//!                 div { {item} }
//!             },
//!         )}
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::dom::{NodeHandle, RenderScope};
use crate::reactive::{Effect, Signal};

/// State for a single rendered item in the virtual list.
struct RenderedItem {
    node: NodeHandle,
    scope: Option<RenderScope>,
}

/// Render a virtualized list that only creates DOM nodes for visible items.
///
/// Returns a `NodeHandle` for the scroll container. The caller should set a
/// fixed height on the container (or its parent) so the list knows how many
/// items to render.
///
/// # Arguments
///
/// * `scope` - The render scope for creating DOM nodes
/// * `item_height` - Fixed height in pixels for each item
/// * `items` - Reactive closure returning the current list of items
/// * `key` - Function to extract a unique key from each item. Keys must be
///   unique: within one visible range a repeat is dropped, keeping the first,
///   with a warning (issue #185). The dropped row's slot is held open by a
///   spacer, so the surviving rows stay at the y the scrollbar promises
/// * `overscan` - Number of extra items to render above/below the viewport
/// * `view` - Function to render a single item to a `NodeHandle`
///
/// # Type Parameters
///
/// * `T` - Item type (must be `Clone + PartialEq + 'static`)
/// * `C` - Items closure type
/// * `K` - Key extractor closure type
/// * `KV` - Key value type (must be `ToString`)
/// * `V` - View closure type
pub fn virtual_list<T, C, K, KV, V>(
    scope: &mut RenderScope,
    item_height: f64,
    items: C,
    key: K,
    overscan: usize,
    view: V,
) -> NodeHandle
where
    T: Clone + PartialEq + 'static,
    C: Fn() -> Vec<T> + 'static,
    K: Fn(&T) -> KV + 'static,
    KV: ToString,
    V: Fn(T, &mut RenderScope) -> NodeHandle + 'static,
{
    // Build the 3-layer DOM structure
    let container = scope.create_element("div");
    container.set_attribute("class", "rinch-vlist");
    container.set_attribute("style", "overflow-y:auto;height:100%;position:relative");

    let spacer = scope.create_element("div");
    spacer.set_attribute("class", "rinch-vlist__spacer");
    spacer.set_attribute("style", "height:0px");
    container.append_child(&spacer);

    let window = scope.create_element("div");
    window.set_attribute("class", "rinch-vlist__window");
    window.set_attribute(
        "style",
        "position:absolute;top:0;left:0;right:0;transform:translateY(0px)",
    );
    container.append_child(&window);

    // Signal for scroll position — updated by the scroll handler
    let scroll_top = Signal::new(0.0_f64);

    // Register scroll handler on the container
    let scroll_signal = scroll_top;
    let handler_id = scope.register_scroll_handler(move |ev: crate::events::ScrollEvent| {
        // Vertical windowing only: the row range is a function of `scroll_top`.
        scroll_signal.set(ev.scroll_top);
    });
    container.set_attribute("data-onscroll", &handler_id.0.to_string());

    // Internal state for rendered items
    let rendered: Rc<RefCell<HashMap<String, RenderedItem>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let keys_order: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let key = Rc::new(key);
    let view = Rc::new(view);
    // Reusable filler nodes standing in for rows a duplicate key cost the range.
    // Empty for every list with unique keys, which is every healthy list.
    let gap_nodes: RefCell<Vec<NodeHandle>> = RefCell::new(Vec::new());
    let doc_weak = scope.doc_weak();
    let window_id = window.node_id();
    let container_handle = container.clone();
    let spacer_handle = spacer.clone();
    let window_handle = window.clone();
    // Which duplicate keys have already been reported. Per *key*, not one shot
    // per list: the effect re-runs on every scroll tick, so a one-shot latch
    // would be spent by the first collision the list ever scrolls past and a
    // different one later would drop a row in silence. Cleared by any pass with
    // no duplicates, which bounds the set to one visible range's worth of keys
    // and makes a collision that comes back after going away newsworthy again.
    let warned_duplicate_keys: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());

    let effect = Effect::new(move || {
        // Read both scroll position and items — tracks both signals
        let st = scroll_top.get();
        let all_items = items();
        let total = all_items.len();

        // Update spacer height
        let total_height = total as f64 * item_height;
        spacer_handle.set_attribute("style", &format!("height:{}px", total_height));

        // Get viewport height from the container
        let viewport_h = container_handle.client_height();

        // Compute visible range
        let (start, end) = if viewport_h <= 0.0 {
            // Viewport not yet laid out — render a reasonable default
            let default_count = 20.min(total);
            (0, default_count)
        } else {
            let raw_start = (st / item_height).floor() as usize;
            let raw_end = ((st + viewport_h) / item_height).ceil() as usize;
            let end = (raw_end + overscan).min(total);
            // Clamp against `end`, not just against `overscan`: `scroll_top` is
            // a signal that survives the item list shrinking under it, so a
            // scrolled list that is replaced by a shorter one arrives here with
            // `raw_start` far past `total` and `all_items[start..end]` would
            // panic with "slice index starts at N but ends at M".
            let start = raw_start.saturating_sub(overscan).min(end);
            (start, end)
        };

        // Build the key list and the by-key lookup for the new visible range.
        //
        // One key, one row (issue #185): a repeat of a key already in this range
        // is dropped, keeping the first — the same rule `for_each_dom` applies.
        // These two used to be built independently, so `new_keys` kept
        // duplicates while `new_items_by_key` collapsed them (last wins). The
        // "render the keys that are new" loop below only consults the *old* key
        // set, so it rendered such a key once per occurrence; the second
        // render's `state.insert` then displaced the first `RenderedItem` and
        // dropped it inline, under the `rendered` borrow — disposing a live
        // scope, and so running user cleanups, exactly where this closure parks
        // scopes to avoid doing that. The re-append loop finished the job by
        // appending the one surviving node once per occurrence.
        // `start <= end` by construction above; `saturating_sub` keeps a future
        // change to that clamp from turning into a subtract-overflow panic here.
        let visible = end.saturating_sub(start);
        let mut new_keys: Vec<String> = Vec::with_capacity(visible);
        // Each retained key's slot within the range (its item index minus
        // `start`). A dropped duplicate leaves a hole here, and the re-append
        // loop below fills the hole with a spacer of exactly its height — see
        // `gap_nodes`. Without that, rows are appended contiguously while the
        // window still sits at `start * item_height`, so every row past the drop
        // paints one `item_height` too high and the range ends short.
        let mut key_slots: Vec<usize> = Vec::with_capacity(visible);
        let mut new_items_by_key: HashMap<String, &T> = HashMap::with_capacity(visible);
        for (slot, item) in all_items[start..end].iter().enumerate() {
            let k = key(item).to_string();
            if new_items_by_key.contains_key(&k) {
                if warned_duplicate_keys.borrow_mut().insert(k.clone()) {
                    tracing::warn!(
                        "duplicate virtual-list key {:?}: this item is not rendered. \
                         Give each item a unique key.",
                        k
                    );
                }
                continue;
            }
            new_keys.push(k.clone());
            key_slots.push(slot);
            new_items_by_key.insert(k, item);
        }
        if new_keys.len() == visible {
            warned_duplicate_keys.borrow_mut().clear();
        }

        let mut state = rendered.borrow_mut();
        let mut old_keys = keys_order.borrow_mut();

        // Determine which keys to remove (in old but not in new)
        let new_key_set: std::collections::HashSet<&String> = new_keys.iter().collect();
        let to_remove: Vec<String> = old_keys
            .iter()
            .filter(|k| !new_key_set.contains(k))
            .cloned()
            .collect();

        // Remove out-of-range items.
        //
        // Their scopes are parked and disposed at the very end of this closure,
        // once `state` and `old_keys` are no longer borrowed. Disposal runs user
        // code — cleanups, handler-closure drops, signal value drops (issue
        // #141) — and any of it that writes a signal flushes effects
        // synchronously, re-entering this closure and panicking on the
        // outstanding `RefMut`s.
        let mut doomed: Vec<RenderScope> = Vec::new();
        for k in &to_remove {
            if let Some(item_state) = state.remove(k) {
                doomed.extend(item_state.scope);
                item_state.node.remove();
            }
        }

        // Determine which keys are new (in new but not in old)
        let old_key_set: std::collections::HashSet<&String> =
            old_keys.iter().filter(|k| !to_remove.contains(k)).collect();

        // Rebuild the window children in the correct order
        // For simplicity and correctness, we clear the window and re-append
        // in the right order. Since we're only dealing with ~viewport items,
        // this is cheap.

        // First, render any new items
        for k in &new_keys {
            if !old_key_set.contains(k)
                && let Some(&item_data) = new_items_by_key.get(k)
                && let Some(doc) = doc_weak.upgrade()
            {
                let mut child_scope = RenderScope::new(doc, window_id);
                // Each virtualized row owns what its view creates, so scrolling
                // it out of the window takes them with it (issue #141).
                let node = {
                    let _owner = child_scope.push_owner();
                    view(item_data.clone(), &mut child_scope)
                };
                state.insert(
                    k.clone(),
                    RenderedItem {
                        node,
                        scope: Some(child_scope),
                    },
                );
            }
        }

        // Re-append all nodes in the correct order
        // We use a simple approach: collect current children, then re-append in order
        //
        // Rows flow contiguously inside a window positioned at
        // `start * item_height`, so slot N of the range must be the Nth
        // `item_height` of the window. A duplicate key dropped above breaks that
        // by one slot per drop; a spacer of the missing height restores it, so
        // the surviving rows stay at the y the scrollbar promises and the range
        // still covers the viewport. Spacers are pooled and reused across
        // passes — a healthy list never allocates one.
        let mut gaps_used = 0usize;
        let mut next_slot = 0usize;
        for (k, &slot) in new_keys.iter().zip(key_slots.iter()) {
            if slot > next_slot {
                let gap = {
                    let mut pool = gap_nodes.borrow_mut();
                    if gaps_used == pool.len()
                        && let Some(doc) = doc_weak.upgrade()
                    {
                        let mut s = RenderScope::new(doc, window_id);
                        let node = s.create_element("div");
                        node.set_attribute("class", "rinch-vlist__gap");
                        pool.push(node);
                    }
                    pool.get(gaps_used).cloned()
                };
                if let Some(gap) = gap {
                    gap.set_attribute(
                        "style",
                        &format!("height:{}px", (slot - next_slot) as f64 * item_height),
                    );
                    window_handle.append_child(&gap);
                    gaps_used += 1;
                }
            }
            if let Some(item_state) = state.get(k) {
                window_handle.append_child(&item_state.node);
            }
            next_slot = slot + 1;
        }
        // A hole at the *end* of the range displaces nothing (the window is
        // absolutely positioned and the spacer div, not the window, drives the
        // scroll height), so it needs no filler. Spacers left over from a
        // previous pass do need unmounting: `append_child` re-parents, so an
        // unused one would otherwise stay where the last pass put it.
        for gap in gap_nodes.borrow().iter().skip(gaps_used) {
            gap.remove();
        }

        // Update the window transform
        let offset = start as f64 * item_height;
        window_handle.set_attribute(
            "style",
            &format!(
                "position:absolute;top:0;left:0;right:0;transform:translateY({}px)",
                offset
            ),
        );

        // Update keys order
        *old_keys = new_keys;

        // Borrows released before the parked scopes are torn down.
        drop(state);
        drop(old_keys);
        for scope in doomed {
            scope.dispose();
        }
    });

    scope.create_effect_from(effect);

    container
}

#[cfg(test)]
mod tests {
    use crate::dom::traits::DomDocument;
    use crate::dom::{RenderScope, mock::MockDomDocument};
    use crate::reactive::{Owner, Signal, current_owner};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Each virtualized row is attributed to its own child scope, so scrolling a
    /// row out of the window takes its resources with it (issue #141).
    #[test]
    fn a_virtualized_row_is_attributed_to_its_own_child_scope() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let items = Signal::new(vec![1u32, 2, 3]);
        let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let list = super::virtual_list(
            &mut scope,
            20.0,
            move || items.get(),
            |item: &u32| *item,
            1,
            move |item: u32, s: &mut RenderScope| {
                log.borrow_mut().push(current_owner());
                Signal::new(item);
                s.create_element("div")
            },
        );
        let _ = list;

        let seen = seen.borrow();
        assert!(!seen.is_empty(), "at least one row rendered");

        for (i, owner) in seen.iter().enumerate() {
            let owner = owner
                .clone()
                .unwrap_or_else(|| panic!("row {i} had no owner"));
            assert_ne!(
                owner,
                scope.owner(),
                "row {i} must not be attributed to the list's own scope"
            );
            assert_eq!(
                owner.owned_counts().map(|c| c.signals),
                Some(1),
                "row {i} owns the signal its view created"
            );
        }
        assert_eq!(
            scope.owned_counts().signals,
            0,
            "no row signal leaked into the parent scope"
        );
    }

    /// One key, one row — the virtual list must not render a repeated key twice
    /// (issue #185).
    ///
    /// `new_keys` kept duplicates while `new_items_by_key` collapsed them, so the
    /// "render the keys that are new" loop rendered the same key once per
    /// occurrence. The second render's `state.insert` displaced the first
    /// `RenderedItem` and dropped it **inline, under the `rendered` `RefMut`** —
    /// disposing a live scope, and so running user cleanups, exactly where this
    /// module parks scopes to avoid doing so. The re-append loop then appended
    /// the one surviving node once per occurrence.
    #[test]
    fn a_virtual_list_renders_one_row_per_key() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let items = Signal::new(vec![
            (1u32, "A1".to_string()),
            (1u32, "A2".to_string()),
            (2u32, "B".to_string()),
        ]);
        #[allow(clippy::type_complexity)]
        let seen: Rc<RefCell<Vec<(String, Option<Owner>)>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let list = super::virtual_list(
            &mut scope,
            20.0,
            move || items.get(),
            |item: &(u32, String)| item.0,
            1,
            move |item: (u32, String), s: &mut RenderScope| {
                log.borrow_mut().push((item.1.clone(), current_owner()));
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.1);
                node
            },
        );

        // container -> [spacer, window]; the rows live in the window.
        let window = list.children().remove(1);
        assert_eq!(
            row_names(&window),
            ["A1", "B"],
            "one row per key, first occurrence wins"
        );

        let seen = seen.borrow();
        let rendered: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(rendered, ["A1", "B"], "the repeated key is never rendered");
        for (name, owner) in seen.iter() {
            let owner = owner
                .clone()
                .unwrap_or_else(|| panic!("{name} ran with no owner"));
            assert!(
                owner.is_alive(),
                "{name}'s scope must still be live while its row is mounted"
            );
        }
    }

    /// The `data-name` of every real row in the window, in order — gap fillers
    /// (`rinch-vlist__gap`) excluded.
    fn row_names(window: &crate::dom::NodeHandle) -> Vec<String> {
        window
            .children()
            .into_iter()
            .filter(|n| n.get_attribute("class").as_deref() != Some("rinch-vlist__gap"))
            .map(|row| row.get_attribute("data-name").unwrap_or_default())
            .collect()
    }

    /// A duplicate key costs a row, but it must not cost the row's **slot**
    /// (issue #185).
    ///
    /// Rows flow contiguously inside a window positioned at
    /// `start * item_height`, so slot N of the visible range has to be the Nth
    /// `item_height` of the window. Simply skipping the duplicate compacted the
    /// list: every row after the drop painted one `item_height` too high and the
    /// range ended one row short of the viewport, leaving a blank strip at the
    /// bottom. A filler of exactly the missing height holds the slot open.
    #[test]
    fn a_dropped_duplicate_key_holds_its_slot_open() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let items = Signal::new(vec![
            (1u32, "A1".to_string()),
            (1u32, "A2".to_string()),
            (2u32, "B".to_string()),
        ]);

        let list = super::virtual_list(
            &mut scope,
            20.0,
            move || items.get(),
            |item: &(u32, String)| item.0,
            1,
            |item: (u32, String), s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.1);
                node
            },
        );

        let window = list.children().remove(1);
        let children = window.children();
        assert_eq!(children.len(), 3, "two rows plus one filler for the hole");
        assert_eq!(
            children[0].get_attribute("data-name").as_deref(),
            Some("A1")
        );
        assert_eq!(
            children[1].get_attribute("class").as_deref(),
            Some("rinch-vlist__gap"),
            "slot 1 belongs to the dropped duplicate and is held open"
        );
        assert_eq!(
            children[1].get_attribute("style").as_deref(),
            Some("height:20px"),
            "exactly one item_height, so B stays at its own y"
        );
        assert_eq!(children[2].get_attribute("data-name").as_deref(), Some("B"));

        // And the filler is not permanent furniture: once the keys are unique
        // again it is unmounted, not left sitting in the window.
        items.set(vec![
            (1u32, "A1".to_string()),
            (3u32, "C".to_string()),
            (2u32, "B".to_string()),
        ]);
        let children = window.children();
        assert_eq!(
            children.len(),
            3,
            "three unique keys, three rows, no filler left behind"
        );
        assert_eq!(row_names(&window), ["A1", "C", "B"]);
    }

    /// A duplicate-free list allocates no filler at all — the common case pays
    /// nothing for the repair above (issue #185).
    #[test]
    fn a_list_with_unique_keys_mounts_no_filler() {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let items = Signal::new(vec![(1u32, "A".to_string()), (2u32, "B".to_string())]);
        let list = super::virtual_list(
            &mut scope,
            20.0,
            move || items.get(),
            |item: &(u32, String)| item.0,
            1,
            |item: (u32, String), s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.1);
                node
            },
        );

        let window = list.children().remove(1);
        assert_eq!(window.children().len(), 2);
        assert_eq!(row_names(&window), ["A", "B"]);
    }
}
