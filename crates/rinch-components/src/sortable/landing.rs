//! Where a keyboard drop put its items, and when the list shows that it has
//! been applied (#3062 reviews 4 and 5).
//!
//! An app may apply a drop later (after an await, or a timer of its own).
//! Until then the item's node, or a new node the app draws for it at its old
//! place, is not the moved item, and taking it for the moved item would end
//! the focus request: the redraw that applies the drop would then send the
//! focus to the document. So a drop's request records the list's key order at
//! the drop and the order the drop asks for, and a node of the item's key
//! satisfies it only once its list's order shows the drop applied.
//!
//! The rule (`Order`), comparing only the keys the list and the order compared
//! with have in common, so that an item the app adds or removes beside the
//! move changes nothing:
//! - **applied**: the list is in the order the drop asks for. A node of the
//!   key takes the focus.
//! - **untouched**: the list is still in its order at the drop, with no
//!   carried item arrived from elsewhere. Not yet: the request waits. A redraw
//!   for something else (a "saving" flag drawing the rows again by position)
//!   leaves the order as it was, and does not end it.
//! - **other**: neither. The app applied the drop its own way (it adjusted the
//!   place, or sorted the list), so the request ends on the item, wherever it
//!   is, rather than wait for an order that will never come.
//!
//! An app that never applies the drop (it refused it) leaves the request
//! waiting; it ends at the next drag, when the focus moves on from the lifted
//! item while that is still drawn, or when no list it names is left
//! (`super::focus`).

use super::*;
use rinch_core::dom::NodeId;

/// The list a keyboard drop names, at the drop and once the drop is applied.
pub(super) struct Landing {
    /// The list's keys in order at the drop.
    before: Vec<String>,
    /// The list's keys in the order the drop asks for: `before` without the
    /// carried keys, with all of them put back, in the order carried, at the
    /// drop's index (`SortPlace::index`).
    after: Vec<String>,
    /// The keys carried (`SortMove::keys`).
    carried: Vec<String>,
    /// The node the focused item was in that list at the drop (a drop within
    /// one list), if it was there.
    held: Option<NodeId>,
}

impl Landing {
    /// The landing of the move `mv` into the list `list`, read before the
    /// app's handler can redraw it; `first` is the key the focus follows.
    pub(super) fn at_drop(list: &NodeHandle, mv: &SortMove, first: &str) -> Landing {
        let items = items_of(list);
        let held = items
            .iter()
            .find(|(_, k)| k == first)
            .map(|(n, _)| n.node_id());
        let before: Vec<String> = items.into_iter().map(|(_, k)| k).collect();
        Landing {
            after: moved(&before, &mv.keys, mv.place.index),
            before,
            carried: mv.keys.clone(),
            held,
        }
    }

    /// What the list's order `keys` says of the drop.
    fn order(&self, keys: &[String]) -> Order {
        if same_order(keys, &self.after) {
            Order::Applied
        } else if same_order(keys, &self.before)
            && keys
                .iter()
                .all(|k| !self.carried.contains(k) || self.before.contains(k))
        {
            Order::Untouched
        } else {
            Order::Other
        }
    }
}

/// `before` with the `carried` keys taken out and put back at `index`.
fn moved(before: &[String], carried: &[String], index: usize) -> Vec<String> {
    let mut out: Vec<String> = before
        .iter()
        .filter(|k| !carried.contains(k))
        .cloned()
        .collect();
    let at = index.min(out.len());
    out.splice(at..at, carried.iter().cloned());
    out
}

/// Whether `keys` and `reference` agree on the order of the keys they have
/// in common. A key drawn twice (mid-redraw, the old row and the new) never
/// agrees.
fn same_order(keys: &[String], reference: &[String]) -> bool {
    let ours = keys.iter().filter(|k| reference.contains(k));
    let theirs = reference.iter().filter(|k| keys.contains(k));
    ours.eq(theirs)
}

/// What a list's order says of a drop (see the module's rule).
enum Order {
    Applied,
    Untouched,
    Other,
}

/// What a request makes of an item of its key in a list it names.
pub(super) enum Claim {
    /// The item it is for: focus it.
    Take,
    /// Not decided until the tree has settled: read again then
    /// (`schedule_check`).
    Later,
    /// Not yet the item it is for.
    No,
}

/// What a request with `landing` makes of `node`, an item of its key in a
/// list it names, whose keys in order are `keys`: see the module's rule.
/// Mid-reconcile (not `settled`) only a new node in the applied order is
/// taken; the rest is read once the tree has `settled` (a scheduled check):
/// a list redrawn row by row passes through orders that are neither, and
/// can pass the old node through the drop's place on its way out.
pub(super) fn claim(
    landing: Option<&Landing>,
    node: &NodeHandle,
    keys: &[String],
    settled: bool,
) -> Claim {
    let Some(landing) = landing else {
        return Claim::Take;
    };
    match landing.order(keys) {
        Order::Applied if settled || landing.held != Some(node.node_id()) => Claim::Take,
        _ if !settled => Claim::Later,
        Order::Untouched => Claim::No,
        Order::Applied | Order::Other => Claim::Take,
    }
}

/// The keys, in order, of the list `item` is in.
pub(super) fn keys_of_list_of(item: &NodeHandle) -> Vec<String> {
    let mut at = item.parent_node();
    while let Some(node) = at {
        if node.get_attribute(LIST_ATTR).is_some() {
            return keys_of(&node);
        }
        at = node.parent_node();
    }
    Vec::new()
}

/// The keys, in order, of the list `list`.
pub(super) fn keys_of(list: &NodeHandle) -> Vec<String> {
    items_of(list).into_iter().map(|(_, k)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn landing(before: &str, carried: &str, index: usize) -> Landing {
        let before = keys(before);
        let carried = keys(carried);
        Landing {
            after: moved(&before, &carried, index),
            before,
            carried,
            held: None,
        }
    }

    /// [a b c d], a and c carried to slot 0: [a c b d], though a is at 0
    /// already (#3071 review 5).
    #[test]
    fn a_multi_item_move_asks_for_every_carried_key_in_place() {
        let l = landing("a b c d", "a c", 0);
        assert_eq!(l.after, keys("a c b d"));
        assert!(matches!(l.order(&keys("a b c d")), Order::Untouched));
        assert!(matches!(l.order(&keys("a c b d")), Order::Applied));
    }

    #[test]
    fn an_item_added_or_removed_beside_the_move_changes_nothing() {
        let l = landing("a b c d", "a", 2);
        assert_eq!(l.after, keys("b c a d"));
        assert!(matches!(l.order(&keys("a b x c d")), Order::Untouched));
        assert!(matches!(l.order(&keys("b c x a")), Order::Applied));
    }

    #[test]
    fn another_order_is_the_drop_applied_another_way() {
        let l = landing("a b c d", "a", 2);
        assert!(matches!(l.order(&keys("b a c d")), Order::Other));
        assert!(matches!(l.order(&keys("a a b c d")), Order::Other));
    }

    /// Into another list: the carried key arriving anywhere else than where
    /// the drop put it is the drop applied its own way.
    #[test]
    fn a_carried_key_arriving_elsewhere_is_another_order() {
        let l = landing("x y", "a", 1);
        assert_eq!(l.after, keys("x a y"));
        assert!(matches!(l.order(&keys("x a y")), Order::Applied));
        assert!(matches!(l.order(&keys("a x y")), Order::Other));
    }
}
