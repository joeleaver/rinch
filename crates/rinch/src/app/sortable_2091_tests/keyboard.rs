use super::*;

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

fn live_text(f: &Fixture) -> String {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let lists: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(id, n)| has_class(n, "rinch-sortable__live") && connected(&d, *id))
        .map(|(id, _)| id)
        .collect();
    lists
        .iter()
        .flat_map(|l| d.tree.nodes[*l].children.clone())
        .filter_map(|c| d.tree.nodes[c].text_content().map(str::to_string))
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn space_lifts_arrows_move_within_and_across_and_enter_drops() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--lifted"));
    assert!(live_text(&f).contains("Picked up a"), "{}", live_text(&f));
    assert_eq!(last_offer(&f), None, "lifted where it is: nothing on offer");

    key(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(
        last_offer(&f),
        Some((
            mv("cards", &["a"], "c1", "c1", place(Some("b"), Some("c"), 1)),
            SortVerdict::Accept
        ))
    );
    key(&mut f, KeyCode::ArrowDown, None);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("c"), Some("i"), 2)
    );
    key(&mut f, KeyCode::ArrowUp, None);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    // Across: the column to the right, at the same index (clamped).
    key(&mut f, KeyCode::ArrowRight, None);
    let expected = mv("cards", &["a"], "c1", "c2", place(Some("d"), None, 1));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert!(
        live_text(&f).contains("c2, position 2"),
        "{}",
        live_text(&f)
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-after"),
        vec![item(&f, "d")]
    );

    SCHEDULED.with(|s| s.borrow_mut().clear());
    key(&mut f, KeyCode::Enter, None);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c2"), ["d", "a"]);
    assert_clean(&f);
    // The moved card, a new node in c2, takes the focus back.
    let sched = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for (id, _) in sched {
        rinch_core::fire_timeout(id);
    }
    // The runtime runs the resumed callback, then applies the focus
    // request it posted, at its next turns.
    send(&mut f, PlatformEvent::AboutToWait);
    send(&mut f, PlatformEvent::AboutToWait);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
}

#[test]
fn down_past_the_end_of_a_column_goes_to_the_lane_below() {
    let mut f = fixture();
    focus_card(&mut f, "d");
    key(&mut f, KeyCode::Space, Some(" "));
    // d is alone in c2: down is past its end, into the lane below, at
    // the start of the nearest column there (c5 or c6 by position).
    key(&mut f, KeyCode::ArrowDown, None);
    let (m, _) = last_offer(&f).expect("an offer in the lane below");
    assert!(["c5", "c6"].contains(&m.to_list.as_str()), "{m:?}");
    assert!(m.place.is_first());
}

#[test]
fn escape_cancels_a_lift_and_nothing_moves() {
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Enter, None);
    key(&mut f, KeyCode::ArrowDown, None);
    assert!(last_offer(&f).is_some());
    key(&mut f, KeyCode::Escape, None);
    assert_eq!(OFFERS.with(|o| o.borrow().last().cloned()), Some(None));
    assert!(drops().is_empty());
    assert_clean(&f);
    assert!(live_text(&f).contains("Cancelled"));
    // The focus stayed on the card; the next Space lifts it again.
    key(&mut f, KeyCode::Space, Some(" "));
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--lifted"));
}

#[test]
fn a_refused_place_is_said_and_not_dropped() {
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowRight, None);
    key(&mut f, KeyCode::ArrowRight, None);
    let (m, verdict) = last_offer(&f).unwrap();
    assert_eq!(m.to_list, "c3");
    assert_eq!(verdict, SortVerdict::Refuse("WIP limit 3".into()));
    assert!(live_text(&f).contains("WIP limit 3"));
    key(&mut f, KeyCode::Enter, None);
    assert!(drops().is_empty());
    assert_eq!(cards(&f, "c3"), ["e", "f", "g"]);
    assert_clean(&f);
}

#[test]
fn a_click_elsewhere_cancels_a_lift() {
    let mut f = fixture();
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowDown, None);
    focus_card(&mut f, "d");
    assert_clean(&f);
    assert!(drops().is_empty());
}

#[test]
fn columns_move_with_left_and_right() {
    let mut f = fixture();
    // Focus c1 by Tab-free means: a click on its header focuses the item.
    let header = point(&f, handle_of(&f, "c1"), 0.5, 0.5);
    press_at(&mut f, header);
    release_at(&mut f, header);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "c1")));
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowRight, None);
    assert_eq!(
        last_offer(&f).map(|(m, _)| (m.group, m.place)),
        Some(("columns".to_string(), place(Some("c2"), Some("c3"), 1)))
    );
    key(&mut f, KeyCode::Space, Some(" "));
    let order: Vec<String> = f.board.get()[0].cols.iter().map(|c| c.id.clone()).collect();
    assert_eq!(order, ["c2", "c1", "c3", "c4"]);
}

/// Drop `a`, lifted in c1, into c2 by keyboard, and let the moved card (a new
/// node in c2) take the focus back.
fn drop_a_into_c2(f: &mut Fixture) {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    focus_card(f, "a");
    key(f, KeyCode::Space, Some(" "));
    key(f, KeyCode::ArrowRight, None);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    key(f, KeyCode::Enter, None);
    assert_eq!(cards(f, "c2"), ["a", "d"]);
    let sched = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for (id, _) in sched {
        rinch_core::fire_timeout(id);
    }
    send(f, PlatformEvent::AboutToWait);
    send(f, PlatformEvent::AboutToWait);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(f, "a")));
}

fn tabindex(f: &Fixture, k: &str) -> Option<String> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[item(f, k)].attributes.get("tabindex").cloned()
}

/// #3062 review: a card dropped by keyboard into a list without `keyboard` is
/// made focusable to take the focus, and is no Tab stop there — not while it
/// holds the focus (`-1`), nor after (the `tabindex` goes with the focus).
#[test]
fn a_card_dropped_into_a_plain_list_is_no_tab_stop_there() {
    let mut f = fixture_with(Some("c2"), |_| None);
    assert_eq!(tabindex(&f, "d"), None, "c2 offers no Tab stops");
    drop_a_into_c2(&mut f);
    assert_eq!(tabindex(&f, "a").as_deref(), Some("-1"));
    // The focus moves on (to b, in keyboard column c1).
    focus_card(&mut f, "b");
    assert_eq!(tabindex(&f, "a"), None);
    assert_eq!(tabindex(&f, "b").as_deref(), Some("0"));
}

/// The control: a `tabindex` the app wrote on the card (its root attribute,
/// written after the card's render) is the app's, and stays when the focus
/// leaves it.
#[test]
fn a_tabindex_the_app_wrote_stays_after_the_drop() {
    let mut f = fixture_with(Some("c2"), |_| Some("0"));
    drop_a_into_c2(&mut f);
    assert_eq!(tabindex(&f, "a").as_deref(), Some("0"));
    focus_card(&mut f, "b");
    assert_eq!(tabindex(&f, "a").as_deref(), Some("0"));
}

/// #3062 review 2: the same for an app's `tabindex="-1"`, the value rinch
/// lends. The lend is decided once the app's attributes are written (by the
/// list that sees the card), so it is never lent over the app's, and never
/// taken away as rinch's when the focus leaves.
#[test]
fn an_app_written_minus_one_tabindex_stays_after_the_drop() {
    let mut f = fixture_with(Some("c2"), |_| Some("-1"));
    let doc = f.app.doc_key();
    drop_a_into_c2(&mut f);
    assert_eq!(tabindex(&f, "a").as_deref(), Some("-1"));
    assert_eq!(lent(doc), 0, "the app's tabindex was taken for a lend");
    focus_card(&mut f, "b");
    assert_eq!(tabindex(&f, "a").as_deref(), Some("-1"));
}

/// How many lent `tabindex` records document `doc` has.
fn lent(doc: u64) -> usize {
    const LENT: &str = "rinch_components::sortable::LENT_TABINDEX";
    rinch_core::doc_entries(doc)
        .into_iter()
        .find(|(n, _)| *n == LENT)
        .map_or(0, |(_, count)| count)
}

/// A lent `tabindex` the focus never left goes with its document (#2797).
#[test]
fn a_lent_tabindex_is_forgotten_with_its_document() {
    let mut f = fixture_with(Some("c2"), |_| None);
    let doc = f.app.doc_key();
    drop_a_into_c2(&mut f);
    assert_eq!(lent(doc), 1, "control: the dropped card's tabindex is lent");
    drop(f);
    assert_eq!(lent(doc), 0, "the lent tabindex outlived its document");
}

/// #3062 review 2: a lent `tabindex` goes with its item. The card is deleted
/// while it holds the lend (its focus-leave never runs), and a new card, with
/// the app's own `tabindex="-1"`, is then added to a keyboard list: the
/// record of the deleted card must neither linger nor be taken for the new
/// card's, which `offer_keyboard` would then make `"0"`.
#[test]
fn a_lent_tabindex_is_forgotten_with_its_item() {
    let mut f = fixture_with(Some("c2"), |k| (k == "z").then_some("-1"));
    let doc = f.app.doc_key();
    drop_a_into_c2(&mut f);
    assert_eq!(lent(doc), 1, "control: the dropped card's tabindex is lent");
    let gone = item(&f, "a");
    f.board.update(|b| b[0].cols[1].cards.retain(|k| k != "a"));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(cards(&f, "c2"), ["d"]);
    assert_eq!(lent(doc), 0, "the lent tabindex outlived its item");
    f.board.update(|b| b[0].cols[0].cards.push("z".into()));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let z = item(&f, "z");
    assert_ne!(z, gone, "a node id is never handed out again");
    assert_eq!(tabindex(&f, "z").as_deref(), Some("-1"));
}

/// Whether document `doc` has a pending focus request.
fn pending_focus(doc: u64) -> usize {
    const FOCUS: &str = "rinch_components::sortable::PENDING_FOCUS";
    rinch_core::doc_entries(doc)
        .into_iter()
        .find(|(n, _)| *n == FOCUS)
        .map_or(0, |(_, count)| count)
}

/// Fire the timers the manual backend has collected.
fn fire_timers() {
    let sched = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for (id, _) in sched {
        rinch_core::fire_timeout(id);
    }
}

/// Lift `a` in c1 and drop it into c2 by keyboard, the app not moving it
/// (`APPLY` off): the request for it waits on c2.
fn drop_a_into_c2_unapplied(f: &mut Fixture) {
    APPLY.with(|a| a.set(false));
    focus_card(f, "a");
    key(f, KeyCode::Space, Some(" "));
    key(f, KeyCode::ArrowRight, None);
    key(f, KeyCode::Enter, None);
    assert_eq!(drops().len(), 1, "control: dropped");
    assert_eq!(cards(f, "c2"), ["d"], "control: the app moved nothing");
}

/// #3062 review 3: c2 sees the dropped card arrive (the app moves it in the
/// drop's batch) and gives it the focus, and that ends the request, before
/// any timer: the watch of the plain list it named goes with it, so a plain
/// list adds no insertion observer outside a request.
#[test]
fn a_satisfied_drop_leaves_no_request_and_no_observer() {
    let mut f = fixture_with(Some("c2"), |_| None);
    let doc = f.app.doc_key();
    let base = rinch_core::dom::insert_observer_count();
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    focus_card(&mut f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowRight, None);
    key(&mut f, KeyCode::Enter, None);
    assert_eq!(
        list_holding(&f, "a"),
        list_holding(&f, "d"),
        "control: moved"
    );
    assert_eq!(pending_focus(doc), 0, "a satisfied request lingers");
    send(&mut f, PlatformEvent::AboutToWait);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(rinch_core::dom::insert_observer_count(), base);
    fire_timers();
}

/// #3062 review 3: a request whose list goes, and is not drawn again, goes
/// too, by the next turn of the event loop; the list's watch at once.
#[test]
fn a_request_goes_with_the_list_it_names() {
    let mut f = fixture_with(Some("c2"), |_| None);
    let doc = f.app.doc_key();
    let base = rinch_core::dom::insert_observer_count();
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    drop_a_into_c2_unapplied(&mut f);
    fire_timers();
    assert_eq!(pending_focus(doc), 1, "control: the request waits");
    assert_eq!(
        rinch_core::dom::insert_observer_count(),
        base + 1,
        "control: c2 watches for the card"
    );
    f.board.update(|b| b[0].cols.retain(|c| c.id != "c2"));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        rinch_core::dom::insert_observer_count(),
        base,
        "the watch outlived c2"
    );
    fire_timers();
    assert_eq!(
        pending_focus(doc),
        0,
        "the request outlived the list it names"
    );
}

/// The control: c2 drawn again as a new node (its lane re-keyed) keeps the
/// request, the new node watches, and the card arriving there later takes
/// the focus.
#[test]
fn a_request_survives_its_list_drawn_again() {
    let mut f = fixture_with(Some("c2"), |_| None);
    let doc = f.app.doc_key();
    let base = rinch_core::dom::insert_observer_count();
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    drop_a_into_c2_unapplied(&mut f);
    fire_timers();
    let old_c2 = list_holding(&f, "d");
    f.board.update(|b| b[0].id = "l1-again".into());
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_ne!(list_holding(&f, "d"), old_c2, "control: c2 was drawn again");
    fire_timers();
    assert_eq!(pending_focus(doc), 1, "the request went with the old c2");
    assert_eq!(
        rinch_core::dom::insert_observer_count(),
        base + 1,
        "the new c2 does not watch"
    );
    f.board.update(|b| {
        b[0].cols[0].cards.retain(|k| k != "a");
        b[0].cols[1].cards.insert(0, "a".into());
    });
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    fire_timers();
    send(&mut f, PlatformEvent::AboutToWait);
    assert_eq!(list_holding(&f, "a"), list_holding(&f, "d"));
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0);
    assert_eq!(rinch_core::dom::insert_observer_count(), base);
}
