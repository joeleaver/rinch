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
    SortAxis, SortJudge, SortKeys, SortMove, SortPlace, SortSource, SortVerdict, SortableHandle,
    SortableItem, SortableList,
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
    /// The app's judge refuses everything (its state changed under a drag).
    static FROZEN: Cell<bool> = const { Cell::new(false) };
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
    if FROZEN.with(Cell::get) {
        return SortVerdict::Refuse("Board frozen".into());
    }
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
    fixture_with(None, false)
}

/// The board with the card column `plain` not a `keyboard` list, and with
/// `tabindex="0"` written by the app on every card when `card_tabindex`.
fn fixture_with(plain: Option<&'static str>, card_tabindex: bool) -> Fixture {
    DROPS.with(|d| d.borrow_mut().clear());
    OFFERS.with(|d| d.borrow_mut().clear());
    SELECTION.with(|d| d.borrow_mut().clear());
    APPLY.with(|a| a.set(true));
    FROZEN.with(|a| a.set(false));
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let board = Signal::new(initial());
        BOARD.with(|b| b.set(Some(board)));
        rsx! {
            div { style: "width: 800px; height: 600px",
                SortableList {
                    keyboard: true,
                    group: "lanes", id: "board", axis: SortAxis::Vertical, label: "Board",
                    style: "--rinch-sortable-gap: 0px",
                    ondrop: move |m: SortMove| on_drop(board, m),
                    ondropchange: move |o: Offer| on_offer(o),
                    // The carried items are told by their dimming (#2154: no
                    // longer the default).
                    source: SortSource::Dim,
                    for lane in board.get() {
                        SortableItem {
                            key: lane.id.clone(), value: lane.id.clone(), handle: true,
                            SortableHandle { style: "height: 20px" }
                            SortableList {
                                keyboard: true,
                                group: "columns", id: lane.id.clone(), axis: SortAxis::Horizontal,
                                style: "--rinch-sortable-gap: 0px; width: 500px; overflow-x: auto",
                                ondrop: move |m: SortMove| on_drop(board, m),
                                ondropchange: move |o: Offer| on_offer(o),
                    // The carried items are told by their dimming (#2154: no
                    // longer the default).
                    source: SortSource::Dim,
                                for c in lane.cols.clone() {
                                    SortableItem {
                                        key: c.id.clone(), value: c.id.clone(), handle: true,
                                        style: "width: 200px; flex-shrink: 0",
                                        SortableHandle { style: "height: 20px" }
                                        SortableList {
                                            keyboard: plain != Some(c.id.as_str()),
                                            group: "cards", id: c.id.clone(), label: c.id.clone(),
                                            style: "--rinch-sortable-gap: 0px; height: 120px; overflow-y: auto; padding-top: 6px",
                                            judge: Some(Rc::new(judge_cards) as SortJudge),
                                            drag_keys: Some(Rc::new(selection_for) as SortKeys),
                                            ondrop: move |m: SortMove| on_drop(board, m),
                                            ondropchange: move |o: Offer| on_offer(o),
                    // The carried items are told by their dimming (#2154: no
                    // longer the default).
                    source: SortSource::Dim,
                                            for card in c.cards.clone() {
                                                if card_tabindex {
                                                    SortableItem {
                                                        key: card.clone(), value: card.clone(),
                                                        style: "height: 30px; flex-shrink: 0",
                                                        "tabindex": "0",
                                                    }
                                                } else {
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

mod auto_scroll;
mod cancel;
mod handler_path;
mod keyboard;
mod moves;
mod nested_same_group;
mod nesting;
mod review_2093;
