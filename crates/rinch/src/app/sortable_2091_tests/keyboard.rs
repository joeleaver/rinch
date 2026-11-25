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
