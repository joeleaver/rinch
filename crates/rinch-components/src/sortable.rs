//! Sortable lists (#2091): drag items within and between ordered lists.
//!
//! [`SortableList`] is a list of one *group* (cards, columns, lanes…) and
//! [`SortableItem`] an item in it, named by a key. A drag — by pointer, or by
//! keyboard (Space/Enter to lift — [`SortableList::lift_keys`] —, arrows to
//! move, the lift key again to drop, Escape to cancel) — only ever targets
//! lists of its own group, so lists nest without fighting: a card dragged over
//! a column's header offers nothing, and a column dragged over its own cards is
//! placed among the columns.
//!
//! The component never reorders anything. While a drag is on offer it draws an
//! insertion caret, asks the target list's `judge`, and shows a refusal's
//! reason at the caret; at the drop it settles the place again against the
//! lists as they are then, judges it again, and on an accepted drop calls the
//! target list's `ondrop` with a key-relative [`SortPlace`]; the app moves its
//! own data.
//!
//! The pointer side rides the DOM drag suite (`draggable` + `data-ondrag*`), as
//! `Tree`'s drag does, so the threshold (a shorter press stays a click), touch
//! long-press activation on the web, Escape / `PointerCancel` cancels, the
//! missed-release heals and drag auto-scroll are the runtime's. Everything runs
//! from element handler attributes (`data-ondragstart`, `data-ondragover`,
//! `data-ondragleave`, `data-ondrop`, `data-ondragmove`, `data-ondragend`,
//! `data-onkeydown`); see `docs/design/sortable.md`.
//!
//! ```ignore
//! let judge: SortJudge = Rc::new(|m: &SortMove| {
//!     if m.to_list == "done" && m.keys.len() > 1 {
//!         SortVerdict::Refuse("One card at a time into Done".into())
//!     } else {
//!         SortVerdict::Accept
//!     }
//! });
//! rsx! {
//!     SortableList {
//!         group: "cards", id: "todo", judge: Some(judge.clone()),
//!         ondrop: move |m: SortMove| board.update(|b| b.apply(&m)),
//!         for card in cards.get() {
//!             SortableItem { key: card.id.clone(), value: card.id.clone(), {card.title.clone()} }
//!         }
//!     }
//! }
//! ```

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::DismissHandle;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ValueCallback;
use rinch_core::events::KeyEventData;
use rinch_core::get_click_context;
use rinch_core::reactive::ElementBounds;
use rinch_core::timer::set_timeout;

/// The attribute naming an item's key.
const KEY_ATTR: &str = "data-sortable-key";
/// The attribute naming a list's registration token.
const LIST_ATTR: &str = "data-sortable-list";

const ITEM_CLASS: &str = "rinch-sortable__item";
const DRAGGING_CLASS: &str = "rinch-sortable__item--dragging";
const LIFTED_CLASS: &str = "rinch-sortable__item--lifted";
const DROP_BEFORE_CLASS: &str = "rinch-sortable__item--drop-before";
const DROP_AFTER_CLASS: &str = "rinch-sortable__item--drop-after";
const DROP_ACROSS_CLASS: &str = "rinch-sortable__item--drop-horizontal";
const ITEM_REFUSED_CLASS: &str = "rinch-sortable__item--refused";
const DROP_INTO_CLASS: &str = "rinch-sortable--drop-into";
const LIST_REFUSED_CLASS: &str = "rinch-sortable--refused";
const REASON_CLASS: &str = "rinch-sortable__reason";
const REASON_SHOWN_CLASS: &str = "rinch-sortable__reason--shown";
const REASON_ABOVE_CLASS: &str = "rinch-sortable__reason--above";
const DISABLED_ATTR: &str = "data-sortable-disabled";
/// The keys that lift and drop when a list names none.
const DEFAULT_LIFT_KEYS: [&str; 2] = [" ", "Enter"];
/// Inline style that keeps the live region off screen without the stylesheet.
const LIVE_STYLE: &str = "position: absolute; width: 1px; height: 1px; margin: -1px; \
     padding: 0; border: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap";

// =============================================================================
// Public types
// =============================================================================

/// The axis a list's items run along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortAxis {
    /// Top to bottom (a column of cards, a stack of lanes).
    #[default]
    Vertical,
    /// Left to right (a row of columns, a tab strip).
    Horizontal,
}

/// Where in a list a drop lands, named by the items around it — not by an
/// index alone, so a reorder elsewhere in the list between the offer and the
/// app's write does not move it.
///
/// `after` is the item the carried items would follow and `before` the one
/// they would precede, both among the list's items with the carried ones left
/// out. First in the list: no `after`; last: no `before`; an empty list:
/// neither.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortPlace {
    /// The key of the item right before the drop, `None` at the start.
    pub after: Option<String>,
    /// The key of the item right after the drop, `None` at the end.
    pub before: Option<String>,
    /// Where the first carried item would sit in the list once the carried
    /// items are taken out of it and put back here.
    pub index: usize,
}

impl SortPlace {
    /// First in the list (an empty list's only place included).
    pub fn is_first(&self) -> bool {
        self.after.is_none()
    }

    /// Last in the list (an empty list's only place included).
    pub fn is_last(&self) -> bool {
        self.before.is_none()
    }
}

/// A move on offer or delivered: which items, from which list to which, and
/// where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortMove {
    /// The group both lists belong to.
    pub group: String,
    /// The keys carried, in order: the dragged item's, or what the source
    /// list's `drag_keys` named for it — as given, not checked against any
    /// list.
    pub keys: Vec<String>,
    /// The `id` of the list the drag started in: the pressed item's list,
    /// even when `drag_keys` carries items of other lists too.
    pub from_list: String,
    /// The `id` of the list the drop lands in.
    pub to_list: String,
    /// Where in `to_list`.
    pub place: SortPlace,
}

/// A list's verdict on a move ([`SortableList::judge`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortVerdict {
    /// The drop may happen.
    Accept,
    /// It may not; the reason is shown at the caret while the drag is over
    /// the place (an empty reason shows the refused caret alone).
    Refuse(String),
}

impl SortVerdict {
    /// Whether the verdict accepts.
    pub fn is_accept(&self) -> bool {
        matches!(self, SortVerdict::Accept)
    }
}

/// How a list judges a move into it.
pub type SortJudge = Rc<dyn Fn(&SortMove) -> SortVerdict>;
/// What a drag from an item carries: given the item's key, the keys moved
/// together, in order.
pub type SortKeys = Rc<dyn Fn(&str) -> Vec<String>>;

// =============================================================================
// Registry and drag state
// =============================================================================

/// One rendered list.
struct ListConfig {
    group: String,
    id: String,
    axis: SortAxis,
    label: String,
    disabled: bool,
    lift_keys: Vec<String>,
    judge: Option<SortJudge>,
    ondrop: Option<ValueCallback<SortMove>>,
    ondropchange: Option<ValueCallback<Option<(SortMove, SortVerdict)>>>,
    drag_keys: Option<SortKeys>,
    node: NodeHandle,
    /// The reason slot for a drop into the list itself (an empty list).
    reason: NodeHandle,
    /// The `aria-live` region the keyboard flow announces in.
    live: NodeHandle,
}

impl ListConfig {
    fn name(&self) -> String {
        if self.label.is_empty() {
            self.id.clone()
        } else {
            self.label.clone()
        }
    }

    /// Whether `ev` is one of this list's lift keys.
    fn is_lift_key(&self, ev: &KeyEventData) -> bool {
        let matches = |k: &str| k == ev.key || (k == " " && ev.is_space());
        if self.lift_keys.is_empty() {
            DEFAULT_LIFT_KEYS.iter().any(|k| matches(k))
        } else {
            self.lift_keys.iter().any(|k| matches(k))
        }
    }
}

/// Where the caret is drawn.
#[derive(Clone)]
enum Mark {
    Before(NodeHandle),
    After(NodeHandle),
    Into(NodeHandle),
}

impl Mark {
    fn node(&self) -> &NodeHandle {
        match self {
            Mark::Before(n) | Mark::After(n) | Mark::Into(n) => n,
        }
    }

    fn same(&self, other: &Mark) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.node().node_id() == other.node().node_id()
    }
}

struct Offer {
    /// The target list, by id (a re-render issues it a new token).
    list_id: String,
    slot: usize,
    mv: SortMove,
    verdict: SortVerdict,
    mark: Mark,
    reason: Option<NodeHandle>,
    horizontal: bool,
}

/// A keyboard lift: the list (by id) and slot the caret is at.
struct Lift {
    list_id: String,
    slot: usize,
    _keys: Option<DismissHandle>,
}

/// The drag in progress (one per thread: one pointer, one focus).
struct Active {
    group: String,
    keys: Vec<String>,
    /// The source list, held: a re-render may unregister it mid-drag, and its
    /// `ondropchange` still hears the drag out.
    from: Rc<ListConfig>,
    from_list: String,
    doc: u64,
    source: NodeHandle,
    source_key: String,
    carried: Vec<NodeHandle>,
    offer: Option<Offer>,
    /// Whether the source list's `ondropchange` last heard a move.
    told: bool,
    /// A target was left; unless the same move reaches another, the offer is
    /// withdrawn at the source's drag move.
    pending_none: bool,
    lift: Option<Lift>,
    /// The source item's node went away mid-drag (the app re-rendered it):
    /// the runtime's drag moves go to the gone node, so a left target
    /// withdraws at once. The end is the runtime's ([`on_drag_end`]).
    source_gone: bool,
}

/// (document, group, key, the list it was dropped in).
type PendingFocus = (u64, String, String, Option<String>);

thread_local! {
    static LISTS: RefCell<HashMap<u64, Rc<ListConfig>>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    /// The item to focus once it is drawn: (document, group, key, the list it
    /// was dropped in).
    static PENDING_FOCUS: RefCell<Option<PendingFocus>> = const { RefCell::new(None) };
    static DRAG_END_HOOKED: Cell<bool> = const { Cell::new(false) };
}

fn list_config(token: u64) -> Option<Rc<ListConfig>> {
    LISTS.with(|l| l.borrow().get(&token).cloned())
}

fn token_of(node: &NodeHandle) -> Option<u64> {
    node.get_attribute(LIST_ATTR)?.parse().ok()
}

/// The nearest list above `node` (an item's own list).
fn list_of(node: &NodeHandle) -> Option<Rc<ListConfig>> {
    let mut at = node.parent_node();
    while let Some(n) = at {
        if let Some(token) = token_of(&n) {
            return list_config(token);
        }
        at = n.parent_node();
    }
    None
}

/// The list of `group` named `id` in `doc`, as it is drawn now.
fn list_by_id(group: &str, id: &str, doc: u64) -> Option<Rc<ListConfig>> {
    LISTS.with(|l| {
        l.borrow()
            .values()
            .find(|c| c.group == group && c.id == id && c.node.doc_key() == doc)
            .cloned()
    })
}

/// A list's own items in order: the descendants carrying a key, not looking
/// inside an item or a nested list.
fn items_of(list: &NodeHandle) -> Vec<(NodeHandle, String)> {
    fn walk(node: &NodeHandle, out: &mut Vec<(NodeHandle, String)>) {
        for child in node.children() {
            if let Some(key) = child.get_attribute(KEY_ATTR) {
                out.push((child, key));
            } else if child.get_attribute(LIST_ATTR).is_none() {
                walk(&child, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(list, &mut out);
    out
}

/// `node`'s position in its document: the child index at every level from the
/// root, so two of them compare in document order.
fn doc_path(node: &NodeHandle) -> Vec<usize> {
    let mut path = Vec::new();
    let mut at = node.clone();
    while let Some(parent) = at.parent_node() {
        let index = parent
            .children()
            .iter()
            .position(|c| c.node_id() == at.node_id())
            .unwrap_or(0);
        path.push(index);
        at = parent;
    }
    path.reverse();
    path
}

/// Every registered list of `group` in `doc`, in document order.
fn lists_of_group(group: &str, doc: u64) -> Vec<Rc<ListConfig>> {
    let lists: Vec<Rc<ListConfig>> = LISTS.with(|l| {
        l.borrow()
            .values()
            .filter(|c| c.group == group && c.node.doc_key() == doc && c.node.is_valid())
            .cloned()
            .collect()
    });
    let mut keyed: Vec<(Vec<usize>, Rc<ListConfig>)> =
        lists.into_iter().map(|c| (doc_path(&c.node), c)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, c)| c).collect()
}

/// Whether `node` is inside an item of `group` that the drag carries: a list
/// there is the carried item's own and can take nothing from the drag.
fn inside_carried(node: &NodeHandle, group: &str, keys: &[String]) -> bool {
    let mut at = node.parent_node();
    while let Some(n) = at {
        if n.get_attribute(KEY_ATTR).is_some_and(|k| keys.contains(&k))
            && list_of(&n).is_some_and(|c| c.group == group)
        {
            return true;
        }
        at = n.parent_node();
    }
    false
}

/// A target list and the item of it under the pointer, if any.
type Target = (Rc<ListConfig>, Option<(NodeHandle, String)>);

/// The list of the dragged group a drag over `node` targets, and the item of
/// that list under the pointer, if any. Walking up, an item is remembered, a
/// list of another group (or a disabled one) forgets it, and the first list of
/// `group` answers — unless an item the drag carries is further up between
/// that list and an outer list of the group: a list inside the carried item is
/// its own, so the outer list answers, over the carried item (its place).
fn resolve_target(node: &NodeHandle, group: &str, keys: &[String]) -> Option<Target> {
    let mut item: Option<(NodeHandle, String)> = None;
    let mut found: Option<Target> = None;
    let mut at = Some(node.clone());
    while let Some(n) = at {
        if let Some(key) = n.get_attribute(KEY_ATTR) {
            item = Some((n.clone(), key));
        } else if let Some(token) = token_of(&n) {
            if let Some(cfg) = list_config(token).filter(|c| c.group == group && !c.disabled) {
                let over_carried = item.as_ref().is_some_and(|(_, k)| keys.contains(k));
                if found.is_none() || over_carried {
                    found = Some((cfg, item.clone()));
                }
            }
            item = None;
        }
        at = n.parent_node();
    }
    found
}

// =============================================================================
// Places, judging and drawing
// =============================================================================

/// The list's items with the carried ones left out, and every key in order.
fn rest_of(list: &NodeHandle, carried: &[String]) -> (Vec<(NodeHandle, String)>, Vec<String>) {
    let all = items_of(list);
    let keys: Vec<String> = all.iter().map(|(_, k)| k.clone()).collect();
    let rest = all
        .into_iter()
        .filter(|(_, k)| !carried.contains(k))
        .collect();
    (rest, keys)
}

/// Whether putting `carried` at `slot` of `rest` leaves the list as it is.
fn is_no_op(
    all: &[String],
    rest: &[(NodeHandle, String)],
    carried: &[String],
    slot: usize,
) -> bool {
    if !carried.iter().all(|k| all.contains(k)) {
        return false;
    }
    let mut after: Vec<&String> = rest.iter().map(|(_, k)| k).collect();
    let tail = after.split_off(slot.min(after.len()));
    after.extend(carried.iter());
    after.extend(tail);
    after.len() == all.len() && after.iter().zip(all).all(|(a, b)| *a == b)
}

/// The start and length of `b` along `axis`.
fn span(b: &ElementBounds, axis: SortAxis) -> (f32, f32) {
    match axis {
        SortAxis::Vertical => (b.y, b.height),
        SortAxis::Horizontal => (b.x, b.width),
    }
}

/// A place worked out against a list as it is drawn now.
struct Placed {
    slot: usize,
    mv: SortMove,
    mark: Mark,
    reason: NodeHandle,
}

/// The move `slot` of `cfg` would be for the active drag; `None` when there is
/// no drag or the place would leave the list as it is.
fn place_in(cfg: &ListConfig, slot: usize) -> Option<Placed> {
    let (group, keys, from_list) = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.group.clone(), a.keys.clone(), a.from_list.clone()))
    })?;
    let (rest, all) = rest_of(&cfg.node, &keys);
    let slot = slot.min(rest.len());
    if is_no_op(&all, &rest, &keys, slot) {
        return None;
    }
    let (mark, reason) = if rest.is_empty() {
        (Mark::Into(cfg.node.clone()), cfg.reason.clone())
    } else if slot == 0 {
        (Mark::Before(rest[0].0.clone()), reason_slot(&rest[0].0))
    } else {
        let item = rest[slot - 1].0.clone();
        let reason = reason_slot(&item);
        (Mark::After(item), reason)
    };
    let place = SortPlace {
        after: slot.checked_sub(1).map(|i| rest[i].1.clone()),
        before: rest.get(slot).map(|(_, k)| k.clone()),
        index: slot,
    };
    Some(Placed {
        slot,
        mv: SortMove {
            group,
            keys,
            from_list,
            to_list: cfg.id.clone(),
            place,
        },
        mark,
        reason,
    })
}

fn judge(cfg: &ListConfig, mv: &SortMove) -> SortVerdict {
    cfg.judge.as_ref().map_or(SortVerdict::Accept, |judge| {
        rinch_core::untracked_handler(|| judge(mv))
    })
}

/// Make `slot` of `cfg` the place on offer for the active drag, judging it
/// and drawing the caret.
fn offer_slot(cfg: &ListConfig, slot: usize) {
    let Some(placed) = place_in(cfg, slot) else {
        set_offer(None);
        return;
    };
    // The same place drawn on the same node: nothing to judge or redraw. (A
    // re-render that replaced the node draws it again.)
    let unchanged = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|a| a.offer.as_ref())
            .is_some_and(|o| {
                o.list_id == cfg.id && o.slot == placed.slot && o.mark.same(&placed.mark)
            })
    });
    if unchanged {
        return;
    }
    let verdict = judge(cfg, &placed.mv);
    set_offer(Some(Offer {
        list_id: cfg.id.clone(),
        slot: placed.slot,
        mv: placed.mv,
        verdict,
        mark: placed.mark,
        reason: Some(placed.reason),
        horizontal: cfg.axis == SortAxis::Horizontal,
    }));
}

/// The drop: the place on offer worked out again against the lists as they
/// are now — the target list looked up by id, the slot found again by the
/// place's keys (after `after`; first when there was none) — and judged
/// again. The offer was made against the lists as they were at the last
/// move; a re-render since (a collaborator's edit) can have moved, removed or
/// changed what it named. `None` when nothing is on offer, the place is gone
/// (its `after` item left the list), it is now a no-op, or it is refused.
fn settle() -> Option<(Rc<ListConfig>, SortMove)> {
    let (group, doc, list_id, place, keys) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let o = a.offer.as_ref()?;
        Some((
            a.group.clone(),
            a.doc,
            o.list_id.clone(),
            o.mv.place.clone(),
            a.keys.clone(),
        ))
    })?;
    let cfg = list_by_id(&group, &list_id, doc).filter(|c| !c.disabled)?;
    let (rest, _) = rest_of(&cfg.node, &keys);
    let slot = match &place.after {
        None => 0,
        Some(after) => rest.iter().position(|(_, k)| k == after)? + 1,
    };
    let placed = place_in(&cfg, slot)?;
    judge(&cfg, &placed.mv)
        .is_accept()
        .then_some((cfg, placed.mv))
}

/// An item's reason slot: its last child carrying the class.
fn reason_slot(item: &NodeHandle) -> NodeHandle {
    item.children()
        .into_iter()
        .rev()
        .find(|c| {
            c.get_attribute("class")
                .is_some_and(|cls| cls.split_whitespace().any(|w| w == REASON_CLASS))
        })
        .unwrap_or_else(|| item.clone())
}

fn draw(offer: &Offer) {
    let refused = !offer.verdict.is_accept();
    match &offer.mark {
        Mark::Before(item) | Mark::After(item) => {
            item.add_class(if matches!(offer.mark, Mark::Before(_)) {
                DROP_BEFORE_CLASS
            } else {
                DROP_AFTER_CLASS
            });
            if offer.horizontal {
                item.add_class(DROP_ACROSS_CLASS);
            }
            if refused {
                item.add_class(ITEM_REFUSED_CLASS);
            }
        }
        Mark::Into(list) => {
            list.add_class(DROP_INTO_CLASS);
            if refused {
                list.add_class(LIST_REFUSED_CLASS);
            }
        }
    }
    if let (SortVerdict::Refuse(reason), Some(slot)) = (&offer.verdict, &offer.reason)
        && !reason.is_empty()
        && slot.get_attribute(KEY_ATTR).is_none()
    {
        slot.set_text(reason);
        slot.add_class(REASON_SHOWN_CLASS);
        slot.set_style("display", "block");
        if matches!(offer.mark, Mark::Before(_)) {
            slot.add_class(REASON_ABOVE_CLASS);
        }
    }
}

fn erase(offer: &Offer) {
    match &offer.mark {
        Mark::Before(item) | Mark::After(item) => {
            for class in [
                DROP_BEFORE_CLASS,
                DROP_AFTER_CLASS,
                DROP_ACROSS_CLASS,
                ITEM_REFUSED_CLASS,
            ] {
                item.remove_class(class);
            }
        }
        Mark::Into(list) => {
            list.remove_class(DROP_INTO_CLASS);
            list.remove_class(LIST_REFUSED_CLASS);
        }
    }
    if let Some(slot) = &offer.reason
        && slot.get_attribute(KEY_ATTR).is_none()
    {
        slot.remove_class(REASON_SHOWN_CLASS);
        slot.remove_class(REASON_ABOVE_CLASS);
        slot.set_style("display", "none");
        slot.set_text("");
    }
}

/// Replace the offer, redraw, and tell the source list's `ondropchange`.
fn set_offer(new: Option<Offer>) {
    let Some((old, from, report)) = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut()?;
        a.pending_none = false;
        if new.is_none() && a.offer.is_none() && !a.told {
            return None;
        }
        let report = new.as_ref().map(|o| (o.mv.clone(), o.verdict.clone()));
        a.told = report.is_some();
        Some((a.offer.take(), a.from.clone(), report))
    }) else {
        return;
    };
    if let Some(old) = &old {
        erase(old);
    }
    if let Some(new) = &new {
        draw(new);
    }
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.offer = new;
        }
    });
    if let Some(cb) = from.ondropchange.clone() {
        cb.invoke(report);
    }
}

/// End the active drag: take the caret and the dimming down; `ondropchange`
/// hears `None` if it last heard a move.
fn end_active() {
    set_offer(None);
    let Some(active) = ACTIVE.with(|a| a.borrow_mut().take()) else {
        return;
    };
    for node in &active.carried {
        node.remove_class(DRAGGING_CLASS);
        node.remove_class(LIFTED_CLASS);
    }
}

/// The runtime ended an element drag (dropped or cancelled): a pointer drag of
/// ours ends with it, whatever became of the source node its `data-ondragend`
/// was sent to.
fn on_drag_end() {
    let pointer = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_none()));
    if pointer {
        end_active();
    }
}

fn hook_drag_end() {
    if !DRAG_END_HOOKED.with(|h| h.replace(true)) {
        rinch_core::on_element_drag_end(on_drag_end);
    }
}

/// The carried items (in any list of the group).
fn carried_nodes(group: &str, doc: u64, keys: &[String]) -> Vec<NodeHandle> {
    let mut out = Vec::new();
    for cfg in lists_of_group(group, doc) {
        for (node, key) in items_of(&cfg.node) {
            if keys.contains(&key) {
                out.push(node);
            }
        }
    }
    out
}

/// The keys a drag from `key` in `cfg` carries.
fn keys_for(cfg: &ListConfig, key: &str) -> Vec<String> {
    let keys = cfg
        .drag_keys
        .as_ref()
        .map(|f| rinch_core::untracked_handler(|| f(key)))
        .unwrap_or_default();
    if keys.iter().any(|k| k == key) {
        keys
    } else {
        vec![key.to_string()]
    }
}

/// Start a drag of `item` (pointer or keyboard). Refused for a disabled item
/// or list.
fn begin(item: &NodeHandle, key: &str, keyboard: bool) -> bool {
    end_active();
    PENDING_FOCUS.with(|p| p.borrow_mut().take());
    if item.get_attribute(DISABLED_ATTR).is_some() {
        return false;
    }
    let Some(cfg) = list_of(item).filter(|c| !c.disabled) else {
        return false;
    };
    let keys = keys_for(&cfg, key);
    let doc = item.doc_key();
    let carried = carried_nodes(&cfg.group, doc, &keys);
    for node in &carried {
        node.add_class(if keyboard {
            LIFTED_CLASS
        } else {
            DRAGGING_CLASS
        });
    }
    ACTIVE.with(|a| {
        *a.borrow_mut() = Some(Active {
            group: cfg.group.clone(),
            keys,
            from: cfg.clone(),
            from_list: cfg.id.clone(),
            doc,
            source: item.clone(),
            source_key: key.to_string(),
            carried,
            offer: None,
            told: false,
            pending_none: false,
            lift: None,
            source_gone: false,
        })
    });
    true
}

fn active_group() -> Option<(String, bool)> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.group.clone(), a.lift.is_some()))
    })
}

fn active_keys() -> Vec<String> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| a.keys.clone())
            .unwrap_or_default()
    })
}

// =============================================================================
// Pointer handlers
// =============================================================================

/// A drag moved over `target` (an item or a list): offer the place under the
/// pointer.
fn drag_over(target: &NodeHandle) {
    let Some((group, false)) = active_group() else {
        return;
    };
    let keys = active_keys();
    let Some((cfg, item)) = resolve_target(target, &group, &keys) else {
        set_offer(None);
        return;
    };
    let ctx = get_click_context();
    let pointer = match cfg.axis {
        SortAxis::Vertical => ctx.mouse_y,
        SortAxis::Horizontal => ctx.mouse_x,
    };
    // The resolved node's box: the handler's own from the click context, any
    // other (a delegated item) asked of the document.
    let bounds_of = |node: &NodeHandle| -> Option<ElementBounds> {
        if node.node_id() == target.node_id() && ctx.element_width + ctx.element_height > 0.0 {
            Some(ElementBounds {
                x: ctx.element_x,
                y: ctx.element_y,
                width: ctx.element_width,
                height: ctx.element_height,
            })
        } else {
            node.bounding_rect()
        }
    };
    let all = items_of(&cfg.node);
    let mut slot = 0;
    match item {
        Some((node, key)) => {
            let mut carried_before = false;
            for (_, k) in &all {
                if *k == key {
                    break;
                }
                if keys.contains(k) {
                    carried_before = true;
                } else {
                    slot += 1;
                }
            }
            if !keys.contains(&key) {
                let past_middle = match bounds_of(&node) {
                    Some(b) => {
                        let (start, len) = span(&b, cfg.axis);
                        pointer >= start + len / 2.0
                    }
                    // No box to measure by (a host that cannot answer
                    // `bounding_rect`): over an item the carried ones are
                    // before, after it — the drag is moving forward past it —
                    // else before it.
                    None => carried_before,
                };
                if past_middle {
                    slot += 1;
                }
            }
        }
        None => {
            for (node, k) in &all {
                if keys.contains(k) {
                    continue;
                }
                match node.bounding_rect() {
                    Some(b) => {
                        let (start, len) = span(&b, cfg.axis);
                        if pointer >= start + len / 2.0 {
                            slot += 1;
                        }
                    }
                    // No boxes to measure by: the end.
                    None => slot += 1,
                }
            }
        }
    }
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.pending_none = false;
        }
    });
    offer_slot(&cfg, slot);
}

fn drag_leave() {
    let now = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let Some(a) = a.as_mut().filter(|a| a.lift.is_none()) else {
            return false;
        };
        a.pending_none = !a.source_gone;
        a.source_gone
    });
    if now {
        set_offer(None);
    }
}

/// After every move's targets: a left target nothing replaced withdraws.
fn drag_move() {
    let pending = ACTIVE.with(|a| {
        a.borrow_mut()
            .as_mut()
            .is_some_and(|a| std::mem::replace(&mut a.pending_none, false))
    });
    if pending {
        set_offer(None);
    }
}

/// The release: settle the offer against the lists as they are, deliver it if
/// still accepted, then end.
fn drop_here() {
    let pointer = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_none()));
    if !pointer {
        return;
    }
    let delivered = settle();
    end_active();
    if let Some((cfg, mv)) = delivered
        && let Some(cb) = cfg.ondrop.clone()
    {
        cb.invoke(mv);
    }
}

// =============================================================================
// Keyboard
// =============================================================================

/// An arrow, as a direction on screen: (dx, dy).
fn arrow(key: &str) -> Option<(i32, i32)> {
    match key {
        "ArrowUp" => Some((0, -1)),
        "ArrowDown" => Some((0, 1)),
        "ArrowLeft" => Some((-1, 0)),
        "ArrowRight" => Some((1, 0)),
        _ => None,
    }
}

fn center(b: &ElementBounds) -> (f32, f32) {
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

/// The nearest list of the group from `from` in direction `dir`, by on-screen
/// boxes; without boxes, the previous or next in document order. Disabled
/// lists and lists inside a carried item are not places.
fn neighbour(from: &ListConfig, dir: (i32, i32), keys: &[String]) -> Option<Rc<ListConfig>> {
    let lists: Vec<Rc<ListConfig>> = lists_of_group(&from.group, from.node.doc_key())
        .into_iter()
        .filter(|c| c.id == from.id || (!c.disabled && !inside_carried(&c.node, &from.group, keys)))
        .collect();
    let at = lists.iter().position(|c| c.id == from.id)?;
    let Some(here) = from.node.bounding_rect() else {
        let forward = dir.0 + dir.1 > 0;
        return if forward {
            lists.get(at + 1).cloned()
        } else {
            at.checked_sub(1).map(|i| lists[i].clone())
        };
    };
    let (hx, hy) = center(&here);
    let mut best: Option<(f32, Rc<ListConfig>)> = None;
    for cfg in &lists {
        if cfg.id == from.id {
            continue;
        }
        let Some(b) = cfg.node.bounding_rect() else {
            continue;
        };
        if b.width <= 0.0 && b.height <= 0.0 {
            continue;
        }
        let (cx, cy) = center(&b);
        let (along, across) = if dir.0 != 0 {
            ((cx - hx) * dir.0 as f32, (cy - hy).abs())
        } else {
            ((cy - hy) * dir.1 as f32, (cx - hx).abs())
        };
        if along <= 1.0 {
            continue;
        }
        let score = along + 2.0 * across;
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, cfg.clone()));
        }
    }
    best.map(|(_, c)| c)
}

/// Lift `item` for a keyboard move.
fn lift(item: &NodeHandle, key: &str) {
    if !begin(item, key, true) {
        return;
    }
    let Some(cfg) = list_of(item) else {
        return;
    };
    let keys = active_keys();
    let home = items_of(&cfg.node)
        .iter()
        .take_while(|(_, k)| *k != key)
        .filter(|(_, k)| !keys.contains(k))
        .count();
    let entry = focus_leave_entry(item);
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.lift = Some(Lift {
                list_id: cfg.id.clone(),
                slot: home,
                _keys: Some(entry),
            });
        }
    });
    let label = label_of(item, key);
    announce(&format!(
        "Picked up {label}. Use the arrow keys to move it, the same key to drop it, \
         Escape to cancel."
    ));
}

/// A key entry (taking no key) whose focus-leave cancels the lift of `item`:
/// when the focus is now on another element. A focus that went nowhere known
/// (the lifted node was re-rendered away) is not a leave.
fn focus_leave_entry(item: &NodeHandle) -> DismissHandle {
    let owner = item.clone();
    rinch_core::push_key_handler(
        item,
        |_| false,
        move || {
            let lifted = ACTIVE.with(|a| {
                a.borrow()
                    .as_ref()
                    .filter(|a| a.lift.is_some())
                    .map(|a| a.source.clone())
            });
            let Some(source) = lifted else { return };
            let Some(focus) = owner.active_element() else {
                return;
            };
            let mut at = Some(focus);
            while let Some(n) = at {
                if n.node_id() == source.node_id() {
                    return;
                }
                at = n.parent_node();
            }
            cancel_lift(true);
        },
    )
}

fn label_of(item: &NodeHandle, key: &str) -> String {
    item.get_attribute("data-sortable-label")
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| key.to_string())
}

/// Say `text` in the live region of the drag's source list, as it is drawn
/// now (a re-render of its lane gives it a new region), and say it even when
/// it repeats the last (an alternating trailing no-break space changes it).
fn announce(text: &str) {
    let region = ACTIVE.with(|a| {
        a.borrow().as_ref().map(|a| {
            list_by_id(&a.group, &a.from_list, a.doc)
                .map_or_else(|| a.from.live.clone(), |c| c.live.clone())
        })
    });
    if let Some(region) = region {
        announce_in(&region, text);
    }
}

fn announce_in(region: &NodeHandle, text: &str) {
    // The same sentence twice in a row (an arrow at the end of the lists) is
    // a change a screen reader still hears: a trailing no-break space, on
    // every other repeat.
    let current = region.text_content().unwrap_or_default();
    let text = if current.trim_end_matches('\u{a0}') == text && !current.ends_with('\u{a0}') {
        format!("{text}\u{a0}")
    } else {
        text.to_string()
    };
    region.set_text(&text);
}

/// The list a keyboard lift's caret is in, as drawn now.
fn lift_list() -> Option<(Rc<ListConfig>, usize)> {
    let (group, doc, list_id, slot) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let lift = a.lift.as_ref()?;
        Some((a.group.clone(), a.doc, lift.list_id.clone(), lift.slot))
    })?;
    Some((list_by_id(&group, &list_id, doc)?, slot))
}

/// Move a keyboard lift one step in `dir`.
fn move_lift(dir: (i32, i32)) {
    let Some((cfg, slot)) = lift_list() else {
        return;
    };
    let keys = active_keys();
    let len = rest_of(&cfg.node, &keys).0.len();
    let slot = slot.min(len);
    let (forward, along) = match cfg.axis {
        SortAxis::Vertical => (dir.1 > 0, dir.0 == 0),
        SortAxis::Horizontal => (dir.0 > 0, dir.1 == 0),
    };
    let (cfg, slot) = if along && forward && slot < len {
        (cfg, slot + 1)
    } else if along && !forward && slot > 0 {
        (cfg, slot - 1)
    } else if let Some(next) = neighbour(&cfg, dir, &keys) {
        let next_len = rest_of(&next.node, &keys).0.len();
        let slot = if along {
            if forward { 0 } else { next_len }
        } else {
            slot.min(next_len)
        };
        (next, slot)
    } else {
        (cfg, slot)
    };
    ACTIVE.with(|a| {
        if let Some(lift) = a.borrow_mut().as_mut().and_then(|a| a.lift.as_mut()) {
            lift.list_id = cfg.id.clone();
            lift.slot = slot;
        }
    });
    offer_slot(&cfg, slot);
    describe_lift(&cfg, slot);
}

/// Scroll the caret into view and announce where the lift is.
fn describe_lift(cfg: &ListConfig, slot: usize) {
    let (label, verdict, mark) = ACTIVE.with(|a| {
        let a = a.borrow();
        let Some(a) = a.as_ref() else {
            return (String::new(), None, None);
        };
        let label = label_of(&a.source, &a.source_key);
        let label = if a.keys.len() > 1 {
            format!("{label} and {} more", a.keys.len() - 1)
        } else {
            label
        };
        let offer = a.offer.as_ref();
        (
            label,
            offer.map(|o| o.verdict.clone()),
            offer.map(|o| o.mark.clone()),
        )
    });
    if let Some(mark) = &mark {
        mark.node().scroll_into_view();
    }
    let list = cfg.name();
    let text = match verdict {
        None => format!("{label}: back where it was."),
        Some(SortVerdict::Accept) => format!("{label}: {list}, position {}.", slot + 1),
        Some(SortVerdict::Refuse(reason)) => {
            format!(
                "{label}: {list}, position {}. Cannot drop here. {reason}",
                slot + 1
            )
        }
    };
    announce(&text);
}

/// Drop a keyboard lift where its caret is: settled and judged again first.
fn drop_lift() {
    let Some((group, doc, first)) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        a.lift.as_ref()?;
        Some((a.group.clone(), a.doc, a.keys.first().cloned()))
    }) else {
        return;
    };
    let delivered = settle();
    match &delivered {
        Some((cfg, mv)) => announce(&format!(
            "Dropped in {}, position {}.",
            cfg.name(),
            mv.place.index + 1
        )),
        None => announce("Not moved."),
    }
    end_active();
    let Some((cfg, mv)) = delivered else {
        return;
    };
    let cfg_id = Some(cfg.id.clone());
    if let Some(cb) = cfg.ondrop.clone() {
        cb.invoke(mv);
    }
    // The app re-renders from its data — in this handler's batch, or later —
    // and the moved item (a new node, when it changed lists) takes the focus
    // back once it is drawn.
    if let Some(first) = first {
        request_focus(doc, group, first, cfg_id);
    }
}

/// Focus the item `key` of `group` once it is drawn in `list`: at every
/// render of it, and on the next turn of the event loop (timers permitting)
/// if it is already there. Forgotten at the next drag.
fn request_focus(doc: u64, group: String, key: String, list: Option<String>) {
    PENDING_FOCUS.with(|p| *p.borrow_mut() = Some((doc, group, key, list)));
    // Unowned: the item that parked it is usually the node the re-render
    // replaced, and a parked callback of a disposed owner is dropped. Where
    // no timer runs, the render of the item is the only place it is focused.
    rinch_core::unowned(|| set_timeout(0, try_pending_focus));
}

fn try_pending_focus() {
    let Some((doc, group, key, list)) = PENDING_FOCUS.with(|p| p.borrow().clone()) else {
        return;
    };
    // Not yet where it was dropped (the app re-renders later): its render
    // there focuses it.
    for cfg in lists_of_group(&group, doc)
        .into_iter()
        .filter(|c| list.as_ref().is_none_or(|l| *l == c.id))
    {
        if let Some((node, _)) = items_of(&cfg.node).into_iter().find(|(_, k)| *k == key) {
            PENDING_FOCUS.with(|p| p.borrow_mut().take());
            node.focus();
            return;
        }
    }
}

/// Cancel a keyboard lift (`focus_left`: the focus moved away).
fn cancel_lift(focus_left: bool) {
    let lifted = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_some()));
    if !lifted {
        return;
    }
    if !focus_left {
        announce("Cancelled. Not moved.");
    }
    end_active();
}

/// An item's `onkeydown`.
fn item_key(item: &NodeHandle, key: &str, ev: &KeyEventData) -> bool {
    if ev.kind != rinch_core::events::KeyEventKind::Down {
        return false;
    }
    // Only for the item itself: a key in a field or a button inside it is
    // that control's. The event names its target where the backend knows it;
    // else the document's focus.
    let elsewhere = match ev.target {
        Some(target) => target != item.node_id(),
        None => item
            .active_element()
            .is_some_and(|f| f.node_id() != item.node_id()),
    };
    if elsewhere {
        return false;
    }
    let Some(cfg) = list_of(item) else {
        return false;
    };
    let lifted_here = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.lift.is_some() && a.source.node_id() == item.node_id())
    });
    let modified = ev.ctrl || ev.alt || ev.meta;
    let activate = cfg.is_lift_key(ev) && !modified;
    if lifted_here {
        if let Some(dir) = arrow(&ev.key) {
            move_lift(dir);
            return true;
        }
        if activate {
            drop_lift();
            return true;
        }
        if ev.key == "Escape" {
            cancel_lift(false);
            return true;
        }
        if ev.key == "Tab" {
            cancel_lift(false);
        }
        return false;
    }
    let pointer_drag = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.lift.is_none() && !a.source_gone)
    });
    if activate && !pointer_drag && !ev.shift && !cfg.disabled {
        lift(item, key);
        return ACTIVE.with(|a| a.borrow().is_some());
    }
    false
}

// =============================================================================
// Components
// =============================================================================

/// A list of sortable items ([`SortableItem`]) of one `group`.
///
/// A drag from one of its items can land in any list of the same group; the
/// list it lands in judges it (`judge`) and delivers it (`ondrop`). Lists of
/// other groups — nested inside its items, or around it — are never targeted,
/// and nor is a list of the same group inside a carried item. Lays its
/// children out as a flex row or column (`axis`), with `--rinch-sortable-gap`
/// between them; `class:` and `style:` override it.
///
/// Within a group, list `id`s are unique in a document, and so are item keys.
pub struct SortableList {
    /// The kind of thing the list holds ("cards", "columns", …). A drag only
    /// targets lists of its own group.
    pub group: String,
    /// The list's id, as `SortMove::from_list` / `to_list` name it.
    pub id: String,
    /// Which way the items run.
    pub axis: SortAxis,
    /// The list's name in keyboard announcements (its `id` when empty).
    pub label: String,
    /// A read-only list: no drag starts from its items and none lands in it.
    /// Its items still draw; give them `disabled` too to take their drag
    /// source away entirely (a press then never becomes a drag).
    pub disabled: bool,
    /// The keys (`KeyEventData::key`) that lift a focused item and drop it:
    /// Space and Enter when empty. `vec![" ".into()]` leaves Enter to the app
    /// (an item that opens on Enter).
    pub lift_keys: Vec<String>,
    /// Whether a move into this list may happen, asked for every place on
    /// offer, live, and again at the drop; a refusal's reason is shown at the
    /// caret. Accepted when not given.
    pub judge: Option<SortJudge>,
    /// An accepted drop into this list: apply it to the app's data.
    pub ondrop: Option<ValueCallback<SortMove>>,
    /// For a drag that started in this list: the move on offer and its
    /// verdict each time it changes, `None` when nothing is on offer (and when
    /// the drag ends).
    pub ondropchange: Option<ValueCallback<Option<(SortMove, SortVerdict)>>>,
    /// What a drag from an item of this list carries (the item alone when not
    /// given, or when the answer leaves the item out): a selection moved
    /// together, in order. The keys are not checked: one that names no item
    /// is passed on as given.
    pub drag_keys: Option<SortKeys>,
}

impl std::fmt::Debug for SortableList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortableList")
            .field("group", &self.group)
            .field("id", &self.id)
            .field("axis", &self.axis)
            .field("disabled", &self.disabled)
            .finish()
    }
}

impl Default for SortableList {
    fn default() -> Self {
        Self {
            group: String::new(),
            id: String::new(),
            axis: SortAxis::Vertical,
            label: String::new(),
            disabled: false,
            lift_keys: Vec::new(),
            judge: None,
            ondrop: None,
            ondropchange: None,
            drag_keys: None,
        }
    }
}

/// Register a list's handlers as a drop target on `node`.
fn wire_target(scope: &mut RenderScope, node: &NodeHandle) {
    let target = node.clone();
    let over = scope.register_handler(move || drag_over(&target));
    node.set_attribute("data-ondragover", &over.0.to_string());
    let leave = scope.register_handler(drag_leave);
    node.set_attribute("data-ondragleave", &leave.0.to_string());
    let drop = scope.register_handler(drop_here);
    node.set_attribute("data-ondrop", &drop.0.to_string());
}

/// A hidden reason slot (hidden inline, so it stays out of the way where the
/// component stylesheet is missing).
fn reason_node(scope: &mut RenderScope) -> NodeHandle {
    let reason = scope.create_element("div");
    reason.set_attribute("class", REASON_CLASS);
    reason.set_attribute("aria-hidden", "true");
    reason.set_attribute("style", "position: absolute; display: none");
    reason
}

impl Component for SortableList {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        hook_drag_end();
        let list = __scope.create_element("div");
        list.set_attribute(
            "class",
            match self.axis {
                SortAxis::Vertical => "rinch-sortable rinch-sortable--vertical",
                SortAxis::Horizontal => "rinch-sortable rinch-sortable--horizontal",
            },
        );
        if self.disabled {
            list.add_class("rinch-sortable--disabled");
        }
        let token = NEXT_TOKEN.with(|n| {
            let t = n.get();
            n.set(t + 1);
            t
        });
        list.set_attribute(LIST_ATTR, &token.to_string());
        list.set_attribute("data-sortable-group", &self.group);
        for child in children {
            list.append_child(child);
        }
        let reason = reason_node(__scope);
        list.append_child(&reason);
        let live = __scope.create_element("div");
        live.set_attribute("class", "rinch-sortable__live");
        live.set_attribute("aria-live", "assertive");
        live.set_attribute("aria-atomic", "true");
        live.set_attribute("style", LIVE_STYLE);
        list.append_child(&live);

        wire_target(__scope, &list);
        let cfg = Rc::new(ListConfig {
            group: self.group.clone(),
            id: self.id.clone(),
            axis: self.axis,
            label: self.label.clone(),
            disabled: self.disabled,
            lift_keys: self.lift_keys.clone(),
            judge: self.judge.clone(),
            ondrop: self.ondrop.clone(),
            ondropchange: self.ondropchange.clone(),
            drag_keys: self.drag_keys.clone(),
            node: list.clone(),
            reason,
            live,
        });
        LISTS.with(|l| l.borrow_mut().insert(token, cfg));
        __scope.on_cleanup(move || {
            // A drag goes on through a re-render of any list (a co-edited
            // board re-renders under it): lists are found again by id.
            LISTS.with(|l| l.borrow_mut().remove(&token));
        });
        list
    }
}

/// An item of a [`SortableList`], named by `value`.
///
/// The whole item drags, or with `handle` only a [`SortableHandle`] inside it.
/// A press that never passes the drag threshold stays a click. Focused, a lift
/// key (Space or Enter, [`SortableList::lift_keys`]) lifts it for a keyboard
/// move.
#[derive(Debug, Default)]
pub struct SortableItem {
    /// The item's key, as `SortMove::keys` and `SortPlace` name it.
    pub value: String,
    /// Drag only by a [`SortableHandle`] inside the item.
    pub handle: bool,
    /// The item's name in keyboard announcements (its key when empty).
    pub label: String,
    /// Not draggable, by pointer or keyboard (it still takes drops around it).
    pub disabled: bool,
}

impl Component for SortableItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        hook_drag_end();
        let item = __scope.create_element("div");
        item.set_attribute("class", ITEM_CLASS);
        item.set_attribute(KEY_ATTR, &self.value);
        if !self.label.is_empty() {
            item.set_attribute("data-sortable-label", &self.label);
        }
        item.set_attribute("tabindex", "0");
        // The component draws the drag itself: a host that paints a drag
        // ghost reads this (the desktop runtime does).
        item.set_attribute("data-drag-ghost", "false");
        if self.disabled {
            item.set_attribute(DISABLED_ATTR, "");
        } else {
            item.set_attribute("aria-roledescription", "sortable item");
            item.set_attribute(
                "aria-description",
                "Press Space to lift, the arrow keys to move, Space to drop, Escape to cancel.",
            );
            if !self.handle {
                item.set_attribute("draggable", "true");
                item.add_class("rinch-sortable__item--grab");
            }
        }
        for child in children {
            item.append_child(child);
        }
        let reason = reason_node(__scope);
        item.append_child(&reason);

        let (node, key) = (item.clone(), self.value.clone());
        let start = __scope.register_handler(move || {
            rinch_core::suppress_drag_ghost();
            begin(&node, &key, false);
        });
        item.set_attribute("data-ondragstart", &start.0.to_string());
        let end = __scope.register_handler(on_drag_end);
        item.set_attribute("data-ondragend", &end.0.to_string());
        let moved = __scope.register_handler(drag_move);
        item.set_attribute("data-ondragmove", &moved.0.to_string());
        wire_target(__scope, &item);

        let (node, key) = (item.clone(), self.value.clone());
        let keys = __scope.register_key_handler(move |ev| item_key(&node, &key, ev));
        item.set_attribute("data-onkeydown", &keys.0.to_string());

        adopt(&item, &self.value);

        let id = item.node_id();
        __scope.on_cleanup(move || {
            // The dragged item went away (deleted, or re-rendered as a new
            // node, which `adopt` takes up). The runtime's moves for a pointer
            // drag go to the gone node; its end still comes (`on_drag_end`).
            ACTIVE.with(|a| {
                if let Some(a) = a.borrow_mut().as_mut()
                    && a.source.node_id() == id
                {
                    a.source_gone = true;
                }
            });
        });
        item
    }
}

/// A freshly drawn item `key` of a drag in progress (a re-render mid-drag):
/// dim or ring it as the carried items are, and when it is the lifted item,
/// make it the lift's source, holding the keyboard. And focus it if a drop
/// asked for it.
fn adopt(item: &NodeHandle, key: &str) {
    let doc = item.doc_key();
    let lifted = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut().filter(|a| a.doc == doc)?;
        if !a.keys.iter().any(|k| k == key) {
            return None;
        }
        item.add_class(if a.lift.is_some() {
            LIFTED_CLASS
        } else {
            DRAGGING_CLASS
        });
        a.carried.push(item.clone());
        if a.lift.is_some() && a.source_key == key {
            a.source = item.clone();
            a.source_gone = false;
            Some(a.group.clone())
        } else {
            None
        }
    });
    if let Some(group) = &lifted {
        let entry = focus_leave_entry(item);
        ACTIVE.with(|a| {
            if let Some(lift) = a.borrow_mut().as_mut().and_then(|a| a.lift.as_mut()) {
                lift._keys = Some(entry);
            }
        });
        PENDING_FOCUS.with(|p| *p.borrow_mut() = Some((doc, group.clone(), key.to_string(), None)));
        item.focus();
        rinch_core::unowned(|| set_timeout(0, try_pending_focus));
    }
    let wanted = PENDING_FOCUS.with(|p| {
        p.borrow()
            .as_ref()
            .is_some_and(|(d, _, k, _)| *d == doc && k == key)
    });
    if wanted && lifted.is_none() {
        item.focus();
    }
}

/// The part of a [`SortableItem`] (with `handle: true`) a pointer drags it by:
/// a column's header, a card's grip.
#[derive(Debug, Default)]
pub struct SortableHandle {}

impl Component for SortableHandle {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let handle = __scope.create_element("div");
        handle.set_attribute("class", "rinch-sortable__handle");
        handle.set_attribute("draggable", "true");
        for child in children {
            handle.append_child(child);
        }
        handle
    }
}

/// Whether a sortable drag (pointer or keyboard) is in progress, and its
/// group.
pub fn sortable_drag_group() -> Option<String> {
    active_group().map(|(g, _)| g)
}
