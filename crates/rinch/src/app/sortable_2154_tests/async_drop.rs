//! A keyboard drop the app applies later (#3062 review 4): after an await or
//! a timer of its own. Until then the node the card was lifted as is still
//! where it was, holding the focus, and must not end the focus request; the
//! redraw that applies the drop gives the moved card the focus.

use super::*;

thread_local! {
    static SCHEDULED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn manual_timers(id: u64, _delay_ms: u32) {
    SCHEDULED.with(|s| s.borrow_mut().push(id));
}

/// How the app draws its cards.
#[derive(Clone, Copy)]
enum Rows {
    /// Keyed by position: a moved card is drawn again as a new node.
    ByIndex,
    /// Keyed by card: a moved card keeps its node.
    ByCard,
}

/// One keyboard list, c1 [a b c d], whose `ondrop` records the drop and
/// applies nothing: the test applies it later (`apply_drop`).
fn list_fixture(rows: Rows) -> Fixture {
    DROPS.with(|d| d.borrow_mut().clear());
    let app = mount(move |__scope: &mut RenderScope| {
        let board: Signal<Board> = Signal::new(vec![(
            "c1".to_string(),
            ["a", "b", "c", "d"].map(String::from).to_vec(),
        )]);
        BOARD.with(|b| b.set(Some(board)));
        // Each row as (its key, its card): the row is drawn again when either
        // changes.
        let rows = move || {
            let cards = board.get().into_iter().next().unwrap().1;
            cards
                .into_iter()
                .enumerate()
                .map(|(i, card)| match rows {
                    Rows::ByIndex => (i.to_string(), card),
                    Rows::ByCard => (card.clone(), card),
                })
                .collect::<Vec<_>>()
        };
        rsx! {
            div { style: "width: 800px; height: 600px; padding: 20px",
                SortableList {
                    keyboard: true, group: "cards", id: "c1",
                    style: "--rinch-sortable-gap: 10px; width: 150px",
                    ondrop: move |m: SortMove| DROPS.with(|d| d.borrow_mut().push(m)),
                    for (row, card) in rows() {
                        SortableItem {
                            key: row.clone(), value: card.clone(),
                            style: "height: 30px; flex-shrink: 0; font-size: 12px; line-height: 14px",
                            {card.clone()}
                        }
                    }
                }
            }
        }
    });
    let board = BOARD.with(|b| b.get()).unwrap();
    Fixture { app, board }
}

/// Whether document `doc` has a pending focus request.
fn pending_focus(doc: u64) -> usize {
    const FOCUS: &str = "rinch_components::sortable::PENDING_FOCUS";
    rinch_core::doc_entries(doc)
        .into_iter()
        .find(|(n, _)| *n == FOCUS)
        .map_or(0, |(_, count)| count)
}

/// The next turns of the event loop: the timers fire, then the runtime
/// applies the focus they asked for.
fn next_turn(f: &mut Fixture) {
    let due = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for id in due {
        rinch_core::fire_timeout(id);
    }
    send(f, PlatformEvent::AboutToWait);
    send(f, PlatformEvent::AboutToWait);
}

fn click_card(f: &mut Fixture, k: &str) {
    let at = point(f, item(f, k), 0.5, 0.5);
    press_at(f, at);
    release_at(f, at);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(f, k)));
}

/// Lift `a` by keyboard and drop it after `b`, the app applying nothing yet,
/// and let a turn of the event loop go by: `a`'s node.
fn drop_a_after_b(f: &mut Fixture) -> u64 {
    click_card(f, "a");
    let a = item(f, "a");
    key(f, KeyCode::Space, Some(" "));
    key(f, KeyCode::ArrowDown, None);
    key(f, KeyCode::Space, Some(" "));
    assert_eq!(DROPS.with(|d| d.borrow().len()), 1, "control: dropped");
    assert_eq!(cards(f, "c1"), ["a", "b", "c", "d"], "control: not applied");
    next_turn(f);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(a),
        "control: a holds the focus"
    );
    a
}

/// The app applies the drop it recorded.
fn apply_drop(f: &mut Fixture) {
    let m = DROPS.with(|d| d.borrow()[0].clone());
    f.board.update(|b| apply(b, &m));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(cards(f, "c1"), ["b", "a", "c", "d"], "control: applied");
}

/// The list's cards keyed by position: the drop, applied after a timer,
/// draws `a` again as a new node in the same list, and that node takes the
/// focus. The timer must not have ended the request on the old node, which
/// was still where it had been lifted.
#[test]
fn a_drop_applied_later_by_an_unkeyed_redraw_gives_the_new_node_the_focus() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    let mut f = list_fixture(Rows::ByIndex);
    let doc = f.app.doc_key();
    let old = drop_a_after_b(&mut f);
    assert_eq!(
        pending_focus(doc),
        1,
        "the request ended on the node a was lifted as"
    );
    apply_drop(&mut f);
    assert_ne!(item(&f, "a"), old, "control: a was drawn again");
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0, "a satisfied request lingers");
}

/// The whole list drawn again (its row in the app's keyed `for` changed),
/// after a timer: the new list's `a` takes the focus.
#[test]
fn a_drop_applied_later_by_a_full_rerender_gives_the_new_node_the_focus() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    let mut f = fixture(Options {
        apply: false,
        ..Options::default()
    });
    let doc = f.app.doc_key();
    let old_list = list(&f, "c1");
    let old = drop_a_after_b(&mut f);
    assert_eq!(
        pending_focus(doc),
        1,
        "the request ended on the node a was lifted as"
    );
    apply_drop(&mut f);
    assert_ne!(list(&f, "c1"), old_list, "control: c1 was drawn again");
    assert_ne!(item(&f, "a"), old);
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0, "a satisfied request lingers");
}

/// The control: cards keyed by card. Applied later, the drop moves the nodes
/// (`a` keeps its own), and the request ends once `a` is where the drop put
/// it, whichever nodes the reconcile moved, with the focus on `a`.
#[test]
fn a_drop_applied_later_by_a_keyed_move_ends_the_request() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    let mut f = list_fixture(Rows::ByCard);
    let doc = f.app.doc_key();
    let old = drop_a_after_b(&mut f);
    apply_drop(&mut f);
    assert_eq!(item(&f, "a"), old, "control: a kept its node");
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(old));
    assert_eq!(pending_focus(doc), 0, "the request outlived the move");
}

/// The focus moving on from the lifted card to another element drops the
/// request: the drop applied after that must not take the focus back.
#[test]
fn the_focus_moving_elsewhere_drops_the_request() {
    let _guard = crate::app::timer_backend_lock();
    rinch_core::set_timer_backend(manual_timers);
    SCHEDULED.with(|s| s.borrow_mut().clear());
    let mut f = list_fixture(Rows::ByIndex);
    let doc = f.app.doc_key();
    drop_a_after_b(&mut f);
    click_card(&mut f, "d");
    assert_eq!(pending_focus(doc), 0, "the request outlived the focus");
    apply_drop(&mut f);
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "d")));
}
