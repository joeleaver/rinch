//! The components: `SortableList`, `SortableItem` and `SortableHandle`.

use super::*;

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
    /// The keyboard drag, off by default (#3062: no browser control and no
    /// WAI-ARIA pattern has one, so it is the app's choice). With it, each
    /// item is a Tab stop (`tabindex="0"`) with an `aria-description` of the
    /// keys, and a focused item's lift key (`lift_keys`) lifts it; the arrows
    /// then move it, within this list and into any list of the group, the
    /// lift key drops it and Escape cancels. Without it the list takes no
    /// key, and its items are not Tab stops unless the app gives them a
    /// `tabindex`.
    pub keyboard: bool,
    /// The keys (`KeyEventData::key`) that lift a focused item and drop it,
    /// in a `keyboard` list: Space and Enter when empty. `vec![" ".into()]`
    /// leaves Enter to the app (an item that opens on Enter).
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
    /// What follows the pointer in a drag from this list's items: a
    /// translucent copy of the item (the default), nothing, the runtime's
    /// snapshot or the app's own element ([`SortPreview`]).
    pub preview: SortPreview,
    /// How a place in this list is shown: a caret (the default), a gap that
    /// opens there, both, or nothing ([`SortIndicator`]).
    pub indicator: SortIndicator,
    /// How this list's items look while a pointer drags them: unchanged (the
    /// default), dimmed, collapsed or hidden ([`SortSource`]).
    pub source: SortSource,
}

impl std::fmt::Debug for SortableList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortableList")
            .field("group", &self.group)
            .field("id", &self.id)
            .field("axis", &self.axis)
            .field("disabled", &self.disabled)
            .field("keyboard", &self.keyboard)
            .field("preview", &self.preview)
            .field("indicator", &self.indicator)
            .field("source", &self.source)
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
            keyboard: false,
            lift_keys: Vec::new(),
            judge: None,
            ondrop: None,
            ondropchange: None,
            drag_keys: None,
            preview: SortPreview::Clone,
            indicator: SortIndicator::Caret,
            source: SortSource::Unchanged,
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

/// The list `seer` has seen the items in `node`'s subtree: at its render,
/// and again for every subtree that arrives later. An item renders before
/// its list, and before the app's attributes on it (its root attributes, a
/// `tabindex` among them) are written, so what depends on its list or on
/// those attributes is done here, where both are known. A nested list's
/// items are that list's.
///
/// In a [`SortableList::keyboard`] list an item is made a keyboard item
/// (`offer_keyboard`). And the item a drop or a lift asked to focus
/// (`PENDING_FOCUS`) takes the focus here when the request names this list
/// (its group, and its id when the request has one), which ends the
/// request: in a list without `keyboard`, the `tabindex` it is lent to take
/// it (`focus_item`) is lent only to an item the app wrote none on, which it
/// cannot tell before now (#3062).
///
/// A `keyboard` list sees every late arrival (`adopt_late_children`); one
/// without sees them only while a request names it (`request_focus`,
/// `watch_if_named`), so an app's plain lists add no insertion observer.
pub(super) fn see_items(node: &NodeHandle, seer: &Seer) {
    if node.get_attribute(LIST_ATTR).is_some() {
        return;
    }
    let Some(key) = node.get_attribute(KEY_ATTR) else {
        for child in node.children() {
            see_items(&child, seer);
        }
        return;
    };
    if seer.keyboard {
        offer_keyboard(node);
    }
    if take_request_for(node.doc_key(), seer, &key) {
        focus_item(node);
    }
}

/// Make the item `node` of a [`SortableList::keyboard`] list a keyboard item:
/// a Tab stop, and the keys in its description when it drags. What the app
/// wrote on the item (a `tabindex`, an `aria-description`, #433) is kept.
fn offer_keyboard(node: &NodeHandle) {
    // A `tabindex` lent to take the focus (`focus_item`) becomes the list's.
    if take_lent(node) || node.get_attribute("tabindex").is_none() {
        node.set_attribute("tabindex", "0");
    }
    if node.get_attribute(DISABLED_ATTR).is_none()
        && node.get_attribute("aria-description").is_none()
    {
        node.set_attribute("aria-description", KEYS_DESCRIPTION);
    }
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
        // The component draws the drag itself unless it asks for the
        // runtime's picture: a host that paints a drag ghost reads this on
        // the source or an ancestor (the desktop runtime does). On the list,
        // not the item: an item is drawn before its list and cannot know.
        if !matches!(self.preview, SortPreview::Snapshot) {
            list.set_attribute("data-drag-ghost", "false");
        }
        for child in children {
            list.append_child(child);
        }
        let seer = Rc::new(Seer {
            group: self.group.clone(),
            id: self.id.clone(),
            keyboard: self.keyboard,
        });
        let doc = list.doc_key();
        if self.keyboard || request_names(doc, &seer) {
            for child in children {
                see_items(child, &seer);
            }
        }
        if self.keyboard {
            let seer = seer.clone();
            crate::late_children::adopt_late_children(
                __scope,
                &list,
                NOT_MINE,
                move |inserted, _| see_items(inserted, &seer),
            );
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
            keyboard: self.keyboard,
            lift_keys: self.lift_keys.clone(),
            judge: self.judge.clone(),
            ondrop: self.ondrop.clone(),
            ondropchange: self.ondropchange.clone(),
            drag_keys: self.drag_keys.clone(),
            preview: self.preview.clone(),
            indicator: self.indicator,
            source: self.source,
            doc: __scope.doc_weak(),
            body: __scope.body_handle(),
            node: list.clone(),
            reason,
            live,
        });
        LISTS.with_mut(|l| l.push((token, doc, cfg)));
        // A list without `keyboard` drawn while a request names it (one
        // re-rendered under a drop) watches for the item until it comes.
        watch_if_named(&list, seer.clone());
        let list_id = list.node_id();
        __scope.on_cleanup(move || {
            // Its watch goes with it, and the request naming it unless a list
            // it names is drawn again (`list_gone`).
            list_gone(doc, list_id, &seer);
            // A drag goes on through a re-render of any list (a co-edited
            // board re-renders under it): lists are found again by id.
            // Dropped once the registry is let go.
            let removed = LISTS.with_mut(|l| {
                let at = l.iter().position(|l| l.0 == token);
                at.map(|at| l.remove(at))
            });
            drop(removed);
        });
        list
    }
}

/// An item of a [`SortableList`], named by `value`.
///
/// The whole item drags, or with `handle` only a [`SortableHandle`] inside it.
/// A press that never passes the drag threshold stays a click. In a
/// [`SortableList::keyboard`] list it is a Tab stop, and focused, a lift key
/// (Space or Enter, [`SortableList::lift_keys`]) lifts it for a keyboard move.
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
        // A keyboard list makes it a Tab stop and describes the keys
        // (`offer_keyboard`): the item cannot know its list yet.
        if self.disabled {
            item.set_attribute(DISABLED_ATTR, "");
        } else {
            item.set_attribute("aria-roledescription", "sortable item");
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
            if !list_of(&node).is_some_and(|c| matches!(c.preview, SortPreview::Snapshot)) {
                rinch_core::suppress_drag_ghost();
            }
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

        let (doc, id) = (item.doc_key(), item.node_id());
        __scope.on_cleanup(move || {
            // A `tabindex` lent to it is forgotten with it (#3062).
            forget_lent(doc, id);
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
/// make it the lift's source, holding the keyboard. A drop's or a lift's
/// item is focused by its list, which sees it after the app's attributes on
/// it are written (`see_items`).
fn adopt(item: &NodeHandle, key: &str) {
    let doc = item.doc_key();
    let lifted = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut().filter(|a| a.doc == doc)?;
        if !a.keys.iter().any(|k| k == key) {
            return None;
        }
        if let Some(class) = carried_class(a.lift.is_some(), a.from.source) {
            item.add_class(class);
        }
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
        // Its list focuses it once it sees it (`see_items`), and the timer
        // if it is already there.
        request_focus(doc, group.clone(), key.to_string(), None);
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
