//! Giving the focus back to an item after a keyboard drop or a re-render
//! during a lift: the pending request (`PENDING_FOCUS`), the lists that watch
//! for its item, and the `tabindex` lent to an item that has none (#3062).

use super::component::see_items;
use super::*;
use rinch_core::dom::NodeId;
use rinch_core::reactive::Scope;

/// The item to focus once it is drawn: `key` of `group` in document `doc`,
/// in the list `list` (any list of the group when `None`).
pub(super) struct FocusRequest {
    pub(super) doc: u64,
    pub(super) group: String,
    pub(super) key: String,
    pub(super) list: Option<String>,
    /// Which request this is: a check scheduled for one request leaves a
    /// later one alone.
    serial: u64,
    /// Where a drop put the item (`super::landing`); `None` for a request
    /// made when a lift's item is drawn again, which any node of its key
    /// satisfies.
    landing: Option<Rc<Landing>>,
    /// Whether a check (`try_pending_focus`) is scheduled for the request.
    check_due: bool,
    /// The entry that drops a drop's request when the focus moves on from
    /// the lifted item to another element (`leave_entry`).
    leave: Option<DismissHandle>,
    /// The lifted item a drop's request was parked from (`leave_entry`).
    holder: Option<NodeHandle>,
    /// Whether the focus fell from `holder` to the document (no element,
    /// `<body>` or the root) since the last check: that check decides
    /// whether it ends the request (`try_pending_focus`).
    fell: bool,
    /// The observers that watch a list without `keyboard` for the item, by
    /// the list's node (#3062). Such a list registers none of its own: an
    /// insertion observer makes every insertion on the thread walk its
    /// ancestors, so a list pays for one only while a request names it. A
    /// `keyboard` list has its own for good (`offer_keyboard`).
    watches: Vec<(NodeId, Watch)>,
}

impl FocusRequest {
    /// Whether the request names the list `id` of `group` in `doc`.
    fn targets(&self, doc: u64, group: &str, id: &str) -> bool {
        self.doc == doc && self.group == group && self.list.as_ref().is_none_or(|l| l == id)
    }
}

/// An observer registered under a scope of its own, released when dropped.
struct Watch(Scope);

impl Drop for Watch {
    fn drop(&mut self) {
        self.0.dispose();
    }
}

/// The pending request, if any: one slot for every document, the last
/// request winning. Its own [`ForgetDoc`](rinch_core::ForgetDoc) because it
/// is one slot, not a map: a document's sweep clears it when the request is
/// that document's.
pub(super) struct FocusSlot(pub(super) Option<FocusRequest>);

impl rinch_core::ForgetDoc for FocusSlot {
    fn take_doc(&mut self, doc: u64) -> rinch_core::Removed {
        if self.0.as_ref().is_some_and(|p| p.doc == doc) {
            vec![Box::new(self.0.take())]
        } else {
            Vec::new()
        }
    }

    fn entries_for_doc(&self, doc: u64) -> usize {
        usize::from(self.0.as_ref().is_some_and(|p| p.doc == doc))
    }
}

thread_local! {
    static NEXT_SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// What a list tells the items it sees about itself (`see_items`).
pub(super) struct Seer {
    pub(super) group: String,
    pub(super) id: String,
    pub(super) keyboard: bool,
}

/// Focus the item `key` of `group` once it is drawn in `list`: when a list
/// the request names sees it (its render, or a later arrival in it), and on
/// the next turn of the event loop (timers permitting) if it is already
/// there. Replaces any earlier request; dropped once satisfied, at the next
/// drag, and when no list it names is left.
pub(super) fn request_focus(doc: u64, group: String, key: String, list: Option<String>) {
    park(doc, group, key, list, None);
}

/// [`request_focus`] for the item a keyboard drop moved: satisfied by a node
/// of its key only once its list shows the drop applied (`landing`, see
/// `super::landing` for the rule), never by the node it was lifted as, or a
/// node drawn at its old place, while the list is still in its order at the
/// drop. Dropped too when the focus moves on from `holder`, the lifted item,
/// to another element, or falls to the document while `holder` is still
/// drawn once the tree has settled (`leave_entry`): the user went elsewhere,
/// and an app that applies the drop later must not take the focus back from
/// there.
pub(super) fn request_drop_focus(
    doc: u64,
    group: String,
    key: String,
    list: Option<String>,
    landing: Landing,
    holder: &NodeHandle,
) {
    let serial = park(doc, group, key, list, Some(Rc::new(landing)));
    // Satisfied already when the app moved the item in the drop's batch.
    if !is_pending(serial) {
        return;
    }
    let leave = leave_entry(holder, serial);
    let unused = PENDING_FOCUS.with_mut(|p| match p.0.as_mut().filter(|r| r.serial == serial) {
        Some(r) => {
            r.leave = Some(leave);
            r.holder = Some(holder.clone());
            None
        }
        None => Some(leave),
    });
    drop(unused);
}

/// Park a request (see [`request_focus`]): its serial.
fn park(
    doc: u64,
    group: String,
    key: String,
    list: Option<String>,
    landing: Option<Rc<Landing>>,
) -> u64 {
    let serial = NEXT_SERIAL.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    let request = FocusRequest {
        doc,
        group: group.clone(),
        key,
        list,
        serial,
        landing,
        // The check scheduled below.
        check_due: true,
        leave: None,
        holder: None,
        fell: false,
        watches: Vec::new(),
    };
    // Pending before the lists are read: reading them can flush the app's
    // re-render (the drop's), and a list drawn then must see the request
    // (`see_items` at its render, `watch_if_named`). The superseded request
    // (and its watches) goes outside the borrow.
    let old = PENDING_FOCUS.with_mut(|p| p.0.replace(request));
    drop(old);
    for cfg in lists_of_group(&group, doc) {
        if !cfg.keyboard && still_drawn(&cfg) {
            attach_watch(serial, &cfg.node, seer_of(&cfg));
        }
    }
    // Unowned: the item that parked it is usually the node the re-render
    // replaced, and a parked callback of a disposed owner is dropped.
    //
    // Where no timer backend is installed (`rinch_core::set_timer_backend`:
    // an embedder that installs none), this check and every later one
    // (`schedule_check`, `list_gone`'s orphan sweep) never run. A list's
    // seeing a new node of the key in the applied order (`see_items`) is then
    // the only place a drop's request is satisfied, so a drop the app applies
    // by a keyed move, which keeps the item's node and draws no new one, is
    // never checked: the item keeps the focus it held, and the request stays
    // until the next drag, the focus moving from the item to another element,
    // the focus falling to the document while the item is still drawn (which
    // `leave_entry` then decides at once, `checks_run`), or a document sweep.
    rinch_core::unowned(|| set_timeout(0, try_pending_focus));
    serial
}

#[cfg(test)]
thread_local! {
    /// Set by a test to stand for a host with no timer backend.
    pub(super) static NO_TIMERS_FOR_TEST: Cell<bool> = const { Cell::new(false) };
}

/// Whether the checks this module schedules will run: whether a timeout
/// fires on this thread (`rinch_core::timers_fire`).
fn checks_run() -> bool {
    #[cfg(test)]
    if NO_TIMERS_FOR_TEST.with(Cell::get) {
        return false;
    }
    rinch_core::timers_fire()
}

/// Whether the request `serial` is still pending.
fn is_pending(serial: u64) -> bool {
    PENDING_FOCUS.with(|p| p.0.as_ref().is_some_and(|r| r.serial == serial))
}

/// The focus-leave entry on `holder` that drops the request `serial` when
/// the focus goes to another element.
///
/// When the focus falls to the document instead (no element, `<body>` or
/// the root), nothing is decided here: the request is marked
/// (`FocusRequest::fell`) and the check once the tree has settled decides
/// (`try_pending_focus`). A click on blank space does that, and so does a
/// redraw that applies the drop: rinch-web blurs a focused node before it
/// removes or moves it, while the node is still in place
/// (`DomDocument::release_focus_within`), so the browser's `focusout` reaches
/// this entry before the redraw has drawn anything; desktop lets the focus
/// go once the node is detached. Settled, the redraw has satisfied the
/// request, or the lifted item is still drawn with the focus still on the
/// document (a click on blank space), which ends it.
///
/// Where no timer backend is installed (`checks_run`), no check would ever
/// come, so the fall is decided at once: the request ends if the lifted item
/// is still connected, and stays for a node drawn in its place if the item
/// is already detached. Since rinch-web blurs a node while it is still in
/// place (above), on such a host a redraw that removes or moves the focused
/// item ends the request too, and the item the drop lands on does not get
/// the focus back: a click on blank space must never be undone later.
///
/// Unowned, like the request: the render running at the drop is not the
/// request's.
fn leave_entry(holder: &NodeHandle, serial: u64) -> DismissHandle {
    let owner = holder.clone();
    rinch_core::unowned(|| {
        rinch_core::push_key_handler(
            holder,
            |_| false,
            move || {
                let Some(focus) = focused_element(&owner) else {
                    if !checks_run() {
                        // No check will come: decide at once. The lifted item
                        // still drawn means the user went elsewhere; one a
                        // redraw took away keeps the request for its
                        // successor (`see_items`).
                        if owner.is_connected() {
                            drop(take_request(serial));
                        }
                        return;
                    }
                    let marked = PENDING_FOCUS.with_mut(|p| {
                        p.0.as_mut()
                            .filter(|r| r.serial == serial)
                            .map(|r| r.fell = true)
                            .is_some()
                    });
                    if marked {
                        schedule_check(serial);
                    }
                    return;
                };
                // A window blur calls this too, with the focus still here.
                let mut at = Some(focus);
                while let Some(n) = at {
                    if n.node_id() == owner.node_id() {
                        return;
                    }
                    at = n.parent_node();
                }
                let old = take_request(serial);
                drop(old);
            },
        )
    })
}

/// The element holding the focus in `node`'s document, unless the focus is
/// on the document itself (no element, `<body>` or the root).
fn focused_element(node: &NodeHandle) -> Option<NodeHandle> {
    node.active_element().filter(|f| {
        !f.tag_name()
            .is_some_and(|t| t.eq_ignore_ascii_case("body") || t.eq_ignore_ascii_case("html"))
    })
}

/// Take the request `serial` out of the slot, if it is still the pending
/// one. The caller drops it outside the registry's borrow (its watches and
/// entry go with it).
fn take_request(serial: u64) -> Option<FocusRequest> {
    PENDING_FOCUS.with_mut(|p| {
        p.0.as_ref()
            .is_some_and(|r| r.serial == serial)
            .then(|| p.0.take())
            .flatten()
    })
}

/// Check the request `serial` once the tree has settled (the next turn of
/// the event loop), unless a check is already due.
fn schedule_check(serial: u64) {
    let due = PENDING_FOCUS.with_mut(|p| {
        p.0.as_mut()
            .filter(|r| r.serial == serial && !r.check_due)
            .map(|r| r.check_due = true)
            .is_some()
    });
    if due {
        rinch_core::unowned(|| set_timeout(0, try_pending_focus));
    }
}

/// Something arrived in the list `seer` of `doc`: when a drop's request
/// names it, the item may now be where the drop put it without having moved
/// itself (a keyed redraw can move its neighbours round it), so check it once
/// the tree has settled. A request without a landing is satisfied by the
/// arrival itself (`see_items`).
pub(super) fn saw_arrival(doc: u64, seer: &Seer) {
    let serial = PENDING_FOCUS.with(|p| {
        p.0.as_ref()
            .filter(|r| r.landing.is_some() && r.targets(doc, &seer.group, &seer.id))
            .map(|r| r.serial)
    });
    if let Some(serial) = serial {
        schedule_check(serial);
    }
}

fn seer_of(cfg: &ListConfig) -> Rc<Seer> {
    Rc::new(Seer {
        group: cfg.group.clone(),
        id: cfg.id.clone(),
        keyboard: cfg.keyboard,
    })
}

/// Watch the list `node` for the items that arrive in it.
fn watch(node: &NodeHandle, seer: Rc<Seer>) -> (NodeId, Watch) {
    let scope = Scope::new();
    let doc = node.doc_key();
    scope.run(|| {
        crate::late_children::observe_late_children(node, NOT_MINE, move |inserted| {
            see_items(inserted, &seer);
            saw_arrival(doc, &seer);
        })
    });
    (node.node_id(), Watch(scope))
}

/// Drop the pending request, if any (a drag starts).
pub(super) fn cancel_pending_focus() {
    let old = PENDING_FOCUS.with_mut(|p| p.0.take());
    drop(old);
}

/// Take the pending request if it is for `node`, the item `key` of the list
/// `seer` in `doc` (`claim`): whether it was. A drop's request is taken here
/// only by a new node in a list that shows the drop applied; anything else is
/// checked once the tree has settled instead, and so is any node while the
/// focus has fallen from the lifted item and that item is still drawn: the
/// check decides whether the user clicked blank space (`try_pending_focus`).
pub(super) fn take_request_for(doc: u64, seer: &Seer, key: &str, node: &NodeHandle) -> bool {
    let Some((serial, landing, fell_from)) = PENDING_FOCUS.with(|p| {
        p.0.as_ref()
            .filter(|r| r.key == key && r.targets(doc, &seer.group, &seer.id))
            .map(|r| {
                let fell_from = r.holder.clone().filter(|_| r.fell);
                (r.serial, r.landing.clone(), fell_from)
            })
    }) else {
        return false;
    };
    if fell_from.is_some_and(|h| h.is_connected()) {
        schedule_check(serial);
        return false;
    }
    let keys = match landing {
        Some(_) => keys_of_list_of(node),
        None => Vec::new(),
    };
    match claim(landing.as_deref(), node, &keys, false) {
        Claim::Take => take_request(serial).is_some(),
        Claim::Later => {
            schedule_check(serial);
            false
        }
        Claim::No => false,
    }
}

/// Whether a request is pending for the list `seer` of `doc`.
pub(super) fn request_names(doc: u64, seer: &Seer) -> bool {
    PENDING_FOCUS.with(|p| {
        p.0.as_ref()
            .is_some_and(|r| r.targets(doc, &seer.group, &seer.id))
    })
}

/// A list without `keyboard`, `node`, just drawn: watch it for the item when
/// the pending request names it (a list re-rendered under a drop).
pub(super) fn watch_if_named(node: &NodeHandle, seer: Rc<Seer>) {
    if seer.keyboard {
        return;
    }
    if let Some(serial) = PENDING_FOCUS.with(|p| p.0.as_ref().map(|r| r.serial)) {
        attach_watch(serial, node, seer);
    }
}

/// Whether the list `cfg` is still registered: one read before a flush that
/// re-rendered it has gone since.
fn still_drawn(cfg: &Rc<ListConfig>) -> bool {
    LISTS.with(|l| l.iter().any(|e| Rc::ptr_eq(&e.2, cfg)))
}

/// Watch the list `node` (`seer`) for the request `serial`'s item, if that
/// request is still pending, names the list, and has no watch on it yet.
fn attach_watch(serial: u64, node: &NodeHandle, seer: Rc<Seer>) {
    let id = node.node_id();
    let doc = node.doc_key();
    let wanted = PENDING_FOCUS.with(|p| {
        p.0.as_ref().is_some_and(|r| {
            r.serial == serial
                && r.targets(doc, &seer.group, &seer.id)
                && r.watches.iter().all(|(n, _)| *n != id)
        })
    });
    if !wanted {
        return;
    }
    let watched = watch(node, seer);
    let unused = PENDING_FOCUS.with_mut(|p| match p.0.as_mut().filter(|r| r.serial == serial) {
        Some(r) => {
            r.watches.push(watched);
            None
        }
        None => Some(watched),
    });
    drop(unused);
}

/// The list `node` (`seer`) of `doc` is going away: its watch goes, and the
/// request goes too when no list it names is drawn by the next turn of the
/// event loop. Not at once: a list re-rendered as a new node may go before
/// its successor is drawn, and the request is for the successor (#3062).
pub(super) fn list_gone(doc: u64, node: NodeId, seer: &Seer) {
    let (watch, serial) = PENDING_FOCUS.with_mut(|p| {
        let Some(r) = p.0.as_mut() else {
            return (None, None);
        };
        let watch = r
            .watches
            .iter()
            .position(|(n, _)| *n == node)
            .map(|at| r.watches.swap_remove(at));
        let serial = r.targets(doc, &seer.group, &seer.id).then_some(r.serial);
        (watch, serial)
    });
    drop(watch);
    if let Some(serial) = serial {
        rinch_core::unowned(|| set_timeout(0, move || drop_if_orphaned(serial)));
    }
}

/// Drop the request `serial`, if it is still pending, when no list it names
/// is drawn.
fn drop_if_orphaned(serial: u64) {
    let orphaned = PENDING_FOCUS.with(|p| {
        p.0.as_ref().filter(|r| r.serial == serial).map(|r| {
            !lists_of_group(&r.group, r.doc)
                .iter()
                .any(|c| r.targets(r.doc, &c.group, &c.id))
        })
    });
    if orphaned == Some(true) {
        cancel_pending_focus();
    }
}

/// The scheduled check: focus the item the pending request is for, if it is
/// drawn in a list the request names and, for a drop's, that list shows the
/// drop applied (`claim`, the tree settled). Otherwise, when the focus fell
/// to the document since the last check (`FocusRequest::fell`, see
/// `leave_entry`) and the lifted item is still drawn with the focus still on
/// the document, the request ends: no redraw took the item away, the user
/// clicked blank space. An item taken away (detached) keeps it.
///
/// The fall is read before any node is claimed: a click on blank space and a
/// redraw by position applied in the same turn (the lifted item's node still
/// drawn, another node now holding its key where the drop put it) must end
/// the request, not hand that node the focus. Only the lifted item itself,
/// still drawn and now where the drop put it, is focused after a fall: the
/// web blurs an item before a keyed redraw moves it (`leave_entry`), and that
/// redraw applied the drop.
pub(super) fn try_pending_focus() {
    let Some((serial, doc, group, key, list, landing, fell_from)) = PENDING_FOCUS.with_mut(|p| {
        p.0.as_mut().map(|r| {
            r.check_due = false;
            let fell_from = std::mem::take(&mut r.fell)
                .then(|| r.holder.clone())
                .flatten();
            (
                r.serial,
                r.doc,
                r.group.clone(),
                r.key.clone(),
                r.list.clone(),
                r.landing.clone(),
                fell_from,
            )
        })
    }) else {
        return;
    };
    // The lifted item, when the focus fell from it to the document and it is
    // still drawn with the focus still there.
    let stranded = fell_from.filter(|h| h.is_connected() && focused_element(h).is_none());
    // Not yet applied (the app applies the drop later): a list that sees the
    // item drawn again, or the next check, focuses it.
    for cfg in lists_of_group(&group, doc)
        .into_iter()
        .filter(|c| list.as_ref().is_none_or(|l| *l == c.id))
    {
        let items = items_of(&cfg.node);
        let keys: Vec<String> = items.iter().map(|(_, k)| k.clone()).collect();
        for (node, _) in items.into_iter().filter(|(_, k)| *k == key) {
            if matches!(claim(landing.as_deref(), &node, &keys, true), Claim::Take) {
                let mine = stranded
                    .as_ref()
                    .is_none_or(|h| h.node_id() == node.node_id());
                if take_request(serial).is_some() && mine {
                    focus_item(&node);
                }
                return;
            }
        }
    }
    if stranded.is_some() {
        let old = take_request(serial);
        drop(old);
    }
}

/// Give a keyboard lift's item the focus. An item is a Tab stop only in a
/// [`SortableList::keyboard`] list (`offer_keyboard`); one dropped into a
/// list without `keyboard` has no `tabindex` and is lent one
/// (`lend_tabindex`).
///
/// Called only once the app's attributes on the item are written: by its
/// list, when it sees the item (`see_items`), or later (`try_pending_focus`).
/// Never from the item's own render, which comes before them: a `tabindex`
/// the app writes on it would be written after the lend, and taken for the
/// lend when the focus leaves (#3062).
pub(super) fn focus_item(item: &NodeHandle) {
    if item.get_attribute("tabindex").is_none() {
        lend_tabindex(item);
    }
    item.focus();
}

/// Make `item` focusable for as long as it holds the focus, and no Tab stop:
/// `tabindex="-1"`, removed once the focus is elsewhere (#3062). Removed
/// while the item holds the focus, it would take the focus with it (Chrome
/// blurs inside that write); kept after, the item would be a focusable stop
/// in a list that offers none, until it is drawn again. A `keyboard` list
/// that adopts the item makes it its own Tab stop instead (`take_lent`).
///
/// The record lives exactly as long as the item: its focus-leave entry is
/// unowned (whatever render is running when the list sees the item is not
/// the item's), and the item's cleanup forgets the record and drops the entry
/// with it (`forget_lent`). So a node id the record names is never a node
/// that took a removed item's place.
fn lend_tabindex(item: &NodeHandle) {
    item.set_attribute("tabindex", "-1");
    let owner = item.clone();
    let entry = rinch_core::unowned(|| {
        rinch_core::push_key_handler(
            item,
            |_| false,
            move || {
                // A window blur calls this too, with the focus still here.
                let mut at = owner.active_element();
                while let Some(n) = at {
                    if n.node_id() == owner.node_id() {
                        return;
                    }
                    at = n.parent_node();
                }
                if take_lent(&owner) && owner.get_attribute("tabindex").as_deref() == Some("-1") {
                    owner.remove_attribute("tabindex");
                }
            },
        )
    });
    let doc = item.doc_key();
    let stale = LENT_TABINDEX.with_mut(|l| {
        // An item the document let go of by a route that ran no cleanup of
        // its own (an ancestor's markup replaced) is forgotten here. Retired,
        // not detached: an item taken out and put back (a branch hidden and
        // shown again) is the same node, and still holds the lend, which its
        // focus-leave entry takes back.
        let (kept, stale): (Vec<_>, Vec<_>) = std::mem::take(&mut **l)
            .into_iter()
            .partition(|(_, n, _)| !n.is_retired());
        **l = kept;
        l.push((doc, item.clone(), entry));
        stale
    });
    drop(stale);
}

/// Forget the `tabindex` lent to `item`, if it was: whether it was.
pub(super) fn take_lent(item: &NodeHandle) -> bool {
    forget_lent(item.doc_key(), item.node_id())
}

/// Forget the `tabindex` lent to the item `id` of document `doc`, if it was
/// (and drop its focus-leave entry): whether it was. The item's cleanup calls
/// this, so the record goes with the item.
pub(super) fn forget_lent(doc: u64, id: NodeId) -> bool {
    let taken = LENT_TABINDEX.with_mut(|l| {
        l.iter()
            .position(|(d, n, _)| *d == doc && n.node_id() == id)
            .map(|i| l.swap_remove(i))
    });
    // The entry is released outside the registry's borrow.
    taken.is_some()
}
