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
//!    - Removed items have their scopes disposed and *then* their nodes
//!      released, so a row's cleanups still see the row (issue #356)
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
//! with both. What a repeat *means*, though, depends on who chose the key — see
//! [`KeySource`] (issue #185):
//!
//! - **[`KeySource::Explicit`]** (a `key:` prop, or a `key_fn` handed to
//!   [`for_each_dom_typed`]): a repeat is a user error. The repeat is **not
//!   rendered** and a warning is logged — first occurrence wins, as in React.
//! - **[`KeySource::Fallback`]** (no `key:`, so rsx fabricates
//!   `format!("{:?}", item)`): a repeated *value* is not an error —
//!   `for tag in ["rust", "rust", "gui"]` is an ordinary list. The fabricated
//!   key is made unique by its occurrence ordinal instead, so every row renders.
//!
//! Either way `for_each_dom` never hands the reconcile a duplicate key.
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

use crate::dom::{NodeHandle, NodeId, RenderScope};
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

/// Where a list's keys came from, which decides what a repeated key means.
///
/// The reconcile needs unique keys either way — one key names one [`ItemState`]
/// and one mounted sibling, with `keys_order` in step with both — but the two
/// provenances earn very different repairs (issue #185).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeySource {
    /// The caller chose the key: a `key:` prop in rsx, or the `key_fn` handed to
    /// [`for_each_dom_typed`]. A repeat is a **user error**, so the repeat is
    /// dropped and the first occurrence wins — the rule React applies to
    /// duplicate `key` props ("only the first child will be used").
    #[default]
    Explicit,
    /// The framework fabricated the key because the list has no `key:` — rsx
    /// falls back to `format!("{:?}", item)`
    /// (`rinch-macros/src/dom_codegen/control_flow.rs`). A repeated *value* is
    /// not a user error: `for tag in ["rust", "rust", "gui"]` is an ordinary
    /// list. Dropping a row there would be a worse bug than the one the dedup
    /// exists to fix, so the fabricated key is made unique by its occurrence
    /// ordinal instead and every row renders.
    Fallback,
}

/// Separates a fabricated key from its occurrence ordinal.
///
/// U+0001 (START OF HEADING). Every derived `Debug` routes strings and chars
/// through `escape_debug`, which renders a literal U+0001 as the four-character
/// text `\u{1}`, so a fabricated key can only contain a raw one if a hand-written
/// `Debug` impl writes it.
///
/// **If it does collide** — a `Debug` impl that emits U+0001 such that a
/// synthesized key equals some other item's key — nothing breaks: the
/// disambiguated list is re-checked and any remaining duplicate falls back to
/// the explicit-key rule (first occurrence wins, repeat dropped, warning
/// logged). The invariant the reconcile rests on holds in every case; only the
/// "never drop an unkeyed row" nicety is lost, for a list whose `Debug` output
/// contains a control character.
const FALLBACK_KEY_ORDINAL_SEP: char = '\u{1}';

/// True if any key in `items` repeats one earlier in the list.
///
/// This runs on every reconcile pass and the overwhelmingly common list is
/// duplicate-free, so it borrows the keys rather than cloning each one into an
/// owned set. Only a list that actually repeats a key pays for anything more.
fn has_duplicate_key(items: &[ForItem]) -> bool {
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(items.len());
    items.iter().any(|item| !seen.insert(item.key.as_str()))
}

/// Make a fabricated key unique by appending its occurrence ordinal.
///
/// The first `rust` keys as `rust`, the second as `rust\u{1}2`, the third as
/// `rust\u{1}3`. Two properties matter and both come from keying off the
/// ordinal rather than the item's index:
///
/// - **Stable pass to pass.** The ordinal follows multiset order, so a list that
///   is re-evaluated unchanged produces byte-identical keys and the reconcile
///   emits no operations at all.
/// - **Identity still follows the value.** `[a, a, b] -> [b, a, a]` keys as
///   `[a, a2, b] -> [b, a, a2]`: the same key set, so `diff_keyed` emits moves
///   and every row keeps its DOM node and its state. An index-primary key would
///   make identity follow position and re-render every row on any reorder —
///   which is exactly the per-row state loss `key:` exists to prevent.
fn disambiguate_fallback_keys(mut items: Vec<ForItem>) -> Vec<ForItem> {
    let mut counts: HashMap<String, usize> = HashMap::with_capacity(items.len());
    for item in &mut items {
        let n = counts.entry(item.key.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            item.key = format!("{}{}{}", item.key, FALLBACK_KEY_ORDINAL_SEP, n);
        }
    }
    items
}

/// Hand the reconcile a list whose keys are unique, whatever the caller passed.
///
/// One key, one item (issue #185). Rendering a repeat used to displace the first
/// occurrence's [`ItemState`] out of `items_state`; dropping the displaced state
/// disposed its [`RenderScope`] — handlers deregistered, signals freed — while
/// its DOM node stayed a sibling of the marker, because [`NodeHandle`] has no
/// `Drop` and nothing else held the node. The result was a row that renders,
/// swallows clicks and never updates again, unreachable from every data
/// structure and so beyond the reach of any later `ListOp::Remove`.
///
/// The repair depends on [`KeySource`]: an explicit repeat is dropped (first
/// wins), a fabricated one is uniquified by ordinal. A fabricated key that is
/// *still* duplicated after uniquification — only reachable through a `Debug`
/// impl that emits [`FALLBACK_KEY_ORDINAL_SEP`] — falls through to the drop.
///
/// `warned` records which keys have already been reported, so a list with two
/// distinct collisions reports both. It is cleared by any duplicate-free pass,
/// which both bounds it to one pass's worth of keys and makes a collision that
/// comes back after going away newsworthy again. (A plain one-shot latch would
/// spend the entire budget on the first duplicate a list ever sees — a
/// placeholder row before data loads — and then silently drop rows forever.)
fn prepare_keys(
    mut items: Vec<ForItem>,
    key_source: KeySource,
    warned: &RefCell<std::collections::HashSet<String>>,
) -> Vec<ForItem> {
    if !has_duplicate_key(&items) {
        warned.borrow_mut().clear();
        return items;
    }

    if key_source == KeySource::Fallback {
        items = disambiguate_fallback_keys(items);
        if !has_duplicate_key(&items) {
            warned.borrow_mut().clear();
            return items;
        }
    }

    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(items.len());
    // Rejected items are dropped by `retain`, and dropping a `ForItem` drops the
    // user's data — user code. Run it untracked: a `Drop` impl that reads a
    // signal would otherwise subscribe the whole reconcile effect to that
    // signal, so an unrelated write to it would re-run the entire list diff. The
    // `Insert` arm wraps `view` in `untracked` for the same reason.
    crate::reactive::untracked(|| {
        items.retain(|item| {
            if seen.insert(item.key.clone()) {
                return true;
            }
            if warned.borrow_mut().insert(item.key.clone()) {
                tracing::warn!(
                    "duplicate `for` key {:?}: this item is not rendered. \
                     Give each item a unique `key:`.",
                    item.key
                );
            }
            false
        });
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

/// A row taken out of a list during a pass, parked until the pass's borrows
/// are released and its scope has been disposed (issue #356).
///
/// **Dispose first, then release** — the order `show_dom`, `match_dom` and the
/// component re-render already use, and `RootHandle::unmount` before them: a
/// row's cleanups legitimately touch their own node, and a node that is already
/// `discard`ed is retired on `rinch-web` (every read answers `None`, every
/// write is a silent no-op) while it is still live on desktop. The scope cannot
/// be disposed where the row is taken out, because disposal runs user code
/// under the list's `RefMut`s (#141), so the *node* is parked with it.
///
/// `owned` is the verb (issue #719), decided **when the row is parked** —
/// before its scope is disposed, as every other helper decides it: a row the
/// `view` closure built through this scope is `discard`ed, one it was handed is
/// only detached. `None` for the scope (a row whose scope was already parked)
/// reads as "not ours", the safe direction: a missed `discard` costs memory on
/// `rinch-web`, a wrong one costs a subtree someone can still show.
pub(crate) struct ParkedRow {
    node: NodeHandle,
    owned: bool,
    scope: Option<RenderScope>,
}

impl ParkedRow {
    pub(crate) fn new(node: NodeHandle, scope: Option<RenderScope>) -> Self {
        let owned = scope.as_ref().is_some_and(|s| s.created(node.node_id()));
        Self { node, owned, scope }
    }
}

/// Tear parked rows down: every scope disposed, *then* every node released.
///
/// Call with no borrow of the list's own state held — disposal runs user code
/// that may re-enter the list (#141). `shown` is asked once, after disposal,
/// for the nodes the list's live rows hold: a memoising `view` can hand the very
/// node a departing key showed to a key that arrived in the same pass, and the
/// insert has already put it in place by the time the parked release runs.
/// Releasing it anyway would take out the row the list is now showing; the
/// inline release this replaced ran *before* that insert and so never met it.
/// Asked once rather than per row, so tearing down `m` rows of an `n`-row list
/// costs `O(n + m)`, not `O(n * m)`.
///
/// **A panicking cleanup still releases every node.** The release is done by a
/// drop guard, so it runs on unwind too. Unwinding drops the rows the loop had
/// not reached yet first, and a `RenderScope` disposes itself on drop, so every
/// other row's cleanups still run before any node goes — the same order as the
/// normal path. Without the guard a panic caught above the reconcile left every
/// departing row mounted and in no list's bookkeeping, where the inline release
/// this replaced had already taken them out.
pub(crate) fn release_parked(
    parked: Vec<ParkedRow>,
    shown: impl FnOnce() -> std::collections::HashSet<NodeId>,
) {
    if parked.is_empty() {
        return;
    }
    let _release = ReleaseNodes {
        nodes: parked
            .iter()
            .map(|row| (row.node.clone(), row.owned))
            .collect(),
        shown: Some(shown),
    };
    for row in parked {
        if let Some(scope) = row.scope {
            scope.dispose();
        }
    }
}

/// The node half of [`release_parked`], run when it is dropped — at the end of
/// the call, or on unwind out of a row's cleanup.
struct ReleaseNodes<F: FnOnce() -> std::collections::HashSet<NodeId>> {
    nodes: Vec<(NodeHandle, bool)>,
    shown: Option<F>,
}

impl<F: FnOnce() -> std::collections::HashSet<NodeId>> Drop for ReleaseNodes<F> {
    fn drop(&mut self) {
        // On unwind, run no queued effect from inside the node verbs: a second
        // panic there would abort the process.
        let _quiet = std::thread::panicking().then(crate::reactive::suppress_effect_flush);
        let shown = self.shown.take().map(|f| f()).unwrap_or_default();
        // Either verb cancels the subtree's transitions and animations in the
        // document implementation (#699); stamping inline `transition: none`
        // here disarmed that permanently (#704).
        for (node, owned) in self.nodes.drain(..) {
            if shown.contains(&node.node_id()) {
                continue;
            }
            if owned {
                node.discard();
            } else {
                node.remove();
            }
        }
    }
}

/// The nodes of the rows `state` still holds.
fn shown_in(state: &RefCell<HashMap<String, ItemState>>) -> std::collections::HashSet<NodeId> {
    state
        .borrow()
        .values()
        .map(|row| row.node.node_id())
        .collect()
}

/// Tear down an [`ItemState`] that a duplicate key displaced out of `items_state`.
///
/// Unreachable in practice — [`prepare_keys`] guarantees one `ItemState` per key,
/// and both call sites `debug_assert!` it — but a `debug_assert!` is a hard panic
/// in dev and *nothing at all* in release, so the release path must still leave
/// the DOM in a state someone can reason about.
///
/// Issue #185 was exactly this situation handled badly: the displaced state was
/// dropped inline, which disposed its [`RenderScope`] while its node stayed
/// mounted — a row that renders, swallows clicks and never updates again,
/// unreachable from every data structure. So park the row — node and scope —
/// for the caller to tear down once its `RefMut`s are released (disposal runs
/// user code, issue #141; the node goes after the scope, issue #356), and say
/// so out loud: reaching this is a rinch bug, not an app bug, and the `warn!` is
/// the only trace of it a release build will ever produce.
fn reclaim_displaced(displaced: ItemState) -> ParkedRow {
    tracing::warn!(
        "for_each_dom: duplicate key {:?} reached the render path — unmounting \
         and disposing the row it displaced. This is a rinch bug (issue #185); \
         please report it.",
        displaced.item.key
    );
    // Ownership decides the verb (issue #719): a row the `view` closure built
    // through this scope can never be shown again, so the backend lets go of
    // it; a row a *memoising* `view` handed back is only detached, so the same
    // key rendering again puts it back.
    ParkedRow::new(displaced.node, displaced.scope)
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
/// Keys must be unique. A caller who builds `ForItem`s by hand chose their
/// keys, so this entry point treats a repeat as a user error and drops it —
/// [`KeySource::Explicit`], first occurrence wins, warning logged (issue #185).
/// [`for_each_dom_with_key_source`] is the same function with that policy
/// spelled out; the rsx macro uses it to say a `key:`-less list's fabricated
/// key must be uniquified rather than dropped.
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
    for_each_dom_with_key_source(scope, parent, each, view, eq_fn, KeySource::Explicit)
}

/// [`for_each_dom`] with the duplicate-key policy stated explicitly.
///
/// See [`KeySource`]. Everything else is identical; `for_each_dom` is this
/// function with [`KeySource::Explicit`], which is the right default for a
/// caller who passed keys of their own choosing.
#[allow(clippy::type_complexity)]
pub fn for_each_dom_with_key_source<E, V>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    each: E,
    view: V,
    eq_fn: Option<Rc<dyn Fn(&ForItem, &ForItem) -> bool>>,
    key_source: KeySource,
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
    // item list, so every one of them would otherwise reintroduce a duplicate
    // that the insert sites had just rejected. It is also the only place a
    // *rewritten* key (the `Fallback` ordinal) can be applied once and be seen
    // by all of them.
    //
    // This must stay at the `each()` call and never move below the
    // `items_state`/`keys_order` borrows: dropping a rejected `ForItem` drops
    // its `Rc<dyn Any>` payload, which runs the user type's `Drop` — user code,
    // under a `RefMut` this closure re-enters (issue #141).
    let each = {
        let warned = RefCell::new(std::collections::HashSet::new());
        move || prepare_keys(each(), key_source, &warned)
    };

    // Initial render - insert items as siblings after the marker
    {
        let initial_items = each();
        let mut state = items_state.borrow_mut();
        let mut keys = keys_order.borrow_mut();

        // Track inserted nodes to chain insert_after calls
        let mut initial_nodes: Vec<NodeHandle> = Vec::new();
        // Rows displaced by a duplicate key, torn down after `state` is
        // released — see the note on `state.insert` below.
        let mut displaced: Vec<ParkedRow> = Vec::new();

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
                // `each` yields unique keys (issue #185), so nothing can be
                // displaced here — the `debug_assert!` says so loudly in dev.
                // The release path is not a no-op: it disposes and unmounts the
                // displaced row, because leaving it half-torn-down is precisely
                // the #185 bug. The row is parked rather than torn down inline,
                // since disposal runs user code under `state`'s `RefMut` (#141).
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
                    "for_each_dom: `prepare_keys` guarantees one `ItemState` per key"
                );
                if let Some(clobbered) = clobbered {
                    displaced.push(reclaim_displaced(clobbered));
                }
            }
        }

        drop(state);
        drop(keys);
        release_parked(displaced, || shown_in(&items_state));
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

        // Rows displaced by this reconcile — node and scope together — torn
        // down at the very end of the closure once `state` and `keys` are no
        // longer borrowed: each scope disposed, *then* its node released, so a
        // row's cleanups still see a live row (issue #356).
        //
        // Disposal runs user code — cleanups, handler-closure drops, signal
        // value drops (issue #141) — and any of it that writes a signal flushes
        // effects synchronously, re-entering this very closure. Disposing under
        // the `RefMut`s below would make that a `BorrowMutError` rather than a
        // reconcile. The cost is that a removed item's effects stay live for the
        // remainder of this pass; they have no signal to wake them in that
        // window, since nothing re-enters the flush before the drop. The other
        // cost is that a removed row's node stays in the DOM for the rest of
        // the pass. Nothing here reads DOM order: every insert and move is
        // anchored on a live row's node or the marker, so a parked node between
        // two of them changes none of their placements.
        let mut doomed: Vec<ParkedRow> = Vec::new();

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
                        // Ownership decides the verb (issue #719). A row the
                        // `view` closure *built* is gone for good — the key
                        // coming back is rendered afresh by the `Insert` arm
                        // below — so the backend lets go of it. A row a
                        // *memoising* `view` handed back is only detached, so
                        // that later `Insert` can put the same subtree in
                        // place. Decided now, applied after the scope is
                        // disposed (issue #356).
                        doomed.push(ParkedRow::new(item_state.node, item_state.scope));
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

                        // Update state. `each` yields unique keys (issue #185),
                        // so nothing can be displaced here; the release path
                        // still unmounts and disposes anything that is, rather
                        // than leaving the #185 half-torn-down row behind. The
                        // scope is parked, not disposed inline: disposal runs
                        // user code under the `state` borrow (#141).
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
                            "for_each_dom: `prepare_keys` guarantees one `ItemState` per key"
                        );
                        if let Some(clobbered) = clobbered {
                            doomed.push(reclaim_displaced(clobbered));
                        }
                    }
                }
                ListOp::Move { key, new_index, .. } => {
                    // Move the existing node to new position
                    if let Some(item_state) = state.get(&key) {
                        let node = item_state.node.clone();

                        // Unlink the key from its old slot *first*, so the
                        // anchor below is read from the list the node is about
                        // to be spliced into rather than from the list it is
                        // still occupying. Reading `keys[new_index - 1]` while
                        // the moved key still sits at its old index shifts every
                        // later entry by one: rotating `[a, b, c]` to
                        // `[b, c, a]` picked `b` as the anchor and produced
                        // `[b, a, c]` in the DOM while `keys_order` recorded
                        // `[b, c, a]` — the exact desync this module's
                        // invariants rest on.
                        if let Some(old_pos) = keys.iter().position(|k| k == &key) {
                            keys.remove(old_pos);
                        }
                        let insert_pos = new_index.min(keys.len());

                        // Find insertion point
                        let prev_key = if insert_pos == 0 {
                            None
                        } else {
                            Some(keys[insert_pos - 1].clone())
                        };

                        match prev_key.and_then(|k| state.get(&k)) {
                            Some(prev_state) => prev_state.node.insert_after(&node),
                            None => marker_effect.insert_after(&node),
                        }

                        // Update keys order
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
                            let previous_scope = old_state.scope.take();
                            let mut child_scope = RenderScope::new(doc, parent_id);
                            // The re-rendered item owns its new resources
                            // (issue #141); the old scope was disposed above,
                            // under the reconcile effect's owner.
                            let new_node = {
                                let _owner = child_scope.push_owner();
                                crate::reactive::untracked(|| view_clone(item, &mut child_scope))
                            };
                            old_state.node.insert_after(&new_node);
                            // Ownership decides the verb (issue #719): the node
                            // being replaced is released only if the render that
                            // produced it built it. A memoising `view` that
                            // returns one of a small set of cached subtrees may
                            // hand this very node back later. Released after
                            // its old scope is disposed (issue #356).
                            doomed.push(ParkedRow::new(
                                std::mem::replace(&mut old_state.node, new_node),
                                previous_scope,
                            ));
                            old_state.item = item.clone();
                            old_state.scope = Some(child_scope);
                        }
                    }
                }
            }
        }

        // Update keys to match new order
        *keys = new_keys;

        // Borrows released before the parked rows are torn down.
        drop(state);
        drop(keys);
        release_parked(doomed, || shown_in(&items_state_clone));

        // The batch is dropped last, and untracked. Dropping a `ForItem` drops
        // the user's data, and a `Drop` impl that reads a signal would otherwise
        // subscribe this reconcile effect to it — an unrelated write would then
        // re-run the whole list diff. `prepare_keys` untracks its own rejected
        // items for the same reason; leaving this one tracked would defeat that,
        // since every item passes through here on every pass.
        drop(new_items_map);
        crate::reactive::untracked(move || drop(new_items));
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
/// # Duplicate keys
///
/// `key_fn` is the caller's choice of key, so a repeat is treated as a user
/// error: [`KeySource::Explicit`], first occurrence wins (issue #185). The rsx
/// macro calls [`for_each_dom_typed_with_key_source`] instead, passing
/// [`KeySource::Fallback`] for a list with no `key:` prop.
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
    for_each_dom_typed_with_key_source(scope, parent, collection, key_fn, view, KeySource::Explicit)
}

/// [`for_each_dom_typed`] with the duplicate-key policy stated explicitly.
///
/// See [`KeySource`]. This is what the rsx `for` loop expands to: a list with a
/// `key:` prop passes [`KeySource::Explicit`], and a list without one passes
/// [`KeySource::Fallback`] so its fabricated `format!("{:?}", item)` key is
/// uniquified by occurrence ordinal instead of dropping the row.
pub fn for_each_dom_typed_with_key_source<T, C, K, V>(
    scope: &mut RenderScope,
    parent: &NodeHandle,
    collection: C,
    key_fn: K,
    view: V,
    key_source: KeySource,
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

    for_each_dom_with_key_source(
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
        key_source,
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
        let reconciles = Rc::new(std::cell::Cell::new(0));
        let counted = reconciles.clone();

        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || {
                counted.set(counted.get() + 1);
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

        let mounted = reconciles.get();

        // Drops item "a": its cleanup writes `churn`, whose flush re-enters this
        // reconcile.
        items.update(|v| v.retain(|i| i.id == "b"));

        assert_eq!(churn.get(), 1, "exactly the removed item's cleanup ran");
        // The cleanup's write is a wake the reconcile gives itself while it is
        // running, and that wake is dropped (issue #343): the list reconciled
        // once for the removal and not again for `churn`. Were it re-run
        // instead, a row whose cleanup writes list state would make every
        // reconcile schedule another.
        assert_eq!(
            reconciles.get() - mounted,
            1,
            "one pass for the removal; the cleanup's own wake was dropped"
        );
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

    /// The `key:`-less case: two `Debug`-equal items are **not** a user error
    /// (issue #185).
    ///
    /// With no `key:` prop the rsx macro keys items by `format!("{:?}", item)`
    /// (`rinch-macros/src/dom_codegen/control_flow.rs`) and passes
    /// [`KeySource::Fallback`], which this mirrors directly — the macro expands
    /// to `rinch::core::…` paths and cannot be invoked from inside `rinch-core`.
    ///
    /// `for tag in ["rust", "rust", "gui"]` is an ordinary list, so all three
    /// rows render. This test replaces an earlier one asserting the opposite:
    /// deduplicating a *fabricated* key deleted rows from lists that rendered
    /// correctly before issue #185's fix, which is a worse regression than the
    /// dead-scope bug it was fixing.
    #[test]
    fn an_unkeyed_for_renders_every_row_even_when_two_items_are_equal() {
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
        let marker = super::for_each_dom_typed_with_key_source(
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
            super::KeySource::Fallback,
        );
        let _ = marker;

        assert_eq!(
            mounted_row_names(&parent),
            ["A", "A", "B"],
            "a repeated value is not a duplicate key error — every row renders"
        );

        let seen = seen.borrow();
        assert_eq!(seen.len(), 3, "every row is rendered exactly once");
        for (i, owner) in seen.iter().enumerate() {
            let owner = owner
                .clone()
                .unwrap_or_else(|| panic!("row {i} had no owner"));
            assert!(owner.is_alive(), "row {i}'s scope must still be live");
        }
    }

    /// An explicit `key:` keeps the React rule even when the *values* repeat
    /// (issue #185).
    ///
    /// The companion to the test above: same three items, same `Debug`-equal
    /// pair, but the caller supplies the key. Choosing a key that collides is a
    /// user error, so the repeat is dropped and warned about. This is the pair
    /// of assertions the [`KeySource`] split exists to keep apart.
    #[test]
    fn an_explicitly_keyed_for_still_drops_a_repeat_of_the_same_values() {
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

        assert_eq!(mounted_row_names(&parent), ["A", "B"]);
    }

    /// The fabricated ordinal key is **stable pass to pass** (issue #185).
    ///
    /// This is what makes the ordinal safe: re-evaluating the same list must
    /// produce byte-identical keys, or `diff_keyed` sees a wholesale key change
    /// and the reconcile churns every row on every update. Asserted the way the
    /// user would notice it — no row re-rendered, no node replaced.
    #[test]
    fn unkeyed_duplicate_rows_keep_their_identity_across_an_unrelated_update() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::Cell;
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
        let renders = Rc::new(Cell::new(0usize));

        let count = renders.clone();
        let marker = super::for_each_dom_typed_with_key_source(
            &mut scope,
            &parent,
            move || items.get(),
            |item: &TestItem| format!("{item:?}"),
            move |item: TestItem, s: &mut RenderScope| {
                count.set(count.get() + 1);
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
            super::KeySource::Fallback,
        );
        let _ = marker;

        assert_eq!(renders.get(), 3);
        let ids_before = mounted_row_ids(&parent);

        // Re-run the reconcile with a list that is `PartialEq`-identical.
        items.update(|v| {
            let restated = v.clone();
            *v = restated;
        });

        assert_eq!(
            renders.get(),
            3,
            "an unchanged list must not re-render a row: the ordinal keys \
             matched, so `diff_keyed` had nothing to do"
        );
        assert_eq!(
            mounted_row_ids(&parent),
            ids_before,
            "and every row kept the DOM node it already had"
        );
        assert_eq!(mounted_row_names(&parent), ["A", "A", "B"]);
    }

    /// Reordering an unkeyed list with repeated values **moves** rows rather
    /// than rebuilding them (issue #185).
    ///
    /// This is the property an index-primary fallback key would destroy. Keying
    /// the repeat as `<debug>\u{1}2` keeps identity attached to the *value*, so
    /// `[A, A, B] -> [B, A, A]` is the same key set in a new order: `diff_keyed`
    /// emits moves, every node survives, nothing re-renders. Keying by index
    /// would instead make every row's data differ from the row now at its
    /// position, re-rendering the whole list and losing per-row state — the
    /// exact failure `key:` exists to prevent.
    #[test]
    fn unkeyed_duplicates_survive_a_reorder_as_moves_not_re_renders() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::Cell;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let a = TestItem {
            id: "a".into(),
            name: "A".into(),
        };
        let b = TestItem {
            id: "b".into(),
            name: "B".into(),
        };
        let items = Signal::new(vec![a.clone(), a.clone(), b.clone()]);
        let renders = Rc::new(Cell::new(0usize));

        let count = renders.clone();
        let marker = super::for_each_dom_typed_with_key_source(
            &mut scope,
            &parent,
            move || items.get(),
            |item: &TestItem| format!("{item:?}"),
            move |item: TestItem, s: &mut RenderScope| {
                count.set(count.get() + 1);
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.name);
                node
            },
            super::KeySource::Fallback,
        );
        let _ = marker;

        assert_eq!(renders.get(), 3);
        let mut ids_before = mounted_row_ids(&parent);
        assert_eq!(ids_before.len(), 3);

        items.set(vec![b, a.clone(), a]);

        assert_eq!(
            mounted_row_names(&parent),
            ["B", "A", "A"],
            "the rotation lands in the DOM"
        );
        assert_eq!(
            renders.get(),
            3,
            "a reorder is a move, not a rebuild — no row re-rendered"
        );

        let mut ids_after = mounted_row_ids(&parent);
        assert_eq!(ids_after.len(), 3, "still three rows, none dropped");
        ids_before.sort_unstable();
        ids_after.sort_unstable();
        assert_eq!(
            ids_after, ids_before,
            "the same three DOM nodes were moved, not rebuilt"
        );
    }

    /// A `Debug` impl that emits the ordinal separator falls back to first-wins
    /// (issue #185).
    ///
    /// [`FALLBACK_KEY_ORDINAL_SEP`] is U+0001, which no derived `Debug` can
    /// produce — `escape_debug` renders it as the text `\u{1}`. A hand-written
    /// impl can, though, and then a synthesized key can equal another item's
    /// key. The documented consequence is not corruption: uniqueness is
    /// re-checked and the leftover duplicate is dropped by the explicit-key
    /// rule, so the reconcile's one-key-one-row invariant still holds.
    #[test]
    fn a_debug_impl_that_emits_the_separator_falls_back_to_first_wins() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use std::cell::RefCell;

        #[derive(Clone, PartialEq)]
        struct Sneaky(String);
        impl std::fmt::Debug for Sneaky {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        // `x`, `x`, and a third item whose own Debug output is already exactly
        // what the second one will be renamed to.
        let items = vec![
            Sneaky("x".into()),
            Sneaky("x".into()),
            Sneaky(format!("x{}2", super::FALLBACK_KEY_ORDINAL_SEP)),
        ];

        let marker = super::for_each_dom_typed_with_key_source(
            &mut scope,
            &parent,
            move || items.clone(),
            |item: &Sneaky| format!("{item:?}"),
            |item: Sneaky, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.0);
                node
            },
            super::KeySource::Fallback,
        );
        let _ = marker;

        // Two rows, not three: the collision is unrepairable by ordinal, so the
        // explicit-key rule takes over. What matters is that the invariant holds
        // — one key, one mounted row — not which row lost.
        let names = mounted_row_names(&parent);
        assert_eq!(names.len(), 2, "the unrepairable collision drops one row");
        assert_eq!(names[0], "x");
    }

    /// Dropping the items a duplicate key rejected must not subscribe the
    /// reconcile effect to whatever their `Drop` reads (issue #185 review).
    ///
    /// `prepare_keys` runs inside the effect's tracked region, and so does the
    /// drop of the whole batch at the end of the pass. A user type whose `Drop`
    /// reads a signal — a handle releasing a shared counter, a guard reading
    /// config — would otherwise put that signal in the reconcile's dependency
    /// set, so an unrelated write to it would re-run the entire list diff. The
    /// `Insert` arm already wraps `view` in `untracked` for the same reason.
    #[test]
    fn a_rejected_items_drop_does_not_subscribe_the_reconcile() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::Cell;
        use std::cell::RefCell;

        /// Reads `probe` on drop. `Clone`, and `PartialEq` on the key alone, so
        /// it can ride `for_each_dom_typed`'s data-comparison pass.
        #[derive(Clone)]
        struct DropReadsSignal {
            id: String,
            probe: Signal<u32>,
        }
        impl PartialEq for DropReadsSignal {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        impl Drop for DropReadsSignal {
            fn drop(&mut self) {
                // The whole point: user code, reading a signal, at drop time.
                let _ = self.probe.get();
            }
        }

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let probe = Signal::new(0u32);
        let items = vec![
            DropReadsSignal {
                id: "a".into(),
                probe,
            },
            // Rejected by `prepare_keys`, and dropped there.
            DropReadsSignal {
                id: "a".into(),
                probe,
            },
            DropReadsSignal {
                id: "b".into(),
                probe,
            },
        ];

        // Counts reconcile passes: the collection closure is called once at the
        // top of every one.
        let passes = Rc::new(Cell::new(0usize));

        let count = passes.clone();
        let marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || {
                count.set(count.get() + 1);
                items.clone()
            },
            |item: &DropReadsSignal| item.id.clone(),
            |item: DropReadsSignal, s: &mut RenderScope| {
                let node = s.create_element("div");
                node.set_attribute("data-name", &item.id);
                node
            },
        );
        let _ = marker;

        assert_eq!(mounted_row_names(&parent), ["a", "b"]);
        let passes_before = passes.get();
        assert!(passes_before > 0);

        probe.set(1);

        assert_eq!(
            passes.get(),
            passes_before,
            "writing a signal that only a dropped item's `Drop` read must not \
             re-run the reconcile"
        );
    }

    /// The unreachable-in-release insurance path actually works (issue #185).
    ///
    /// `prepare_keys` guarantees one `ItemState` per key and both `state.insert`
    /// sites `debug_assert!` it — but a `debug_assert!` is a hard panic in dev
    /// and *nothing* in release, so the release path has to leave the DOM in a
    /// state someone can reason about. Issue #185 was this exact situation
    /// handled badly: scope disposed, node left mounted. So `reclaim_displaced`
    /// parks the whole row for the caller to tear down once its borrows are
    /// released — scope first, then node (issue #356).
    #[test]
    fn a_displaced_item_state_is_parked_then_disposed_then_unmounted() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let mut child_scope = RenderScope::new(doc.clone(), parent.node_id());
        let node = {
            let _owner = child_scope.push_owner();
            let n = child_scope.create_element("div");
            Signal::new(0);
            n
        };
        parent.append_child(&node);
        let owner = child_scope.owner();
        assert_eq!(parent.children().len(), 1, "the row starts out mounted");

        let displaced = super::ItemState {
            node,
            item: ForItem::new("a", "A".to_string()),
            scope: Some(child_scope),
        };

        let parked = super::reclaim_displaced(displaced);

        assert!(
            owner.is_alive(),
            "its scope is still live until the caller tears the row down, so \
             disposal never runs user code under a `RefMut`"
        );
        assert_eq!(
            parent.children().len(),
            1,
            "and so is its node: it leaves after its scope, not before (#356)"
        );

        super::release_parked(vec![parked], Default::default);
        assert!(!owner.is_alive(), "tearing the row down frees its scope");
        assert_eq!(
            parent.children().len(),
            0,
            "and unmounts the row — leaving it is issue #185 itself"
        );
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

    /// A rotation lands the moved row where `keys_order` says it went.
    ///
    /// `[a, b, c] -> [b, c, a]` is the shortest list that produces a single
    /// `ListOp::Move` whose target index sits *after* the moved key's own old
    /// slot. Reading the anchor as `keys[new_index - 1]` while `a` still
    /// occupied index 0 picked `b` — one entry short — so the DOM ended up
    /// `[b, a, c]` while `keys_order` recorded `[b, c, a]`, and nothing ever
    /// repaired the split: the next diff sees the key order it wanted.
    #[test]
    fn a_rotation_moves_the_row_to_the_slot_keys_order_records() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let mk = |id: &str| TestItem {
            id: id.into(),
            name: id.to_uppercase(),
        };
        let items = Signal::new(vec![mk("a"), mk("b"), mk("c")]);

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
        assert_eq!(mounted_row_names(&parent), ["A", "B", "C"]);

        items.set(vec![mk("b"), mk("c"), mk("a")]);
        assert_eq!(
            mounted_row_names(&parent),
            ["B", "C", "A"],
            "the moved row lands after its new predecessor"
        );

        // And back the other way, which moves two rows in one pass.
        items.set(vec![mk("c"), mk("a"), mk("b")]);
        assert_eq!(mounted_row_names(&parent), ["C", "A", "B"]);
    }

    /// A moved node is *relocated*, never duplicated: the parent lists it once.
    ///
    /// Pins `MockDomDocument`'s re-parenting against `RinchDocument` and the web
    /// backend, both of which detach before inserting. Without that, every
    /// sibling-order assertion in this module reads a reordered row as a second
    /// mount.
    #[test]
    fn moving_a_row_does_not_leave_it_listed_twice() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let mk = |id: &str| TestItem {
            id: id.into(),
            name: id.to_uppercase(),
        };
        let items = Signal::new(vec![mk("a"), mk("b"), mk("c")]);

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

        items.set(vec![mk("b"), mk("c"), mk("a")]);

        let mut ids = mounted_row_ids(&parent);
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(total, 3, "three rows, not a duplicated sibling");
        assert_eq!(ids.len(), total, "every mounted node appears exactly once");
    }

    /// A removed item's cleanup that reads a signal must not subscribe the
    /// reconcile effect (issue #494).
    ///
    /// The reconcile tail already drops the item *collection* untracked (issue
    /// #345); this pins the scope disposal a few lines above it — the doomed
    /// scopes' cleanups run inside the same effect, and a tracked read there
    /// would re-run the whole list diff on every later write to that signal.
    /// Counts reconcile passes via the collection closure, since the DOM is
    /// identical either way.
    #[test]
    fn a_removed_items_cleanup_that_reads_a_signal_does_not_subscribe_the_reconcile_effect() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::{Signal, on_cleanup};
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec![1u32, 2]);
        let probe = Signal::new(0u32);
        let passes = Rc::new(Cell::new(0usize));

        let count = passes.clone();
        let _marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || {
                count.set(count.get() + 1);
                items.get()
            },
            |item: &u32| item.to_string(),
            move |_item: u32, s: &mut RenderScope| {
                // Registered against the item's own scope, read by the
                // disposal fixpoint when the item leaves the list.
                on_cleanup(move || {
                    let _ = probe.get();
                });
                s.create_element("div")
            },
        );

        items.update(|v| v.retain(|&i| i != 2)); // dispose item 2's scope
        let passes_before = passes.get();

        probe.set(1);
        assert_eq!(
            passes.get(),
            passes_before,
            "a write to a signal only a removed item's cleanup read must not re-run the reconcile"
        );

        // Positive control: a write the reconcile effect legitimately tracks.
        items.update(|v| v.push(3));
        assert_eq!(passes.get(), passes_before + 1);
    }

    /// What a row's cleanup saw of its own node: its `data-name`, and whether it
    /// was still a child of the list's parent.
    type CleanupSight = Rc<RefCell<Vec<(Option<String>, bool)>>>;

    /// A `view` that stamps `data-name` on its row and registers a cleanup
    /// recording what that cleanup can still see of the row (issue #356).
    fn row_that_looks_at_itself_on_cleanup(
        sight: CleanupSight,
        parent: NodeHandle,
    ) -> impl Fn(String, &mut crate::dom::RenderScope) -> NodeHandle + 'static {
        move |name: String, s: &mut crate::dom::RenderScope| {
            let row = s.create_element("div");
            row.set_attribute("data-name", &name);
            let (me, sight, parent) = (row.clone(), sight.clone(), parent.clone());
            crate::reactive::on_cleanup(move || {
                let attached = me
                    .parent_node()
                    .is_some_and(|p| p.node_id() == parent.node_id());
                sight
                    .borrow_mut()
                    .push((me.get_attribute("data-name"), attached));
            });
            row
        }
    }

    /// A removed row's cleanup runs against its node while that node is still
    /// the live, mounted row (issue #356).
    ///
    /// The `Remove` arm used to `discard` the row during the pass and dispose
    /// its scope at the end of it, so the cleanup ran against a retired node:
    /// on `rinch-web` (and the mock, which retires the same way) every read
    /// answered `None` and every write was a silent no-op, while `show_dom`,
    /// `match_dom` and the component re-render all dispose first. Sampled off
    /// the fixed point: the removed row is the *middle* one, with a sibling on
    /// each side, and carries a non-empty attribute.
    #[test]
    fn a_removed_rows_cleanup_sees_its_own_live_node() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let sight: CleanupSight = Rc::new(RefCell::new(Vec::new()));
        let _marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |s: &String| s.clone(),
            row_that_looks_at_itself_on_cleanup(sight.clone(), parent.clone()),
        );

        items.set(vec!["a".to_string(), "c".to_string()]);

        assert_eq!(
            *sight.borrow(),
            vec![(Some("b".to_string()), true)],
            "#356: the cleanup must run before its row is discarded or detached"
        );
        // And the row is still released afterwards (#719: it was built by the
        // row's own scope, so it is discarded).
        let names: Vec<_> = parent
            .children()
            .iter()
            .filter_map(|n| n.get_attribute("data-name"))
            .collect();
        assert_eq!(names, vec!["a".to_string(), "c".to_string()]);
    }

    /// The `Changed` arm too: a row re-rendered because its data changed has its
    /// old scope disposed while the old node is still mounted (issue #356).
    #[test]
    fn a_rerendered_rows_old_cleanup_sees_its_own_live_node() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        // Keyed by the first character, so "b1" -> "b2" is the same key with
        // different data: a re-render, not a remove + insert.
        let items = Signal::new(vec!["a1".to_string(), "b1".to_string(), "c1".to_string()]);
        let sight: CleanupSight = Rc::new(RefCell::new(Vec::new()));
        let _marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |s: &String| s[..1].to_string(),
            row_that_looks_at_itself_on_cleanup(sight.clone(), parent.clone()),
        );

        items.set(vec!["a1".to_string(), "b2".to_string(), "c1".to_string()]);

        assert_eq!(
            *sight.borrow(),
            vec![(Some("b1".to_string()), true)],
            "#356: the replaced row's cleanup must run before its node is released"
        );
        let names: Vec<_> = parent
            .children()
            .iter()
            .filter_map(|n| n.get_attribute("data-name"))
            .collect();
        assert_eq!(
            names,
            vec!["a1".to_string(), "b2".to_string(), "c1".to_string()]
        );
    }

    /// Deferring the release to the end of the pass must not take out a node
    /// that the same pass put back (issue #356).
    ///
    /// A memoising `view` may hand the *same* cached subtree to a different key.
    /// When the key that owned it leaves and the new one arrives in one pass,
    /// the row was detached and then re-inserted — in that order, while removal
    /// happened inline. With the release parked until after disposal, the
    /// insert comes first, so a parked release that did not check would detach
    /// the node the list is now showing.
    #[test]
    fn a_parked_release_leaves_a_node_the_same_pass_reused() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        // Built outside every row scope, so each row merely *hands it back*:
        // `remove`, not `discard` (#719).
        let shared = scope.create_element("section");
        shared.set_attribute("data-name", "shared");

        let items = Signal::new(vec![1u32, 2]);
        let cached = shared.clone();
        let _marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                if n >= 2 {
                    cached.clone()
                } else {
                    let row = s.create_element("div");
                    row.set_attribute("data-name", &n.to_string());
                    row
                }
            },
        );

        // Key 2 leaves and key 3 arrives, each showing the same cached node.
        items.set(vec![1u32, 3]);

        assert_eq!(
            shared.parent_node().map(|p| p.node_id()),
            Some(parent.node_id()),
            "#356: the release parked for key 2 must not detach the node key 3 now shows"
        );
        let names: Vec<_> = parent
            .children()
            .iter()
            .filter_map(|n| n.get_attribute("data-name"))
            .collect();
        assert_eq!(names, vec!["1".to_string(), "shared".to_string()]);
    }

    /// A row cleanup that panics still lets every departing row go (review of
    /// PR #984, F1; the reviewer's probe P3).
    ///
    /// The nodes are released after the scopes are disposed (issue #356), so a
    /// panic out of a disposal skipped the release entirely: caught above the
    /// reconcile, it left `b` *and* `c` mounted and in no list's bookkeeping.
    /// The panicking row is the middle one, so there is a row torn down after
    /// it whose cleanup must still run.
    #[test]
    fn a_panicking_row_cleanup_still_releases_every_departing_row() {
        use crate::dom::traits::DomDocument;
        use crate::dom::{RenderScope, mock::MockDomDocument};
        use crate::reactive::Signal;
        use std::cell::RefCell;

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let parent = scope.parent();

        let items = Signal::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let c_cleaned = Rc::new(Cell::new(false));
        let flag = c_cleaned.clone();
        let _marker = super::for_each_dom_typed(
            &mut scope,
            &parent,
            move || items.get(),
            |s: &String| s.clone(),
            move |name: String, s: &mut RenderScope| {
                let row = s.create_element("div");
                row.set_attribute("data-name", &name);
                match name.as_str() {
                    "b" => {
                        crate::reactive::on_cleanup(|| panic!("boom"));
                    }
                    "c" => {
                        let flag = flag.clone();
                        crate::reactive::on_cleanup(move || flag.set(true));
                    }
                    _ => {}
                }
                row
            },
        );

        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            items.set(vec!["a".to_string()])
        }));
        assert!(
            r.is_err(),
            "precondition: the cleanup's panic reached the caller"
        );

        let names: Vec<_> = parent
            .children()
            .iter()
            .filter_map(|n| n.get_attribute("data-name"))
            .collect();
        assert_eq!(
            names,
            vec!["a".to_string()],
            "every departing row is released, the panicking one included"
        );
        assert!(
            c_cleaned.get(),
            "and the other departing row's cleanup still ran"
        );
    }
}
