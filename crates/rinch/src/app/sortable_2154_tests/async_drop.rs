//! A keyboard drop the app applies later (#3062 review 4): after an await or
//! a timer of its own. Until then the node the card was lifted as is still
//! where it was, holding the focus, and must not end the focus request; the
//! redraw that applies the drop gives the moved card the focus.
//!
//! And (#3071 review 5) only that redraw: the request waits for the list's
//! order to show the drop applied, every carried card in place, so a redraw
//! for something else does not end it; and it ends when the focus falls to
//! blank space while the lifted card is still drawn.

use super::*;
use rinch_components::SortKeys;

thread_local! {
    static SCHEDULED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// The cards a drag from one of them carries together.
    static CARRY: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The app's "saving" flag: a `Rows::ByIndex` list draws every row again
    /// when it changes, in the same order.
    static SAVING: Cell<Option<Signal<bool>>> = const { Cell::new(None) };
}

/// `CARRY` when `key` is one of them, else the card alone.
fn carry_for(key: &str) -> Vec<String> {
    let carry = CARRY.with(|c| c.borrow().clone());
    if carry.iter().any(|k| k == key) {
        carry
    } else {
        Vec::new()
    }
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
    list_fixture_carrying(rows, &[])
}

/// [`list_fixture`], a drag from any of `carry` carrying all of them.
fn list_fixture_carrying(rows: Rows, carry: &[&str]) -> Fixture {
    DROPS.with(|d| d.borrow_mut().clear());
    CARRY.with(|c| *c.borrow_mut() = carry.iter().map(|k| k.to_string()).collect());
    let app = mount(move |__scope: &mut RenderScope| {
        let board: Signal<Board> = Signal::new(vec![(
            "c1".to_string(),
            ["a", "b", "c", "d"].map(String::from).to_vec(),
        )]);
        BOARD.with(|b| b.set(Some(board)));
        let saving = Signal::new(false);
        SAVING.with(|s| s.set(Some(saving)));
        // Each row as (its key, its card): the row is drawn again when either
        // changes.
        let rows = move || {
            let cards = board.get().into_iter().next().unwrap().1;
            let mark = if saving.get() { "s" } else { "" };
            cards
                .into_iter()
                .enumerate()
                .map(|(i, card)| match rows {
                    Rows::ByIndex => (format!("{i}{mark}"), card),
                    Rows::ByCard => (card.clone(), card),
                })
                .collect::<Vec<_>>()
        };
        rsx! {
            div { style: "width: 800px; height: 600px; padding: 20px",
                SortableList {
                    keyboard: true, group: "cards", id: "c1",
                    style: "--rinch-sortable-gap: 10px; width: 150px",
                    drag_keys: Some(Rc::new(carry_for) as SortKeys),
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
    apply_drop_as(f, &["b", "a", "c", "d"]);
}

/// The app applies the drop it recorded, which leaves the list `expect`.
fn apply_drop_as(f: &mut Fixture, expect: &[&str]) {
    let m = DROPS.with(|d| d.borrow()[0].clone());
    f.board.update(|b| apply(b, &m));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(cards(f, "c1"), expect, "control: applied");
}

fn manual_timer_setup() {
    rinch_core::set_timer_backend(manual_timers);
    SCHEDULED.with(|s| s.borrow_mut().clear());
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

/// Two cards carried (#3071 review 5): [a b c d] with a and c, dropped at
/// the top, asks for [a c b d]. `a` is at the top already, so its node there
/// is not the drop applied: the request must wait for the order, and the
/// redraw that applies it later, drawing every row again (the "saving" flag
/// goes with it), gives the new `a` the focus.
#[test]
fn a_multi_card_drop_waits_for_every_carried_card() {
    let _guard = crate::app::timer_backend_lock();
    manual_timer_setup();
    let mut f = list_fixture_carrying(Rows::ByIndex, &["a", "c"]);
    let doc = f.app.doc_key();
    click_card(&mut f, "a");
    let old = item(&f, "a");
    key(&mut f, KeyCode::Space, Some(" "));
    key(&mut f, KeyCode::ArrowUp, None);
    key(&mut f, KeyCode::Space, Some(" "));
    let m = DROPS
        .with(|d| d.borrow().first().cloned())
        .expect("control: dropped");
    assert_eq!(m.keys, ["a", "c"], "control: both carried");
    assert_eq!(m.place.index, 0, "control: dropped at the top");
    assert_eq!(
        cards(&f, "c1"),
        ["a", "b", "c", "d"],
        "control: not applied"
    );
    next_turn(&mut f);
    assert_eq!(
        pending_focus(doc),
        1,
        "the request ended on a's old node, at the drop's index already"
    );
    SAVING.with(|s| s.get()).unwrap().set(true);
    apply_drop_as(&mut f, &["a", "c", "b", "d"]);
    assert_ne!(item(&f, "a"), old, "control: a was drawn again");
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0, "a satisfied request lingers");
}

/// A redraw for something else (#3071 review 5): the app's "saving" flag
/// draws every row again by position before it applies the drop, so `a` is a
/// new node at its old place. That is not the drop applied: the request
/// waits, and the redraw that applies the drop gives `a` the focus.
#[test]
fn a_redraw_in_the_old_order_does_not_end_the_request() {
    let _guard = crate::app::timer_backend_lock();
    manual_timer_setup();
    let mut f = list_fixture(Rows::ByIndex);
    let doc = f.app.doc_key();
    let old = drop_a_after_b(&mut f);
    SAVING.with(|s| s.get()).unwrap().set(true);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_ne!(item(&f, "a"), old, "control: a was drawn again");
    assert_eq!(
        cards(&f, "c1"),
        ["a", "b", "c", "d"],
        "control: not applied"
    );
    next_turn(&mut f);
    assert_eq!(
        pending_focus(doc),
        1,
        "a redraw in the old order ended the request"
    );
    apply_drop(&mut f);
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0, "a satisfied request lingers");
}

/// The bound on waiting for the order: an app that applies the drop its own
/// way (here `a` goes last, not after `b`) ends the request on `a`, wherever
/// it put it, rather than leave it waiting for an order that will not come.
#[test]
fn a_drop_applied_another_way_ends_the_request_on_the_card() {
    let _guard = crate::app::timer_backend_lock();
    manual_timer_setup();
    let mut f = list_fixture(Rows::ByIndex);
    let doc = f.app.doc_key();
    drop_a_after_b(&mut f);
    f.board.update(|b| {
        let cards = &mut b[0].1;
        cards.retain(|k| k != "a");
        cards.push("a".to_string());
    });
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(cards(&f, "c1"), ["b", "c", "d", "a"], "control: applied");
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "a")));
    assert_eq!(pending_focus(doc), 0, "the request outlived another order");
}

/// The focus falling to blank space (#3071 review 5) while the lifted card
/// is still drawn drops the request, as the focus moving to another element
/// does: the drop applied after that must not take the focus back. (Desktop
/// makes the clicked element the document's active element; the focus
/// falling to `<body>`, a browser's answer, is `focus_tests.rs`'s
/// `the_focus_falling_to_the_body_drops_a_drop_request`.)
#[test]
fn the_focus_falling_to_blank_space_drops_the_request() {
    let _guard = crate::app::timer_backend_lock();
    manual_timer_setup();
    let mut f = list_fixture(Rows::ByIndex);
    let doc = f.app.doc_key();
    drop_a_after_b(&mut f);
    press_at(&mut f, (600.0, 400.0));
    release_at(&mut f, (600.0, 400.0));
    assert_eq!(f.app.focus_target, FocusTarget::None, "control: blurred");
    assert_eq!(pending_focus(doc), 0, "the request outlived the blur");
    apply_drop(&mut f);
    next_turn(&mut f);
    assert_eq!(f.app.focus_target, FocusTarget::None);
}
