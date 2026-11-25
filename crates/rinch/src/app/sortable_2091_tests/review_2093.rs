//! The review of #2093: a co-edited board re-renders whole lanes under a
//! drag; the place and verdict at the drop; focus and announcements.
use super::*;
use rinch_core::events::{self, ClickContext, EventHandlerId};

fn key(f: &mut Fixture, key: KeyCode, text: Option<&str>) {
    send(
        f,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
    );
}

fn focus_card(f: &mut Fixture, k: &str) {
    let at = point(f, item(f, k), 0.5, 0.5);
    press_at(f, at);
    release_at(f, at);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(f, k)));
}

fn escape(f: &mut Fixture) {
    key(f, KeyCode::Escape, None);
}

/// A collaborator's edit to lane `lane` (a card added to its last
/// column): every node of the lane is new.
fn rerender_lane(f: &mut Fixture, lane: usize) {
    f.board.update(|b| {
        let cols = &mut b[lane].cols;
        let last = cols.len() - 1;
        cols[last].cards.push(format!("new{lane}"));
    });
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn live_text(f: &Fixture) -> Vec<String> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(id, n)| has_class(n, "rinch-sortable__live") && connected(&d, *id))
        .map(|(_, n)| {
            n.children
                .iter()
                .filter_map(|c| d.tree.nodes[*c].text_content())
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

fn handler(f: &Fixture, id: u64, attr: &str) -> EventHandlerId {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    EventHandlerId(d.tree.nodes[id].attributes[attr].parse().unwrap())
}

// ── 1. The drag ends with the runtime's drag, whatever the source became.

#[test]
fn a_rerendered_source_is_dimmed_and_escape_still_ends_the_drag() {
    let mut f = fixture();
    SELECTION.with(|s| *s.borrow_mut() = vec!["a".into(), "h".into()]);
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "d"), 0.5, 0.75);
    move_to(&mut f, at);
    let old_a = item(&f, "a");
    rerender_lane(&mut f, 0);
    assert_ne!(item(&f, "a"), old_a, "a was re-rendered");
    assert!(
        classes(&f, item(&f, "a")).contains("rinch-sortable__item--dragging"),
        "the re-rendered card is dimmed as carried"
    );
    escape(&mut f);
    assert!(f.app.active_dnd.is_none());
    assert_eq!(rinch_components::sortable_drag_group(), None);
    assert_clean(&f);
    // A drop that reaches a list later (a drag of something else, a
    // proxied event) finds nothing stale to deliver.
    let d = item(&f, "d");
    events::dispatch_event(handler(&f, d, "data-ondrop"));
    assert!(drops().is_empty());
}

#[test]
fn a_rerendered_source_released_outside_every_list_ends_the_drag() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, at);
    rerender_lane(&mut f, 0);
    // Below the board: no list there.
    move_to(&mut f, (700.0, 590.0));
    release_at(&mut f, (700.0, 590.0));
    assert_eq!(rinch_components::sortable_drag_group(), None);
    assert!(drops().is_empty());
    assert_clean(&f);
}

// ── 2. A keyboard lift goes on through re-renders of its lists.

#[test]
fn a_keyboard_move_goes_on_after_its_target_list_rerenders() {
    let mut f = fixture();
    focus_card(&mut f, "h");
    key(&mut f, KeyCode::Space, Some(" "));
    // Up from c5's only card: into lane 1 above, at its column's end.
    key(&mut f, KeyCode::ArrowUp, None);
    let (m, _) = last_offer(&f).expect("an offer in lane 1");
    let to = m.to_list.clone();
    assert_eq!(to, "c1", "{m:?}");
    rerender_lane(&mut f, 0);
    key(&mut f, KeyCode::ArrowUp, None);
    let (m2, _) = last_offer(&f).expect("the lift still moves");
    assert_eq!(m2.to_list, to);
    assert_eq!(m2.place.index + 1, m.place.index, "one place up");
    key(&mut f, KeyCode::Enter, None);
    let delivered = drops();
    assert_eq!(delivered.len(), 1, "delivered after the re-render");
    assert_eq!(delivered[0].to_list, to);
    assert!(cards(&f, &to).contains(&"h".to_string()));
}

#[test]
fn a_lift_goes_on_when_its_own_lane_rerenders() {
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowDown, None);
    rerender_lane(&mut f, 0);
    send(&mut f, PlatformEvent::AboutToWait);
    // The lifted card is a new node, ringed and holding the keyboard.
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--lifted"));
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    key(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("c"), Some("i"), 2)
    );
    // Announced in the lane's new live region.
    assert!(
        live_text(&f).iter().any(|t| t.contains("position 3")),
        "{:?}",
        live_text(&f)
    );
    key(&mut f, KeyCode::Enter, None);
    assert_eq!(cards(&f, "c1")[..4], ["b", "c", "a", "i"]);
}

// ── 4. The drop settles the place and the verdict again.

#[test]
fn a_verdict_that_changed_since_the_offer_refuses_the_drop() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, at);
    assert_eq!(last_offer(&f).unwrap().1, SortVerdict::Accept);
    FROZEN.with(|x| x.set(true));
    release_at(&mut f, at);
    assert!(drops().is_empty(), "judged again at the drop: refused");
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "i", "j"]);
}

#[test]
fn the_place_is_settled_by_its_keys_against_the_lists_at_the_drop() {
    let mut f = fixture();
    let a = item(&f, "a");
    events::dispatch_event(handler(&f, a, "data-ondragstart"));
    let c = item(&f, "c");
    let at = point(&f, c, 0.5, 0.75);
    let (ex, ey, ew, eh) = box_of(&f, c);
    events::set_click_context(ClickContext {
        mouse_x: at.0,
        mouse_y: at.1,
        element_x: ex,
        element_y: ey,
        element_width: ew,
        element_height: eh,
        ..Default::default()
    });
    events::dispatch_event(handler(&f, c, "data-ondragover"));
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("c"), Some("i"), 2)
    );
    // A collaborator puts z right after c and moves b ahead of a: the
    // lane is re-rendered; the drop arrives at the new c.
    f.board.update(|b| {
        let c1 = &mut b[0].cols[0].cards;
        c1.insert(3, "z".into());
        c1.retain(|k| k != "b");
        c1.insert(0, "b".into());
    });
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let c = item(&f, "c");
    events::dispatch_event(handler(&f, c, "data-ondrop"));
    let delivered = drops();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].place,
        place(Some("c"), Some("z"), 2),
        "after c, as offered; its neighbour and index as the list is now"
    );
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "z", "i", "j"]);
}

#[test]
fn a_place_whose_anchor_left_the_list_is_not_delivered() {
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowDown, None);
    key(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(last_offer(&f).unwrap().0.place.after.as_deref(), Some("c"));
    f.board.update(|b| b[0].cols[0].cards.retain(|k| k != "c"));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    key(&mut f, KeyCode::Enter, None);
    assert!(drops().is_empty(), "after c, and c is gone");
    assert!(live_text(&f).iter().any(|t| t.contains("Not moved")));
}

// ── 6a / 9. The dropped item takes the focus when it is drawn.

#[test]
fn the_dropped_item_is_focused_when_drawn_late_with_no_timer_after() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    let mut f = fixture();
    APPLY.with(|a| a.set(false));
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowRight, None);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    key(&mut f, KeyCode::Enter, None);
    let delivered = drops().pop().expect("delivered");
    // A timer that fires before the app re-renders finds a in c1, not
    // where it was dropped, and leaves the request standing.
    for (id, _) in SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        rinch_core::fire_timeout(id);
    }
    send(&mut f, PlatformEvent::AboutToWait);
    // The app applies the move later; no timer fires after it.
    f.board.update(|b| apply(b, &delivered));
    send(&mut f, PlatformEvent::AboutToWait);
    send(&mut f, PlatformEvent::AboutToWait);
    assert_eq!(cards(&f, "c2"), ["a", "d"]);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
}

// ── 8. Announcements.

#[test]
fn identical_announcements_change_the_region_and_items_describe_themselves() {
    let mut f = fixture();
    let a = item(&f, "a");
    {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let attrs = &d.tree.nodes[a].attributes;
        assert_eq!(
            attrs.get("aria-roledescription").map(String::as_str),
            Some("sortable item")
        );
        assert!(
            attrs
                .get("aria-description")
                .is_some_and(|t| t.contains("Space"))
        );
        let live = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| has_class(n, "rinch-sortable__live"))
            .unwrap()
            .1;
        assert_eq!(
            live.attributes.get("role"),
            None,
            "no status role beside assertive"
        );
        assert_eq!(
            live.attributes.get("aria-atomic").map(String::as_str),
            Some("true")
        );
    }
    focus_card(&mut f, "h");
    key(&mut f, KeyCode::Space, Some(" "));
    // h is alone in c5, the bottom lane: Down has nowhere to go, and says
    // so twice in a row.
    key(&mut f, KeyCode::ArrowDown, None);
    let first = live_text(&f);
    key(&mut f, KeyCode::ArrowDown, None);
    let second = live_text(&f);
    let bare = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|t| t.trim_end_matches('\u{a0}').to_string())
            .collect()
    };
    assert_eq!(bare(&first), bare(&second), "the same sentence");
    assert_ne!(first, second, "but a change a screen reader hears");
}

// ── The ghost attribute.

#[test]
fn data_drag_ghost_false_hides_the_built_in_ghost() {
    let mut f = fixture();
    // A sortable item says it: the runtime hides its ghost at the start,
    // before any handler runs.
    start_card(&mut f, "a");
    assert!(!events::is_drag_ghost_visible());
    escape(&mut f);
    assert!(events::is_drag_ghost_visible());
    assert_eq!(
        f.app.doc.as_ref().unwrap().borrow().tree.nodes[list_holding(&f, "a")]
            .attributes
            .get("data-drag-ghost")
            .map(String::as_str),
        Some("false")
    );
}
