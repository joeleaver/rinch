//! The registry of drawn lists and the drag in progress.

use super::*;

/// One rendered list.
pub(super) struct ListConfig {
    pub(super) group: String,
    pub(super) id: String,
    pub(super) axis: SortAxis,
    pub(super) label: String,
    pub(super) disabled: bool,
    /// Whether a lift key lifts this list's items ([`SortableList::keyboard`]).
    pub(super) keyboard: bool,
    pub(super) lift_keys: Vec<String>,
    pub(super) judge: Option<SortJudge>,
    pub(super) ondrop: Option<ValueCallback<SortMove>>,
    pub(super) ondropchange: Option<ValueCallback<Option<(SortMove, SortVerdict)>>>,
    pub(super) drag_keys: Option<SortKeys>,
    pub(super) preview: SortPreview,
    pub(super) indicator: SortIndicator,
    pub(super) source: SortSource,
    /// The document and its `<body>`, where a preview is mounted.
    pub(super) doc: std::rc::Weak<RefCell<dyn rinch_core::dom::DomDocument>>,
    pub(super) body: NodeHandle,
    pub(super) node: NodeHandle,
    /// The reason slot for a drop into the list itself (an empty list).
    pub(super) reason: NodeHandle,
    /// The `aria-live` region the keyboard flow announces in.
    pub(super) live: NodeHandle,
}

impl ListConfig {
    pub(super) fn name(&self) -> String {
        if self.label.is_empty() {
            self.id.clone()
        } else {
            self.label.clone()
        }
    }

    /// Whether `ev` is one of this list's lift keys.
    pub(super) fn is_lift_key(&self, ev: &KeyEventData) -> bool {
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
pub(super) enum Mark {
    Before(NodeHandle),
    After(NodeHandle),
    Into(NodeHandle),
}

impl Mark {
    pub(super) fn node(&self) -> &NodeHandle {
        match self {
            Mark::Before(n) | Mark::After(n) | Mark::Into(n) => n,
        }
    }

    pub(super) fn same(&self, other: &Mark) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.node().node_id() == other.node().node_id()
    }
}

pub(super) struct Offer {
    /// The target list, by id (a re-render issues it a new token).
    pub(super) list_id: String,
    pub(super) slot: usize,
    pub(super) mv: SortMove,
    pub(super) verdict: SortVerdict,
    pub(super) mark: Mark,
    pub(super) reason: Option<NodeHandle>,
    pub(super) horizontal: bool,
    /// Whether the caret (or the refused caret) is drawn: the target list's
    /// indicator.
    pub(super) caret: bool,
}

/// A keyboard lift: the list (by id) and slot the caret is at.
pub(super) struct Lift {
    pub(super) list_id: String,
    pub(super) slot: usize,
    pub(super) _keys: Option<DismissHandle>,
}

/// The drag in progress (one per thread: one pointer, one focus).
pub(super) struct Active {
    pub(super) group: String,
    pub(super) keys: Vec<String>,
    /// The source list, held: a re-render may unregister it mid-drag, and its
    /// `ondropchange` still hears the drag out.
    pub(super) from: Rc<ListConfig>,
    pub(super) from_list: String,
    pub(super) doc: u64,
    pub(super) source: NodeHandle,
    pub(super) source_key: String,
    pub(super) carried: Vec<NodeHandle>,
    pub(super) offer: Option<Offer>,
    /// Whether the source list's `ondropchange` last heard a move.
    pub(super) told: bool,
    /// A target was left; unless the same move reaches another, the offer is
    /// withdrawn at the source's drag move.
    pub(super) pending_none: bool,
    pub(super) lift: Option<Lift>,
    /// The source item's node went away mid-drag (the app re-rendered it):
    /// the runtime's drag moves go to the gone node, so a left target
    /// withdraws at once. The end is the runtime's ([`on_drag_end`]).
    pub(super) source_gone: bool,
    /// The place under the pointer or the lift's, no-op places included.
    pub(super) over: Option<(String, usize)>,
    /// A target heard this move (`data-ondragover`): a move with none is over
    /// no list.
    pub(super) hovered: bool,
    /// The place withdrawn by the last leave, until the next place: a leave
    /// onto the source drawn at that place (a re-rendered source withdraws at
    /// once, #2093) takes it back.
    pub(super) left_over: Option<(String, usize)>,
    /// The carried nodes drawn away from home when the last place was
    /// withdrawn: the only ones a leave's place can be taken back from.
    pub(super) left_moved: Vec<rinch_core::dom::NodeId>,
    /// The carried items' extent along x and y, spacing included: a gap's size.
    pub(super) extent: (f32, f32),
    /// The pointer drag's grab point in the source item and its size.
    pub(super) grab: Option<SortPointer>,
    /// The preview following the pointer, and the scope it was built in.
    pub(super) ghost: Option<(NodeHandle, Option<RenderScope>)>,
    /// How far the preview's body sits from the item's painted corner: half
    /// the difference of its layout and painted sizes (a scaled ancestor).
    pub(super) ghost_fix: (f32, f32),
    /// The spacing between the source list's items (its own units).
    pub(super) spacing: f32,
    /// Items carrying a shift, and lists carrying the reflow class.
    pub(super) shifted: Vec<(NodeHandle, f32, f32)>,
    pub(super) reflowing: Vec<NodeHandle>,
}

thread_local! {
    pub(super) static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    pub(super) static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    pub(super) static DRAG_END_HOOKED: Cell<bool> = const { Cell::new(false) };
}

fn list_config(token: u64) -> Option<Rc<ListConfig>> {
    LISTS.with(|l| l.iter().find(|l| l.0 == token).map(|l| l.2.clone()))
}

fn token_of(node: &NodeHandle) -> Option<u64> {
    node.get_attribute(LIST_ATTR)?.parse().ok()
}

/// The nearest list above `node` (an item's own list).
pub(super) fn list_of(node: &NodeHandle) -> Option<Rc<ListConfig>> {
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
pub(super) fn list_by_id(group: &str, id: &str, doc: u64) -> Option<Rc<ListConfig>> {
    LISTS.with(|l| {
        l.iter()
            .map(|l| &l.2)
            .find(|c| c.group == group && c.id == id && c.node.doc_key() == doc)
            .cloned()
    })
}

/// A list's own items in order: the descendants carrying a key, not looking
/// inside an item or a nested list.
pub(super) fn items_of(list: &NodeHandle) -> Vec<(NodeHandle, String)> {
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
pub(super) fn lists_of_group(group: &str, doc: u64) -> Vec<Rc<ListConfig>> {
    let lists: Vec<Rc<ListConfig>> = LISTS.with(|l| {
        l.iter()
            .map(|l| &l.2)
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
pub(super) fn inside_carried(node: &NodeHandle, group: &str, keys: &[String]) -> bool {
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
pub(super) fn resolve_target(node: &NodeHandle, group: &str, keys: &[String]) -> Option<Target> {
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
