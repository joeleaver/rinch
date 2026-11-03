//! Sortable lists (#2091), driven through the real event path: presses,
//! moves, releases, `AboutToWait` frames and key presses through
//! `handle_event`, on a three-level kanban — a vertical list of lanes, a
//! horizontal list of columns per lane, a vertical list of cards per column —
//! whose `ondrop` applies the move to the app's own data, which the keyed
//! `for`s re-render.
//!
//! The last module drives the same board through the element handler
//! attributes alone, as a plugin host that proxies `data-*` handlers does.

use super::*;
use crate as rinch;
use std::cell::{Cell, RefCell};

use rinch_components::{
    SortAxis, SortJudge, SortKeys, SortMove, SortPlace, SortVerdict, SortableHandle, SortableItem,
    SortableList,
};
use rinch_core::Signal;
use rinch_core::element::IntoEventHandler;
use rinch_macros::rsx;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

#[derive(Clone, Debug, PartialEq)]
struct Col {
    id: String,
    cards: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Lane {
    id: String,
    cols: Vec<Col>,
}

type Offer = Option<(SortMove, SortVerdict)>;

thread_local! {
    static BOARD: Cell<Option<Signal<Vec<Lane>>>> = const { Cell::new(None) };
    static DROPS: RefCell<Vec<SortMove>> = const { RefCell::new(Vec::new()) };
    static OFFERS: RefCell<Vec<Offer>> = const { RefCell::new(Vec::new()) };
    /// The cards a drag carries together (the app's selection).
    static SELECTION: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Whether `ondrop` applies the move at once (else the test does).
    static APPLY: Cell<bool> = const { Cell::new(true) };
    static SCHEDULED: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
}

fn manual_timers(id: u64, delay_ms: u32) {
    SCHEDULED.with(|s| s.borrow_mut().push((id, delay_ms)));
}

fn col(id: &str, cards: &[&str]) -> Col {
    Col {
        id: id.into(),
        cards: cards.iter().map(|c| c.to_string()).collect(),
    }
}

/// Lane l1: c1 [a b c i j] (taller than its list: it scrolls), c2 [d],
/// c3 [e f g] (a WIP limit of 3), c4 [] — four 200px columns in a 500px row,
/// which scrolls sideways. Lane l2: c5 [h], c6 [].
fn initial() -> Vec<Lane> {
    vec![
        Lane {
            id: "l1".into(),
            cols: vec![
                col("c1", &["a", "b", "c", "i", "j"]),
                col("c2", &["d"]),
                col("c3", &["e", "f", "g"]),
                col("c4", &[]),
            ],
        },
        Lane {
            id: "l2".into(),
            cols: vec![col("c5", &["h"]), col("c6", &[])],
        },
    ]
}

/// Put `moved` (taken out of wherever they are) at `place` among `list`'s
/// items, by the place's keys: after `after`, else first.
fn place_by_keys<T>(list: &mut Vec<T>, moved: Vec<T>, place: &SortPlace, id: impl Fn(&T) -> &str) {
    let at = place
        .after
        .as_ref()
        .and_then(|a| list.iter().position(|t| id(t) == a))
        .map_or(0, |i| i + 1);
    for (i, t) in moved.into_iter().enumerate() {
        list.insert(at + i, t);
    }
}

fn apply(board: &mut Vec<Lane>, m: &SortMove) {
    match m.group.as_str() {
        "cards" => {
            for lane in board.iter_mut() {
                for c in lane.cols.iter_mut() {
                    c.cards.retain(|k| !m.keys.contains(k));
                }
            }
            let target = board
                .iter_mut()
                .flat_map(|l| l.cols.iter_mut())
                .find(|c| c.id == m.to_list)
                .unwrap();
            place_by_keys(&mut target.cards, m.keys.clone(), &m.place, |k| k);
        }
        "columns" => {
            let mut moved = Vec::new();
            for key in &m.keys {
                for lane in board.iter_mut() {
                    if let Some(i) = lane.cols.iter().position(|c| &c.id == key) {
                        moved.push(lane.cols.remove(i));
                    }
                }
            }
            let lane = board.iter_mut().find(|l| l.id == m.to_list).unwrap();
            place_by_keys(&mut lane.cols, moved, &m.place, |c| &c.id);
        }
        "lanes" => {
            let mut moved = Vec::new();
            for key in &m.keys {
                if let Some(i) = board.iter().position(|l| &l.id == key) {
                    moved.push(board.remove(i));
                }
            }
            place_by_keys(board, moved, &m.place, |l| &l.id);
        }
        other => panic!("unknown group {other}"),
    }
}

fn judge_cards(m: &SortMove) -> SortVerdict {
    if m.to_list == "c3" && m.from_list != "c3" {
        SortVerdict::Refuse("WIP limit 3".into())
    } else {
        SortVerdict::Accept
    }
}

fn selection_for(key: &str) -> Vec<String> {
    let selection = SELECTION.with(|s| s.borrow().clone());
    if selection.iter().any(|k| k == key) {
        selection
    } else {
        Vec::new()
    }
}

fn on_drop(board: Signal<Vec<Lane>>, m: SortMove) {
    DROPS.with(|d| d.borrow_mut().push(m.clone()));
    if APPLY.with(Cell::get) {
        board.update(|b| apply(b, &m));
    }
}

fn on_offer(o: Offer) {
    OFFERS.with(|v| v.borrow_mut().push(o));
}

struct Fixture {
    app: RinchApp,
    board: Signal<Vec<Lane>>,
}

fn fixture() -> Fixture {
    DROPS.with(|d| d.borrow_mut().clear());
    OFFERS.with(|d| d.borrow_mut().clear());
    SELECTION.with(|d| d.borrow_mut().clear());
    APPLY.with(|a| a.set(true));
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let board = Signal::new(initial());
        BOARD.with(|b| b.set(Some(board)));
        rsx! {
            div { style: "width: 800px; height: 600px",
                SortableList {
                    group: "lanes", id: "board", axis: SortAxis::Vertical, label: "Board",
                    style: "--rinch-sortable-gap: 0px",
                    ondrop: move |m: SortMove| on_drop(board, m),
                    ondropchange: move |o: Offer| on_offer(o),
                    for lane in board.get() {
                        SortableItem {
                            key: lane.id.clone(), value: lane.id.clone(), handle: true,
                            SortableHandle { style: "height: 20px" }
                            SortableList {
                                group: "columns", id: lane.id.clone(), axis: SortAxis::Horizontal,
                                style: "--rinch-sortable-gap: 0px; width: 500px; overflow-x: auto",
                                ondrop: move |m: SortMove| on_drop(board, m),
                                ondropchange: move |o: Offer| on_offer(o),
                                for c in lane.cols.clone() {
                                    SortableItem {
                                        key: c.id.clone(), value: c.id.clone(), handle: true,
                                        style: "width: 200px; flex-shrink: 0",
                                        SortableHandle { style: "height: 20px" }
                                        SortableList {
                                            group: "cards", id: c.id.clone(), label: c.id.clone(),
                                            style: "--rinch-sortable-gap: 0px; height: 120px; overflow-y: auto; padding-top: 6px",
                                            judge: Some(Rc::new(judge_cards) as SortJudge),
                                            drag_keys: Some(Rc::new(selection_for) as SortKeys),
                                            ondrop: move |m: SortMove| on_drop(board, m),
                                            ondropchange: move |o: Offer| on_offer(o),
                                            for card in c.cards.clone() {
                                                SortableItem {
                                                    key: card.clone(), value: card.clone(),
                                                    style: "height: 30px; flex-shrink: 0",
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let board = BOARD.with(|b| b.get()).unwrap();
    Fixture { app, board }
}

fn send(f: &mut Fixture, event: PlatformEvent) {
    f.app
        .handle_event(event, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

/// The node of the item keyed `key`.
fn item(f: &Fixture, key: &str) -> u64 {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let found: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(id, n)| {
            n.attributes.get("data-sortable-key").map(String::as_str) == Some(key)
                && connected(&d, *id)
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(found.len(), 1, "one item keyed {key}");
    found[0]
}

fn connected(d: &RinchDocument, mut id: u64) -> bool {
    loop {
        if id == d.tree.body_id {
            return true;
        }
        match d.tree.nodes.get(id).and_then(|n| n.parent) {
            Some(p) => id = p,
            None => return false,
        }
    }
}

/// The list whose `data-sortable-group` is `group` and that holds `key` (or
/// is the `n`th of its group when `key` is empty).
fn list_holding(f: &Fixture, key: &str) -> u64 {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut at = d.tree.nodes[item_in(&d, key)].parent;
    while let Some(id) = at {
        if d.tree.nodes[id]
            .attributes
            .contains_key("data-sortable-list")
        {
            return id;
        }
        at = d.tree.nodes[id].parent;
    }
    panic!("no list holds {key}")
}

fn item_in(d: &RinchDocument, key: &str) -> u64 {
    d.tree
        .nodes
        .iter()
        .find(|(id, n)| {
            n.attributes.get("data-sortable-key").map(String::as_str) == Some(key)
                && connected(d, *id)
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn box_of(f: &Fixture, id: u64) -> (f32, f32, f32, f32) {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    hit_testing::painted_element_box(&d.tree, id)
}

/// A point `fx`, `fy` of the way across and down `id`'s box.
fn point(f: &Fixture, id: u64, fx: f32, fy: f32) -> (f32, f32) {
    let (x, y, w, h) = box_of(f, id);
    (x + w * fx, y + h * fy)
}

fn classes(f: &Fixture, id: u64) -> String {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id]
        .attributes
        .get("class")
        .cloned()
        .unwrap_or_default()
}

/// Every connected node carrying `class`.
fn carrying(f: &Fixture, class: &str) -> Vec<u64> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(id, n)| has_class(n, class) && connected(&d, *id))
        .map(|(id, _)| id)
        .collect()
}

fn press_at(f: &mut Fixture, (x, y): (f32, f32)) {
    send(
        f,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

fn move_to(f: &mut Fixture, (x, y): (f32, f32)) {
    send(f, PlatformEvent::MouseMove { x, y });
}

fn release_at(f: &mut Fixture, (x, y): (f32, f32)) {
    send(
        f,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

/// Press at `at` and move past the threshold: the drag goes live.
fn start_at(f: &mut Fixture, at: (f32, f32)) {
    press_at(f, at);
    move_to(f, (at.0 + 8.0, at.1 + 1.0));
    assert!(f.app.active_dnd.is_some(), "the drag went live");
}

/// Start dragging the card `key` from its middle.
fn start_card(f: &mut Fixture, key: &str) {
    let at = point(f, item(f, key), 0.5, 0.5);
    start_at(f, at);
}

/// The handle of the item keyed `key` (its first child).
fn handle_of(f: &Fixture, key: &str) -> u64 {
    let id = item(f, key);
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id].children[0]
}

fn last_offer(f: &Fixture) -> Offer {
    let _ = f;
    OFFERS.with(|o| o.borrow().last().cloned().flatten())
}

fn drops() -> Vec<SortMove> {
    DROPS.with(|d| d.borrow().clone())
}

fn cards(f: &Fixture, column: &str) -> Vec<String> {
    f.board
        .get()
        .into_iter()
        .flat_map(|l| l.cols)
        .find(|c| c.id == column)
        .unwrap()
        .cards
}

fn place(after: Option<&str>, before: Option<&str>, index: usize) -> SortPlace {
    SortPlace {
        after: after.map(str::to_string),
        before: before.map(str::to_string),
        index,
    }
}

fn mv(group: &str, keys: &[&str], from: &str, to: &str, place: SortPlace) -> SortMove {
    SortMove {
        group: group.into(),
        keys: keys.iter().map(|k| k.to_string()).collect(),
        from_list: from.into(),
        to_list: to.into(),
        place,
    }
}

/// Nothing of a drag is left drawn.
fn assert_clean(f: &Fixture) {
    for class in [
        "rinch-sortable__item--dragging",
        "rinch-sortable__item--lifted",
        "rinch-sortable__item--drop-before",
        "rinch-sortable__item--drop-after",
        "rinch-sortable__item--refused",
        "rinch-sortable--drop-into",
        "rinch-sortable__reason--shown",
    ] {
        assert!(carrying(f, class).is_empty(), "{class} left drawn");
    }
}

#[test]
fn a_card_moves_within_its_column() {
    let mut f = fixture();
    start_card(&mut f, "a");
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--dragging"));

    // Below the middle of c: after it.
    let at = point(&f, item(&f, "c"), 0.5, 0.7);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c1", place(Some("c"), Some("i"), 2));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-after"),
        vec![item(&f, "c")],
        "the caret is drawn after c, and only there"
    );

    // Above the middle of c: before it, which is after b.
    let above = point(&f, item(&f, "c"), 0.5, 0.3);
    move_to(&mut f, above);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    move_to(&mut f, at);

    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "i", "j"]);
    assert_eq!(OFFERS.with(|o| o.borrow().last().cloned()), Some(None));
    assert_clean(&f);
}

#[test]
fn a_card_over_its_own_place_offers_nothing() {
    let mut f = fixture();
    start_card(&mut f, "b");
    // The bottom of a: after a, which is where b is.
    let at = point(&f, item(&f, "a"), 0.5, 0.8);
    move_to(&mut f, at);
    assert_eq!(last_offer(&f), None);
    release_at(&mut f, at);
    assert!(drops().is_empty());
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "i", "j"]);
}

#[test]
fn a_card_moves_to_another_column() {
    let mut f = fixture();
    start_card(&mut f, "b");
    let at = point(&f, item(&f, "d"), 0.5, 0.25);
    move_to(&mut f, at);
    let expected = mv("cards", &["b"], "c1", "c2", place(None, Some("d"), 0));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-before"),
        vec![item(&f, "d")]
    );
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["a", "c", "i", "j"]);
    assert_eq!(cards(&f, "c2"), ["b", "d"]);
}

#[test]
fn over_a_list_between_its_cards_the_nearest_gap_is_offered() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let c2 = list_holding(&f, "d");
    let (lx, ly, lw, _) = box_of(&f, c2);
    let (_, dy, _, _) = box_of(&f, item(&f, "d"));
    assert!(dy >= ly + 6.0, "the list's padding is above d");
    // In the padding above d: first.
    move_to(&mut f, (lx + lw / 2.0, ly + 3.0));
    assert_eq!(last_offer(&f).unwrap().0.place, place(None, Some("d"), 0));
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-before"),
        vec![item(&f, "d")]
    );
    // Below d, in the list's empty space: last.
    move_to(&mut f, (lx + lw / 2.0, dy + 60.0));
    assert_eq!(last_offer(&f).unwrap().0.place, place(Some("d"), None, 1));
}

#[test]
fn a_card_drops_into_an_empty_column_of_another_lane() {
    let mut f = fixture();
    start_card(&mut f, "a");
    // c6 is empty: the pointer is over its card list itself.
    let c6_list = {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .find(|(id, n)| {
                n.attributes.get("data-sortable-group").map(String::as_str) == Some("cards")
                    && connected(&d, *id)
                    && d.tree.nodes[n.parent.unwrap()]
                        .attributes
                        .get("data-sortable-key")
                        .map(String::as_str)
                        == Some("c6")
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let at = point(&f, c6_list, 0.5, 0.6);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c6", place(None, None, 0));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert!(expected.place.is_first() && expected.place.is_last());
    assert!(classes(&f, c6_list).contains("rinch-sortable--drop-into"));
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c6"), ["a"]);
}

#[test]
fn a_refused_drop_shows_its_reason_before_the_release_and_is_not_delivered() {
    let mut f = fixture();
    start_card(&mut f, "a");
    // f in c3, whose WIP limit refuses cards from elsewhere. c3 starts at
    // x 400; the row shows up to 500.
    let (fx, _, _, _) = box_of(&f, item(&f, "f"));
    let (_, fy, _, fh) = box_of(&f, item(&f, "f"));
    let at = (fx + 20.0, fy + fh * 0.75);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c3", place(Some("f"), Some("g"), 2));
    assert_eq!(
        last_offer(&f),
        Some((expected, SortVerdict::Refuse("WIP limit 3".into())))
    );
    let f_item = item(&f, "f");
    let class = classes(&f, f_item);
    assert!(
        class.contains("rinch-sortable__item--drop-after"),
        "{class}"
    );
    assert!(class.contains("rinch-sortable__item--refused"), "{class}");
    // The reason is on screen, beside the caret, before any release.
    let shown = carrying(&f, "rinch-sortable__reason--shown");
    assert_eq!(shown.len(), 1);
    {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert_eq!(d.tree.nodes[shown[0]].parent, Some(f_item));
        let text: String = d.tree.nodes[shown[0]]
            .children
            .iter()
            .filter_map(|c| d.tree.nodes[*c].text_content())
            .collect();
        assert_eq!(text, "WIP limit 3");
    }
    let (_, ry, rw, rh) = box_of(&f, shown[0]);
    assert!(rw > 0.0 && rh > 0.0, "the reason has a box");
    // At the caret's edge of f (its bottom), inside f's box: a box hanging
    // below it would grow the scrolling list's overflow.
    assert!(
        ry + rh <= fy + fh && ry + rh > fy + fh / 2.0,
        "at f's bottom edge, inside it: {ry}+{rh} vs {fy}+{fh}"
    );

    release_at(&mut f, at);
    assert!(drops().is_empty(), "a refused drop is not delivered");
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "i", "j"]);
    assert_eq!(cards(&f, "c3"), ["e", "f", "g"]);
    assert_clean(&f);
}

#[test]
fn the_place_is_by_key_so_a_reorder_before_the_app_applies_it_does_not_move_it() {
    let mut f = fixture();
    APPLY.with(|a| a.set(false));
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, at);
    release_at(&mut f, at);
    let delivered = drops().pop().expect("delivered");
    assert_eq!(delivered.place, place(Some("c"), Some("i"), 2));
    // Before the app applies it, a collaborator puts z at the top of c1 and
    // takes b out.
    f.board.update(|b| {
        let c1 = &mut b[0].cols[0].cards;
        c1.insert(0, "z".into());
        c1.retain(|k| k != "b");
    });
    f.board.update(|b| apply(b, &delivered));
    assert_eq!(
        cards(&f, "c1"),
        ["z", "c", "a", "i", "j"],
        "still right after c; index 2 would have put it before c"
    );
}

#[test]
fn a_drag_survives_a_rerender_of_its_list_and_offers_from_the_new_rows() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, at);
    // A collaborator's edit re-renders the lane under the drag: every node
    // of it, the dragged card's included, is new.
    let old_a = item(&f, "a");
    f.board.update(|b| b[0].cols[0].cards.push("k".into()));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_ne!(item(&f, "a"), old_a, "the card was re-rendered");
    // The next move offers from the new rows.
    let at = point(&f, item(&f, "b"), 0.5, 0.75);
    move_to(&mut f, at);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-after"),
        vec![item(&f, "b")]
    );
    release_at(&mut f, at);
    assert_eq!(cards(&f, "c1"), ["b", "a", "c", "i", "j", "k"]);
    assert_clean(&f);
}

#[test]
fn several_selected_cards_move_together_in_order() {
    let mut f = fixture();
    SELECTION.with(|s| *s.borrow_mut() = vec!["a".into(), "c".into()]);
    start_card(&mut f, "c");
    let dimmed = carrying(&f, "rinch-sortable__item--dragging");
    assert_eq!(dimmed.len(), 2);
    assert!(dimmed.contains(&item(&f, "a")) && dimmed.contains(&item(&f, "c")));
    let at = point(&f, item(&f, "d"), 0.5, 0.75);
    move_to(&mut f, at);
    let expected = mv("cards", &["a", "c"], "c1", "c2", place(Some("d"), None, 1));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["b", "i", "j"]);
    assert_eq!(cards(&f, "c2"), ["d", "a", "c"]);
}

mod nesting {
    //! A drag targets only lists of its own group, at every level.
    use super::*;

    fn columns(f: &Fixture, lane: &str) -> Vec<String> {
        f.board
            .get()
            .into_iter()
            .find(|l| l.id == lane)
            .unwrap()
            .cols
            .into_iter()
            .map(|c| c.id)
            .collect()
    }

    #[test]
    fn a_card_drags_alone_and_not_its_column() {
        let mut f = fixture();
        start_card(&mut f, "b");
        assert_eq!(
            carrying(&f, "rinch-sortable__item--dragging"),
            vec![item(&f, "b")]
        );
        // Over its own column's header: the columns are not a card's place.
        let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
        move_to(&mut f, header);
        assert_eq!(last_offer(&f), None);
        release_at(&mut f, header);
        assert!(drops().is_empty());
    }

    #[test]
    fn a_column_drags_by_its_header_along_the_row() {
        let mut f = fixture();
        let header = point(&f, handle_of(&f, "c1"), 0.5, 0.5);
        start_at(&mut f, header);
        assert_eq!(
            carrying(&f, "rinch-sortable__item--dragging"),
            vec![item(&f, "c1")]
        );
        // The right part of c2's header: after c2.
        let at = point(&f, handle_of(&f, "c2"), 0.8, 0.5);
        move_to(&mut f, at);
        let expected = mv(
            "columns",
            &["c1"],
            "l1",
            "l1",
            place(Some("c2"), Some("c3"), 1),
        );
        assert_eq!(
            last_offer(&f),
            Some((expected.clone(), SortVerdict::Accept))
        );
        let class = classes(&f, item(&f, "c2"));
        assert!(
            class.contains("rinch-sortable__item--drop-after")
                && class.contains("rinch-sortable__item--drop-horizontal"),
            "{class}"
        );
        release_at(&mut f, at);
        assert_eq!(drops(), vec![expected]);
        assert_eq!(columns(&f, "l1"), ["c2", "c1", "c3", "c4"]);
    }

    #[test]
    fn a_column_dragged_over_cards_is_placed_among_the_columns() {
        let mut f = fixture();
        let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
        start_at(&mut f, header);
        // Over card b, in the left half of column c1: before c1.
        let at = point(&f, item(&f, "b"), 0.2, 0.5);
        move_to(&mut f, at);
        assert_eq!(
            last_offer(&f).map(|(m, _)| (m.group, m.place)),
            Some(("columns".to_string(), place(None, Some("c1"), 0)))
        );
        assert!(
            carrying(&f, "rinch-sortable__item--drop-before") == vec![item(&f, "c1")],
            "drawn on the column, not the card"
        );
        release_at(&mut f, at);
        assert_eq!(columns(&f, "l1"), ["c2", "c1", "c3", "c4"]);
    }

    #[test]
    fn a_column_moves_to_another_lane() {
        let mut f = fixture();
        let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
        start_at(&mut f, header);
        let at = point(&f, handle_of(&f, "c6"), 0.7, 0.5);
        move_to(&mut f, at);
        assert_eq!(
            last_offer(&f).map(|(m, _)| (m.to_list, m.place)),
            Some(("l2".to_string(), place(Some("c6"), None, 2)))
        );
        release_at(&mut f, at);
        assert_eq!(columns(&f, "l1"), ["c1", "c3", "c4"]);
        assert_eq!(columns(&f, "l2"), ["c5", "c6", "c2"]);
        assert_eq!(cards(&f, "c2"), ["d"], "its cards went with it");
    }

    #[test]
    fn a_lane_drags_by_its_handle() {
        let mut f = fixture();
        let handle = point(&f, handle_of(&f, "l2"), 0.5, 0.5);
        start_at(&mut f, handle);
        // The top of lane l1's handle: before l1.
        let at = point(&f, handle_of(&f, "l1"), 0.5, 0.2);
        move_to(&mut f, at);
        assert_eq!(
            last_offer(&f).map(|(m, _)| (m.group, m.place)),
            Some(("lanes".to_string(), place(None, Some("l1"), 0)))
        );
        release_at(&mut f, at);
        let order: Vec<String> = f.board.get().into_iter().map(|l| l.id).collect();
        assert_eq!(order, ["l2", "l1"]);
    }

    #[test]
    fn a_press_on_a_card_that_never_passes_the_threshold_is_a_click() {
        let mut f = fixture();
        let at = point(&f, item(&f, "b"), 0.5, 0.5);
        press_at(&mut f, at);
        move_to(&mut f, (at.0 + 2.0, at.1));
        release_at(&mut f, (at.0 + 2.0, at.1));
        assert!(f.app.active_dnd.is_none());
        assert!(OFFERS.with(|o| o.borrow().is_empty()));
        // The click focused the card (it is focusable), as a click does.
        assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "b")));
    }
}

mod cancel {
    use super::*;

    fn hovering_after_c(f: &mut Fixture) -> (f32, f32) {
        start_card(f, "a");
        let at = point(f, item(f, "c"), 0.5, 0.75);
        move_to(f, at);
        assert!(last_offer(f).is_some());
        at
    }

    #[test]
    fn escape_cancels_a_pointer_drag() {
        let mut f = fixture();
        let at = hovering_after_c(&mut f);
        send(
            &mut f,
            PlatformEvent::KeyDown {
                key: KeyCode::Escape,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
        );
        assert!(f.app.active_dnd.is_none());
        assert_eq!(OFFERS.with(|o| o.borrow().last().cloned()), Some(None));
        assert_clean(&f);
        release_at(&mut f, at);
        assert!(drops().is_empty());
    }

    #[test]
    fn a_missed_release_heals_without_dropping() {
        let mut f = fixture();
        let at = hovering_after_c(&mut f);
        // The release went missing; the next left press proves it.
        press_at(&mut f, at);
        assert!(f.app.active_dnd.is_none());
        assert!(drops().is_empty(), "a healed drag drops nothing");
        assert_clean(&f);
        release_at(&mut f, at);
        assert!(drops().is_empty());
    }
}

mod auto_scroll {
    use super::*;
    use std::time::Duration;

    fn frames(f: &mut Fixture, n: usize) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_millis(5));
            send(f, PlatformEvent::AboutToWait);
        }
    }

    fn scroll_offset(f: &Fixture, id: u64) -> (f64, f64) {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let o = d.tree.nodes[id].scroll_offset;
        (o.0, o.1)
    }

    #[test]
    fn a_card_near_the_bottom_of_its_column_scrolls_the_column() {
        let mut f = fixture();
        let list = list_holding(&f, "a");
        start_card(&mut f, "b");
        let (lx, ly, lw, lh) = box_of(&f, list);
        move_to(&mut f, (lx + lw * 0.5, ly + lh - 3.0));
        assert_eq!(scroll_offset(&f, list), (0.0, 0.0));
        frames(&mut f, 3);
        let (x, y) = scroll_offset(&f, list);
        assert!(y > 0.0, "the column scrolled down: {y}");
        assert_eq!(x, 0.0);
    }

    #[test]
    fn a_column_near_the_right_edge_of_the_row_scrolls_the_row() {
        let mut f = fixture();
        let row = list_holding(&f, "c1");
        let header = point(&f, handle_of(&f, "c1"), 0.5, 0.5);
        start_at(&mut f, header);
        let (rx, _, rw, _) = box_of(&f, row);
        move_to(&mut f, (rx + rw - 3.0, header.1));
        frames(&mut f, 3);
        let (x, y) = scroll_offset(&f, row);
        assert!(x > 0.0, "the row scrolled right: {x}");
        assert_eq!(y, 0.0);
        // And back: the left edge.
        move_to(&mut f, (rx + 2.0, header.1));
        let before = scroll_offset(&f, row).0;
        frames(&mut f, 3);
        assert!(scroll_offset(&f, row).0 < before, "scrolled back left");
    }

    #[test]
    fn a_card_in_a_corner_scrolls_its_column_and_the_row_together() {
        let mut f = fixture();
        let row = list_holding(&f, "c1");
        let column = list_holding(&f, "e");
        start_card(&mut f, "a");
        // c3's list runs past the row's right edge at 500: the pointer at the
        // row's right edge and c3's list's bottom is in both bands.
        let (rx, _, rw, _) = box_of(&f, row);
        let (_, cy, _, ch) = box_of(&f, column);
        move_to(&mut f, (rx + rw - 3.0, cy + ch - 3.0));
        frames(&mut f, 3);
        assert!(scroll_offset(&f, row).0 > 0.0, "the row scrolled");
        assert!(
            scroll_offset(&f, column).1 == 0.0,
            "e f g fit their list, the refusal reason drawn inside g included"
        );
        // c1's list is the one that overflows vertically: take a card to
        // its bottom-left corner, by the row's left edge.
        let c1 = list_holding(&f, "b");
        let (_, by, _, bh) = box_of(&f, c1);
        move_to(&mut f, (rx + 2.0, by + bh - 3.0));
        frames(&mut f, 3);
        assert!(scroll_offset(&f, c1).1 > 0.0, "c1 scrolled down");
    }
}

mod keyboard {
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
}

mod handler_path {
    //! A plugin host applies the guest's DOM ops and proxies only its `data-*`
    //! handler attributes (Pimble's `wasm32-wasip1` plugins): no pointer
    //! capture, no runtime drag state, no dismiss stack. This drives a drag
    //! and a keyboard move by reading the attributes and dispatching the
    //! handlers they name, as such a host does.
    use super::*;
    use rinch_core::events::{self, ClickContext, EventHandlerId, KeyEventData};

    fn handler(f: &Fixture, id: u64, attr: &str) -> EventHandlerId {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let mut at = Some(id);
        while let Some(n) = at {
            if let Some(v) = d.tree.nodes[n].attributes.get(attr) {
                return EventHandlerId(v.parse().unwrap());
            }
            at = d.tree.nodes[n].parent;
        }
        panic!("no {attr} at or above {id}")
    }

    fn context(f: &Fixture, id: u64, (x, y): (f32, f32)) {
        let (ex, ey, ew, eh) = box_of(f, id);
        events::set_click_context(ClickContext {
            mouse_x: x,
            mouse_y: y,
            element_x: ex,
            element_y: ey,
            element_width: ew,
            element_height: eh,
            ..Default::default()
        });
    }

    #[test]
    fn a_drag_runs_on_the_element_handlers_alone() {
        let f = fixture();
        let a = item(&f, "a");
        events::dispatch_event(handler(&f, a, "data-ondragstart"));
        let d = item(&f, "d");
        let at = point(&f, d, 0.5, 0.75);
        context(&f, d, at);
        events::dispatch_event(handler(&f, d, "data-ondragover"));
        events::dispatch_event(handler(&f, a, "data-ondragmove"));
        let expected = mv("cards", &["a"], "c1", "c2", place(Some("d"), None, 1));
        assert_eq!(
            last_offer(&f),
            Some((expected.clone(), SortVerdict::Accept))
        );
        events::dispatch_event(handler(&f, d, "data-ondrop"));
        events::dispatch_event(handler(&f, d, "data-ondragleave"));
        events::dispatch_event(handler(&f, a, "data-ondragend"));
        assert_eq!(drops(), vec![expected]);
        assert!(f.app.active_dnd.is_none(), "the runtime's drag never ran");
    }

    #[test]
    fn a_keyboard_move_runs_on_the_key_handler_alone() {
        let f = fixture();
        let a = item(&f, "a");
        let keys = handler(&f, a, "data-onkeydown");
        let press = |k: &str, code: &str| {
            events::dispatch_key_handler(keys, &KeyEventData::new(k, code));
        };
        press(" ", "Space");
        press("ArrowDown", "ArrowDown");
        press("ArrowDown", "ArrowDown");
        assert_eq!(
            last_offer(&f).unwrap().0.place,
            place(Some("c"), Some("i"), 2)
        );
        press("Enter", "Enter");
        assert_eq!(
            drops(),
            vec![mv(
                "cards",
                &["a"],
                "c1",
                "c1",
                place(Some("c"), Some("i"), 2)
            )]
        );
        // Escape too is the element's: a second lift is cancelled by it.
        let a = item(&f, "a");
        let keys = handler(&f, a, "data-onkeydown");
        events::dispatch_key_handler(keys, &KeyEventData::new(" ", "Space"));
        assert!(classes(&f, a).contains("rinch-sortable__item--lifted"));
        events::dispatch_key_handler(keys, &KeyEventData::new("Escape", "Escape"));
        assert!(!classes(&f, a).contains("rinch-sortable__item--lifted"));
    }
}
