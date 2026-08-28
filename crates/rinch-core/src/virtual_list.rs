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
/// * `key` - Function to extract a unique key from each item
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
    let doc_weak = scope.doc_weak();
    let window_id = window.node_id();
    let container_handle = container.clone();
    let spacer_handle = spacer.clone();
    let window_handle = window.clone();

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
            let start = raw_start.saturating_sub(overscan);
            let end = (raw_end + overscan).min(total);
            (start, end)
        };

        // Build key list for the new visible range
        let new_keys: Vec<String> = all_items[start..end]
            .iter()
            .map(|item| key(item).to_string())
            .collect();

        // Build a map of new items by key for quick lookup
        let new_items_by_key: HashMap<String, &T> = all_items[start..end]
            .iter()
            .map(|item| (key(item).to_string(), item))
            .collect();

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
        for k in &new_keys {
            if let Some(item_state) = state.get(k) {
                window_handle.append_child(&item_state.node);
            }
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
}
