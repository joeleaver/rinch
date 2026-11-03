//! Sortable lists (#2091): drag items within and between ordered lists.
//!
//! [`SortableList`] is a list of one *group* (cards, columns, lanes…) and
//! [`SortableItem`] an item in it, named by a key. A drag — by pointer, or by
//! keyboard (Space/Enter to lift, arrows to move, Space/Enter to drop, Escape
//! to cancel) — only ever targets lists of its own group, so lists nest without
//! fighting: a card dragged over a column's header offers nothing, and a column
//! dragged over its own cards is placed among the columns.
//!
//! The component never reorders anything. While a drag is on offer it draws an
//! insertion caret, asks the target list's `judge`, and shows a refusal's
//! reason beside the caret; on an accepted drop it calls the target list's
//! `ondrop` with a key-relative [`SortPlace`], and the app moves its own data.
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
    /// list's `drag_keys` named for it.
    pub keys: Vec<String>,
    /// The `id` of the list the drag started in.
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
    /// It may not; the reason is shown beside the caret while the drag is
    /// over the place (an empty reason shows the refused caret alone).
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
    token: u64,
    slot: usize,
    mv: SortMove,
    verdict: SortVerdict,
    mark: Mark,
    reason: Option<NodeHandle>,
    horizontal: bool,
}

/// A keyboard lift: the list and slot the caret is at.
struct Lift {
    token: u64,
    slot: usize,
    _keys: Option<DismissHandle>,
}

/// The drag in progress (one per thread: one pointer, one focus).
struct Active {
    group: String,
    keys: Vec<String>,
    from_token: u64,
    /// The source list, held: a re-render may unregister it mid-drag, and its
    /// `ondropchange` still hears the drag out.
    from: Rc<ListConfig>,
    from_list: String,
    source: NodeHandle,
    carried: Vec<NodeHandle>,
    offer: Option<Offer>,
    /// Whether the source list's `ondropchange` last heard a move.
    told: bool,
    /// A target was left; unless the same move reaches another, the offer is
    /// withdrawn at the source's drag move.
    pending_none: bool,
    lift: Option<Lift>,
    /// The source item's node went away mid-drag (the app re-rendered it):
    /// the runtime's drag moves and end go to the gone node, so a left target
    /// withdraws at once, and nothing waits on the end.
    source_gone: bool,
}

thread_local! {
    static LISTS: RefCell<HashMap<u64, Rc<ListConfig>>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

fn list_config(token: u64) -> Option<Rc<ListConfig>> {
    LISTS.with(|l| l.borrow().get(&token).cloned())
}

fn token_of(node: &NodeHandle) -> Option<u64> {
    node.get_attribute(LIST_ATTR)?.parse().ok()
}

/// The nearest list at or above `node` (an item's own list).
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

fn token_of_config(cfg: &ListConfig) -> u64 {
    token_of(&cfg.node).unwrap_or(0)
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

/// Every registered list of `group` in `doc`, in registration order.
fn lists_of_group(group: &str, doc: u64) -> Vec<(u64, Rc<ListConfig>)> {
    let mut lists: Vec<(u64, Rc<ListConfig>)> = LISTS.with(|l| {
        l.borrow()
            .iter()
            .filter(|(_, c)| c.group == group && c.node.doc_key() == doc && c.node.is_valid())
            .map(|(t, c)| (*t, c.clone()))
            .collect()
    });
    lists.sort_by_key(|(t, _)| *t);
    lists
}

/// The list of the dragged group a drag over `node` targets, and the item of
/// that list under the pointer, if any: walking up, an item is remembered, a
/// list of another group forgets it, and the first list of `group` answers.
fn resolve_target(node: &NodeHandle, group: &str) -> Option<(u64, Option<(NodeHandle, String)>)> {
    let mut item: Option<(NodeHandle, String)> = None;
    let mut at = Some(node.clone());
    while let Some(n) = at {
        if let Some(key) = n.get_attribute(KEY_ATTR) {
            item = Some((n.clone(), key));
        } else if let Some(token) = token_of(&n) {
            if list_config(token).is_some_and(|c| c.group == group) {
                return Some((token, item));
            }
            item = None;
        }
        at = n.parent_node();
    }
    None
}

// =============================================================================
// Places, judging and drawing
// =============================================================================

/// The list's items with the carried ones left out.
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

/// Make `slot` of `token`'s list the place on offer for the active drag
/// (`None` withdraws it), judging it and drawing the caret.
fn offer_slot(token: u64, slot: Option<usize>) {
    let Some(cfg) = list_config(token) else {
        set_offer(None);
        return;
    };
    let Some(slot) = slot else {
        set_offer(None);
        return;
    };
    let Some((group, keys, from_list)) = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.group.clone(), a.keys.clone(), a.from_list.clone()))
    }) else {
        return;
    };
    let (rest, all) = rest_of(&cfg.node, &keys);
    let slot = slot.min(rest.len());
    if is_no_op(&all, &rest, &keys, slot) {
        set_offer(None);
        return;
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
    // The same place drawn on the same node: nothing to judge or redraw. (A
    // re-render that replaced the node draws it again.)
    let unchanged = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|a| a.offer.as_ref())
            .is_some_and(|o| o.token == token && o.slot == slot && o.mark.same(&mark))
    });
    if unchanged {
        return;
    }
    let place = SortPlace {
        after: slot.checked_sub(1).map(|i| rest[i].1.clone()),
        before: rest.get(slot).map(|(_, k)| k.clone()),
        index: slot,
    };
    let mv = SortMove {
        group,
        keys,
        from_list,
        to_list: cfg.id.clone(),
        place,
    };
    let verdict = cfg.judge.as_ref().map_or(SortVerdict::Accept, |judge| {
        rinch_core::untracked_handler(|| judge(&mv))
    });
    set_offer(Some(Offer {
        token,
        slot,
        mv,
        verdict,
        mark,
        reason: Some(reason),
        horizontal: cfg.axis == SortAxis::Horizontal,
    }));
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

/// Mark the carried items (in any list of the group) and remember them.
fn carried_nodes(group: &str, doc: u64, keys: &[String]) -> Vec<NodeHandle> {
    let mut out = Vec::new();
    for (_, cfg) in lists_of_group(group, doc) {
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

/// Start a drag of `item` (pointer or keyboard).
fn begin(item: &NodeHandle, key: &str, keyboard: bool) -> bool {
    end_active();
    let Some(cfg) = list_of(item) else {
        return false;
    };
    let keys = keys_for(&cfg, key);
    let carried = carried_nodes(&cfg.group, item.doc_key(), &keys);
    for node in &carried {
        node.add_class(if keyboard {
            LIFTED_CLASS
        } else {
            DRAGGING_CLASS
        });
    }
    let from_token = token_of_config(&cfg);
    ACTIVE.with(|a| {
        *a.borrow_mut() = Some(Active {
            group: cfg.group.clone(),
            keys,
            from_token,
            from: cfg.clone(),
            from_list: cfg.id.clone(),
            source: item.clone(),
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

// =============================================================================
// Pointer handlers
// =============================================================================

/// A drag moved over `target` (an item or a list): offer the place under the
/// pointer.
fn drag_over(target: &NodeHandle) {
    let Some((group, false)) = active_group() else {
        return;
    };
    let Some((token, item)) = resolve_target(target, &group) else {
        set_offer(None);
        return;
    };
    let Some(cfg) = list_config(token) else {
        return;
    };
    let keys = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| a.keys.clone())
            .unwrap_or_default()
    });
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
            for (_, k) in &all {
                if *k == key {
                    break;
                }
                if !keys.contains(k) {
                    slot += 1;
                }
            }
            if !keys.contains(&key) {
                let past_middle = bounds_of(&node).is_some_and(|b| {
                    let (start, len) = span(&b, cfg.axis);
                    pointer >= start + len / 2.0
                });
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
    offer_slot(token, Some(slot));
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

/// The release: deliver an accepted offer to its list, then end.
fn drop_here() {
    let delivered = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        if a.lift.is_some() {
            return None;
        }
        let o = a.offer.as_ref()?;
        o.verdict.is_accept().then(|| (o.token, o.mv.clone()))
    });
    end_active();
    if let Some((token, mv)) = delivered
        && let Some(cb) = list_config(token).and_then(|c| c.ondrop.clone())
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
/// boxes; without boxes, the previous or next in registration order.
fn neighbour(from: u64, dir: (i32, i32), group: &str, doc: u64) -> Option<u64> {
    let lists = lists_of_group(group, doc);
    let here = lists
        .iter()
        .find(|(t, _)| *t == from)?
        .1
        .node
        .bounding_rect();
    let Some(here) = here else {
        let at = lists.iter().position(|(t, _)| *t == from)?;
        let forward = dir.0 + dir.1 > 0;
        return if forward {
            lists.get(at + 1).map(|(t, _)| *t)
        } else {
            at.checked_sub(1).map(|i| lists[i].0)
        };
    };
    let (hx, hy) = center(&here);
    let mut best: Option<(f32, u64)> = None;
    for (token, cfg) in &lists {
        if *token == from {
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
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, *token));
        }
    }
    best.map(|(_, t)| t)
}

/// Lift `item` for a keyboard move.
fn lift(item: &NodeHandle, key: &str) {
    if !begin(item, key, true) {
        return;
    }
    let Some(cfg) = list_of(item) else {
        return;
    };
    let token = token_of_config(&cfg);
    let keys = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| a.keys.clone())
            .unwrap_or_default()
    });
    let all = items_of(&cfg.node);
    let home = all
        .iter()
        .take_while(|(_, k)| *k != key)
        .filter(|(_, k)| !keys.contains(k))
        .count();
    // Focus leaving the lifted item cancels the lift.
    let entry = rinch_core::push_key_handler(item, |_| false, || cancel_lift(true));
    ACTIVE.with(|a| {
        if let Some(a) = a.borrow_mut().as_mut() {
            a.lift = Some(Lift {
                token,
                slot: home,
                _keys: Some(entry),
            });
        }
    });
    let label = label_of(item, key);
    announce(
        &cfg,
        &format!(
            "Picked up {label}. Use the arrow keys to move it, Space or Enter to drop it, \
             Escape to cancel."
        ),
    );
}

fn label_of(item: &NodeHandle, key: &str) -> String {
    item.get_attribute("data-sortable-label")
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| key.to_string())
}

fn announce(cfg: &ListConfig, text: &str) {
    cfg.live.set_text(text);
}

/// The source list's config (announcements go there).
fn source_config() -> Option<Rc<ListConfig>> {
    ACTIVE.with(|a| a.borrow().as_ref().map(|a| a.from.clone()))
}

/// Move a keyboard lift one step in `dir`.
fn move_lift(dir: (i32, i32)) {
    let Some((token, slot, group, keys, doc)) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        let lift = a.lift.as_ref()?;
        Some((
            lift.token,
            lift.slot,
            a.group.clone(),
            a.keys.clone(),
            a.source.doc_key(),
        ))
    }) else {
        return;
    };
    let Some(cfg) = list_config(token) else {
        return;
    };
    let len = rest_of(&cfg.node, &keys).0.len();
    let (forward, along) = match cfg.axis {
        SortAxis::Vertical => (dir.1 > 0, dir.0 == 0),
        SortAxis::Horizontal => (dir.0 > 0, dir.1 == 0),
    };
    let (token, slot) = if along && forward && slot < len {
        (token, slot + 1)
    } else if along && !forward && slot > 0 {
        (token, slot - 1)
    } else if let Some(next) = neighbour(token, dir, &group, doc) {
        let next_len = list_config(next).map_or(0, |c| rest_of(&c.node, &keys).0.len());
        let slot = if along {
            if forward { 0 } else { next_len }
        } else {
            slot.min(next_len)
        };
        (next, slot)
    } else {
        (token, slot)
    };
    ACTIVE.with(|a| {
        if let Some(lift) = a.borrow_mut().as_mut().and_then(|a| a.lift.as_mut()) {
            lift.token = token;
            lift.slot = slot;
        }
    });
    offer_slot(token, Some(slot));
    describe_lift(token, slot);
}

/// Scroll the caret into view and announce where the lift is.
fn describe_lift(token: u64, slot: usize) {
    let Some(cfg) = list_config(token) else {
        return;
    };
    let (label, verdict, mark) = ACTIVE.with(|a| {
        let a = a.borrow();
        let Some(a) = a.as_ref() else {
            return (String::new(), None, None);
        };
        let label = label_of(&a.source, a.keys.first().map_or("", |k| k.as_str()));
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
    if let Some(Mark::Before(n) | Mark::After(n) | Mark::Into(n)) = &mark {
        n.scroll_into_view();
    }
    let list = if cfg.label.is_empty() {
        cfg.id.clone()
    } else {
        cfg.label.clone()
    };
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
    if let Some(source) = source_config() {
        announce(&source, &text);
    }
}

/// Drop a keyboard lift where its caret is.
fn drop_lift() {
    let Some((delivered, source_node, first)) = ACTIVE.with(|a| {
        let a = a.borrow();
        let a = a.as_ref()?;
        a.lift.as_ref()?;
        let delivered = a
            .offer
            .as_ref()
            .filter(|o| o.verdict.is_accept())
            .map(|o| (o.token, o.mv.clone()));
        Some((delivered, a.source.clone(), a.keys.first().cloned()))
    }) else {
        return;
    };
    let source = source_config();
    end_active();
    let Some((token, mv)) = delivered else {
        if let Some(source) = &source {
            announce(source, "Not moved.");
        }
        return;
    };
    let group = mv.group.clone();
    if let Some(source) = &source {
        let list = list_config(token)
            .map(|c| {
                if c.label.is_empty() {
                    c.id.clone()
                } else {
                    c.label.clone()
                }
            })
            .unwrap_or_default();
        announce(
            source,
            &format!("Dropped in {list}, position {}.", mv.place.index + 1),
        );
    }
    if let Some(cb) = list_config(token).and_then(|c| c.ondrop.clone()) {
        cb.invoke(mv);
    }
    // The app re-renders from its data once this handler's batch flushes; the
    // moved item (a new node, when it changed lists) takes the focus back.
    let Some(first) = first else { return };
    let doc = source_node.doc_key();
    // Unowned: the item that parked it is usually the node the re-render
    // replaced, and a parked callback of a disposed owner is dropped.
    rinch_core::unowned(|| {
        set_timeout(0, move || {
            for (_, cfg) in lists_of_group(&group, doc) {
                if let Some((node, _)) = items_of(&cfg.node).into_iter().find(|(_, k)| *k == first)
                {
                    node.focus();
                    return;
                }
            }
        })
    });
}

/// Cancel a keyboard lift (`focus_left`: the focus moved away).
fn cancel_lift(focus_left: bool) {
    let lifted = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| a.lift.is_some()));
    if !lifted {
        return;
    }
    let source = source_config();
    end_active();
    if let Some(source) = source
        && !focus_left
    {
        announce(&source, "Cancelled. Not moved.");
    }
}

/// An item's `onkeydown`.
fn item_key(item: &NodeHandle, key: &str, ev: &KeyEventData) -> bool {
    if ev.kind != rinch_core::events::KeyEventKind::Down {
        return false;
    }
    // Only for the item itself: a key in a field or a button inside it is
    // that control's.
    if item
        .active_element()
        .is_some_and(|f| f.node_id() != item.node_id())
    {
        return false;
    }
    let lifted_here = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.lift.is_some() && a.source.node_id() == item.node_id())
    });
    let modified = ev.ctrl || ev.alt || ev.meta;
    let activate = (ev.is_space() || ev.key == "Enter") && !modified;
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
    if activate && !pointer_drag && !ev.shift {
        lift(item, key);
        return true;
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
/// other groups — nested inside its items, or around it — are never targeted.
/// Lays its children out as a flex row or column (`axis`), with
/// `--rinch-sortable-gap` between them; `class:` and `style:` override it.
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
    /// Whether a move into this list may happen, asked for every place on
    /// offer, live; a refusal's reason is shown by the caret. Accepted when
    /// not given.
    pub judge: Option<SortJudge>,
    /// An accepted drop into this list: apply it to the app's data.
    pub ondrop: Option<ValueCallback<SortMove>>,
    /// For a drag that started in this list: the move on offer and its
    /// verdict each time it changes, `None` when nothing is on offer (and when
    /// the drag ends).
    pub ondropchange: Option<ValueCallback<Option<(SortMove, SortVerdict)>>>,
    /// What a drag from an item of this list carries (the item alone when not
    /// given, or when the answer leaves the item out): a selection moved
    /// together, in order.
    pub drag_keys: Option<SortKeys>,
}

impl std::fmt::Debug for SortableList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortableList")
            .field("group", &self.group)
            .field("id", &self.id)
            .field("axis", &self.axis)
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

impl Component for SortableList {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let list = __scope.create_element("div");
        list.set_attribute(
            "class",
            match self.axis {
                SortAxis::Vertical => "rinch-sortable rinch-sortable--vertical",
                SortAxis::Horizontal => "rinch-sortable rinch-sortable--horizontal",
            },
        );
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
        let reason = __scope.create_element("div");
        reason.set_attribute("class", REASON_CLASS);
        reason.set_attribute("aria-hidden", "true");
        list.append_child(&reason);
        let live = __scope.create_element("div");
        live.set_attribute("class", "rinch-sortable__live");
        live.set_attribute("aria-live", "assertive");
        live.set_attribute("role", "status");
        list.append_child(&live);

        wire_target(__scope, &list);
        let cfg = Rc::new(ListConfig {
            group: self.group.clone(),
            id: self.id.clone(),
            axis: self.axis,
            label: self.label.clone(),
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
            LISTS.with(|l| l.borrow_mut().remove(&token));
            // A lift whose list went away ends with it; a pointer drag goes on
            // (a co-edited board re-renders under it) and drops by the
            // target's handlers.
            let orphaned = ACTIVE.with(|a| {
                a.borrow()
                    .as_ref()
                    .is_some_and(|a| a.from_token == token && a.lift.is_some())
            });
            if orphaned {
                end_active();
            }
        });
        list
    }
}

/// An item of a [`SortableList`], named by `value`.
///
/// The whole item drags, or with `handle` only a [`SortableHandle`] inside it.
/// A press that never passes the drag threshold stays a click. Focused, Space
/// or Enter lifts it for a keyboard move.
#[derive(Debug, Default)]
pub struct SortableItem {
    /// The item's key, as `SortMove::keys` and `SortPlace` name it.
    pub value: String,
    /// Drag only by a [`SortableHandle`] inside the item.
    pub handle: bool,
    /// The item's name in keyboard announcements (its key when empty).
    pub label: String,
}

impl Component for SortableItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let item = __scope.create_element("div");
        item.set_attribute("class", ITEM_CLASS);
        item.set_attribute(KEY_ATTR, &self.value);
        if !self.label.is_empty() {
            item.set_attribute("data-sortable-label", &self.label);
        }
        item.set_attribute("tabindex", "0");
        if !self.handle {
            item.set_attribute("draggable", "true");
            item.add_class("rinch-sortable__item--grab");
        }
        for child in children {
            item.append_child(child);
        }
        let reason = __scope.create_element("div");
        reason.set_attribute("class", REASON_CLASS);
        reason.set_attribute("aria-hidden", "true");
        item.append_child(&reason);

        let (node, key) = (item.clone(), self.value.clone());
        let start = __scope.register_handler(move || {
            rinch_core::suppress_drag_ghost();
            begin(&node, &key, false);
        });
        item.set_attribute("data-ondragstart", &start.0.to_string());
        let end = __scope.register_handler(end_active);
        item.set_attribute("data-ondragend", &end.0.to_string());
        let moved = __scope.register_handler(drag_move);
        item.set_attribute("data-ondragmove", &moved.0.to_string());
        wire_target(__scope, &item);

        let (node, key) = (item.clone(), self.value.clone());
        let keys = __scope.register_key_handler(move |ev| item_key(&node, &key, ev));
        item.set_attribute("data-onkeydown", &keys.0.to_string());

        let id = item.node_id();
        __scope.on_cleanup(move || {
            // The dragged item went away (deleted, or re-rendered as a new
            // node). A lift ends: nothing is left holding the focus. A pointer
            // drag goes on (a co-edited list re-renders under it), knowing its
            // source's moves and end will not come.
            let lifted = ACTIVE.with(|a| {
                let mut a = a.borrow_mut();
                let Some(a) = a.as_mut().filter(|a| a.source.node_id() == id) else {
                    return false;
                };
                a.source_gone = true;
                a.lift.is_some()
            });
            if lifted {
                end_active();
            }
        });
        item
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
