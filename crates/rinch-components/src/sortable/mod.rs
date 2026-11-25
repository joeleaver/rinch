//! Sortable lists (#2091): drag items within and between ordered lists.
//!
//! [`SortableList`] is a list of one *group* (cards, columns, lanes…) and
//! [`SortableItem`] an item in it, named by a key. A drag — by pointer, or, in
//! a list with [`SortableList::keyboard`], by keyboard (Space/Enter to lift —
//! [`SortableList::lift_keys`] —, arrows to move, the lift key again to drop,
//! Escape to cancel) — only ever targets
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
use std::rc::Rc;

use rinch_core::Component;
use rinch_core::DismissHandle;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ValueCallback;
use rinch_core::events::KeyEventData;
use rinch_core::get_click_context;
use rinch_core::reactive::ElementBounds;
use rinch_core::reactive::Signal;
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
const HIDDEN_CLASS: &str = "rinch-sortable__item--drag-hidden";
const COLLAPSED_CLASS: &str = "rinch-sortable__item--collapsed";
const SHIFTED_CLASS: &str = "rinch-sortable__item--shifted";
const REFLOWING_CLASS: &str = "rinch-sortable--reflowing";
const GHOST_CLASS: &str = "rinch-sortable__ghost";
const GHOST_ATTR: &str = "data-sortable-ghost";
const SHIFT_X: &str = "--rinch-sortable-shift-x";
const SHIFT_Y: &str = "--rinch-sortable-shift-y";
const GHOST_X: &str = "--rinch-sortable-ghost-x";
const GHOST_Y: &str = "--rinch-sortable-ghost-y";
/// The keys that lift and drop when a list names none.
const DEFAULT_LIFT_KEYS: [&str; 2] = [" ", "Enter"];
/// The `aria-description` of an item a keyboard list's keys can lift.
const KEYS_DESCRIPTION: &str =
    "Press Space to lift, the arrow keys to move, Space to drop, Escape to cancel.";
/// What a [`SortableList::keyboard`] list's walk stops descending at: its
/// items, and nested lists (whose items are theirs).
const NOT_MINE: &[&str] = &[ITEM_CLASS, "rinch-sortable"];
/// Inline style that keeps the live region off screen without the stylesheet.
const LIVE_STYLE: &str = "position: absolute; width: 1px; height: 1px; margin: -1px; \
     padding: 0; border: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap";

mod component;
mod drag_state;
mod focus;
#[cfg(test)]
mod focus_tests;
mod ghost;
mod keyboard;
mod offer;
mod pointer;
mod reflow;
mod registry;
mod types;

pub use component::{SortableHandle, SortableItem, SortableList, sortable_drag_group};
pub use drag_state::{sortable_drag, sortable_drag_pointer};
pub use types::{
    SortAxis, SortDragState, SortIndicator, SortJudge, SortKeys, SortMove, SortPlace, SortPointer,
    SortPreview, SortPreviewFn, SortSource, SortTarget, SortVerdict,
};

use drag_state::*;
use focus::*;
use ghost::*;
use keyboard::*;
use offer::*;
use pointer::*;
use reflow::*;
use registry::*;

// The document-keyed statics live here, not in the modules that use them:
// a registry's name is its module path (`rinch_components::sortable::LISTS`),
// which the document-sweep tests name.
rinch_core::doc_keyed! {
    /// Every drawn list, as `(token, its document, config)`. Swept with its
    /// document (#2797): a list rendered where no scope will unregister it
    /// goes when the document dies.
    static LISTS: rinch_core::DocList<(u64, u64, Rc<ListConfig>)> =
        rinch_core::DocList::new(|l| Some(l.1));
    /// The item to focus once it is drawn, and the lists watching for it
    /// (`focus::request_focus`). Swept with its document (#2797).
    static PENDING_FOCUS: FocusSlot = FocusSlot(None);
    /// The items rinch made focusable to give them the focus (`focus_item`),
    /// as `(their document, the item, the key entry that takes the
    /// `tabindex` back when the focus leaves)`. Swept with its document.
    static LENT_TABINDEX: rinch_core::DocList<(u64, NodeHandle, DismissHandle)> =
        rinch_core::DocList::new(|l| Some(l.0));
}

rinch_core::doc_keyed! {
    /// Each document's preview wrapper, kept detached between drags: one
    /// node per document rather than one per drag. Swept with its document
    /// (#2797).
    static GHOST_WRAPPERS: rinch_core::DocMap<NodeHandle> = rinch_core::DocMap::new();
}

/// How many lists are registered on this thread, whatever their document:
/// what a test asserts stays flat as windows open and close (#2797). Not part
/// of the API.
#[doc(hidden)]
pub fn __registered_count() -> usize {
    LISTS.with(|l| l.len())
}
