//! `SortableList::slot` (#3065): with a gap, a box the size of the carried
//! card where the drop lands, drawn as the list's own pseudo-element. Off by
//! default, and then a drag writes nothing of it and changes no layout.
//!
//! Every placement is checked the same way: the slot's painted box, then the
//! drop, then the dropped card's box, which must be the slot's. Two lists c1
//! and c2 side by side, each laid out as the test says (a column, a row, a
//! wrapping row, a grid, right to left), cards 30px along the axis with 10px
//! between them, and the lists only as tall as their content.

use super::*;
use crate as rinch;
use std::cell::{Cell, RefCell};

use rinch_components::{
    SortAxis, SortIndicator, SortMove, SortPreview, SortSource, SortableItem, SortableList,
};
use rinch_core::Signal;
use rinch_core::element::IntoEventHandler;
use rinch_macros::rsx;

const VIEWPORT: (f32, f32) = (800.0, 600.0);

type Board = Vec<(String, Vec<String>)>;

#[derive(Clone)]
struct Setup {
    slot: bool,
    indicator: SortIndicator,
    axis: SortAxis,
    /// Each list's style, after the spacing.
    list: &'static str,
    /// Each card's style.
    card: &'static str,
    c1: &'static [&'static str],
    c2: &'static [&'static str],
}

impl Setup {
    /// Two columns of 150px-wide cards, the slot on.
    fn column() -> Self {
        Self {
            slot: true,
            indicator: SortIndicator::Gap,
            axis: SortAxis::Vertical,
            list: "width: 150px",
            card: "height: 30px; flex-shrink: 0",
            c1: &["a", "b", "c", "d"],
            c2: &["e", "f"],
        }
    }

    /// Two rows of 30px-square cards, the slot on.
    fn row(list: &'static str) -> Self {
        Self {
            axis: SortAxis::Horizontal,
            list,
            card: "width: 30px; height: 30px; flex-shrink: 0",
            ..Self::column()
        }
    }
}

thread_local! {
    static SETUP: RefCell<Setup> = RefCell::new(Setup::column());
    static BOARD: Cell<Option<Signal<Board>>> = const { Cell::new(None) };
}

fn apply(board: &mut Board, m: &SortMove) {
    for (_, cards) in board.iter_mut() {
        cards.retain(|k| !m.keys.contains(k));
    }
    let (_, cards) = board.iter_mut().find(|(id, _)| *id == m.to_list).unwrap();
    let at = m
        .place
        .after
        .as_ref()
        .and_then(|a| cards.iter().position(|k| k == a))
        .map_or(0, |i| i + 1);
    for (i, k) in m.keys.iter().enumerate() {
        cards.insert(at + i, k.clone());
    }
}

struct Fixture {
    app: RinchApp,
    board: Signal<Board>,
}

fn fixture(setup: Setup) -> Fixture {
    SETUP.with(|s| *s.borrow_mut() = setup);
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let s = SETUP.with(|s| s.borrow().clone());
        let keys = |k: &[&str]| k.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        let board = Signal::new(vec![
            ("c1".to_string(), keys(s.c1)),
            ("c2".to_string(), keys(s.c2)),
        ]);
        BOARD.with(|b| b.set(Some(board)));
        let list_style = format!(
            "--rinch-sortable-gap: 10px; \
             --rinch-sortable-slot-background: rgb(0, 200, 0); {}",
            s.list
        );
        rsx! {
            div { style: "display: flex; align-items: flex-start; gap: 100px; width: 800px; height: 600px; padding: 20px",
                for (id, cards) in board.get() {
                    SortableList {
                        key: id.clone(), group: "cards", id: id.clone(),
                        style: list_style.clone(), axis: s.axis, slot: s.slot,
                        preview: SortPreview::None, indicator: s.indicator,
                        source: SortSource::Collapse,
                        ondrop: move |m: SortMove| board.update(|b| apply(b, &m)),
                        for card in cards.clone() {
                            SortableItem {
                                key: card.clone(), value: card.clone(), style: s.card,
                                span { {card.clone()} }
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
        d.load_css(":root { --rinch-sortable-reflow-duration: 0ms }");
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

/// The lists, c1 then c2 (in document order).
fn lists(f: &Fixture) -> Vec<u64> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut found: Vec<u64> = d
        .tree
        .nodes
        .iter()
        .filter(|(id, n)| n.attributes.contains_key("data-sortable-list") && connected(&d, *id))
        .map(|(id, _)| id)
        .collect();
    found.sort_by_key(|id| box_in(&d, *id).0 as i64);
    found
}

fn box_in(d: &RinchDocument, id: u64) -> (f32, f32, f32, f32) {
    hit_testing::painted_element_box(&d.tree, id)
}

fn box_of(f: &Fixture, id: u64) -> (f32, f32, f32, f32) {
    let doc = f.app.doc.as_ref().unwrap();
    box_in(&doc.borrow(), id)
}

fn point(f: &Fixture, id: u64, fx: f32, fy: f32) -> (f32, f32) {
    let (x, y, w, h) = box_of(f, id);
    (x + w * fx, y + h * fy)
}

fn press_at(f: &mut Fixture, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    send(f, PlatformEvent::MouseDown { x, y, button });
}

fn move_to(f: &mut Fixture, (x, y): (f32, f32)) {
    send(f, PlatformEvent::MouseMove { x, y });
}

fn release_at(f: &mut Fixture, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    send(f, PlatformEvent::MouseUp { x, y, button });
}

/// Press `k`'s card in its middle and pass the threshold.
fn grab(f: &mut Fixture, k: &str) {
    let at = point(f, item(f, k), 0.5, 0.5);
    press_at(f, at);
    move_to(f, (at.0 + 8.0, at.1 + 3.0));
    assert!(f.app.active_dnd.is_some(), "the drag went live");
}

const SLOT_CLASSES: [&str; 2] = ["rinch-sortable--slot", "rinch-sortable--slot-into"];

/// Every node carrying a slot class, a slot property or a generated box.
fn slot_traces(f: &Fixture) -> Vec<String> {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut traces = Vec::new();
    for (id, n) in d.tree.nodes.iter() {
        if !connected(&d, id) {
            continue;
        }
        let class = n.attributes.get("class").map_or("", String::as_str);
        if class.split_whitespace().any(|c| SLOT_CLASSES.contains(&c)) {
            traces.push(format!("{id}: class {class}"));
        }
        let style = n.attributes.get("style").map_or("", String::as_str);
        if style.contains("--rinch-sortable-slot-") && !style.contains("slot-background") {
            traces.push(format!("{id}: style {style}"));
        }
        if n.is_pseudo_element && n.parent.is_some_and(|p| lists(f).contains(&p)) {
            traces.push(format!("{id}: a list's pseudo-element"));
        }
    }
    traces
}

/// The painted box of the one slot drawn, and the list drawing it.
fn slot(f: &Fixture) -> (u64, (f32, f32, f32, f32)) {
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut drawn = Vec::new();
    for list in lists(f) {
        let n = &d.tree.nodes[list];
        let class = n.attributes.get("class").cloned().unwrap_or_default();
        let has = |c: &str| class.split_whitespace().any(|one| one == c);
        let child = if has(SLOT_CLASSES[0]) {
            n.children.first()
        } else if has(SLOT_CLASSES[1]) {
            n.children.last()
        } else {
            continue;
        };
        let child = *child.expect("the list has children");
        assert!(
            d.tree.nodes[child].is_pseudo_element,
            "the slot is generated"
        );
        drawn.push((list, box_in(&d, child)));
    }
    assert_eq!(drawn.len(), 1, "one slot: {drawn:?}");
    drawn[0]
}

fn near(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
        .iter()
        .all(|(p, q)| (p - q).abs() < 0.5)
}

/// Carry `k` over `over`: the slot drawn is where the drop then puts `k`.
/// Returns which list drew it (0 for c1, 1 for c2) and its box.
fn lands_in_its_slot(f: &mut Fixture, k: &str, over: (f32, f32)) -> (usize, (f32, f32, f32, f32)) {
    move_to(f, over);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let (list, at) = slot(f);
    let list = lists(f).iter().position(|l| *l == list).unwrap();
    release_at(f, over);
    let landed = box_of(f, item(f, k));
    assert!(
        near(at, landed),
        "the slot {at:?}, {k} landed at {landed:?}"
    );
    assert!(slot_traces(f).is_empty(), "{:?}", slot_traces(f));
    (list, at)
}

fn cards(f: &Fixture, id: &str) -> Vec<String> {
    f.board.get().into_iter().find(|(l, _)| l == id).unwrap().1
}

#[test]
fn off_by_default_a_gap_writes_no_slot_and_moves_nothing() {
    assert!(!SortableList::default().slot);
    let mut f = fixture(Setup {
        slot: false,
        ..Setup::column()
    });
    let c2 = lists(&f)[1];
    let before = box_of(&f, c2);
    grab(&mut f, "b");
    // Before f, then after it, the last.
    for fy in [0.2, 0.8] {
        let over = point(&f, item(&f, "f"), 0.5, fy);
        move_to(&mut f, over);
        assert!(slot_traces(&f).is_empty(), "{:?}", slot_traces(&f));
        assert_eq!(box_of(&f, c2), before, "the list keeps its size");
    }
    let over = point(&f, item(&f, "f"), 0.5, 0.8);
    release_at(&mut f, over);
    assert_eq!(cards(&f, "c2"), ["e", "f", "b"]);
}

#[test]
fn no_slot_without_a_gap() {
    let mut f = fixture(Setup {
        indicator: SortIndicator::Caret,
        ..Setup::column()
    });
    grab(&mut f, "a");
    let over = point(&f, item(&f, "f"), 0.5, 0.2);
    move_to(&mut f, over);
    assert!(slot_traces(&f).is_empty(), "{:?}", slot_traces(&f));
}

/// Between two cards: the list's `::before`, in the gap, the card's size.
#[test]
fn between_cards_the_list_draws_it_in_the_gap() {
    let mut f = fixture(Setup::column());
    grab(&mut f, "b");
    let over = point(&f, item(&f, "f"), 0.5, 0.2);
    let (list, (_, y, w, h)) = lands_in_its_slot(&mut f, "b", over);
    assert_eq!(list, 1);
    assert_eq!(cards(&f, "c2"), ["e", "b", "f"]);
    let (_, ey, _, eh) = box_of(&f, item(&f, "e"));
    assert!((y - (ey + eh + 10.0)).abs() < 0.5, "{y} after e");
    assert_eq!((w, h), (150.0, 30.0));
}

/// A card that clips its content (`overflow: hidden`) does not clip the
/// slot: it is the list's, not the card's.
#[test]
fn a_clipping_card_does_not_clip_the_slot() {
    let mut f = fixture(Setup {
        card: "height: 30px; flex-shrink: 0; overflow: hidden",
        ..Setup::column()
    });
    grab(&mut f, "b");
    let over = point(&f, item(&f, "f"), 0.5, 0.2);
    move_to(&mut f, over);
    let (_, (x, y, w, h)) = slot(&f);
    let px = f.app.build_pixels(1.0, (800, 600), true).0.to_vec();
    let (cx, cy) = ((x + w / 2.0) as usize, (y + h / 2.0) as usize);
    let at = (cy * 800 + cx) * 4;
    assert_eq!(
        &px[at..at + 3],
        &[0, 200, 0],
        "the slot is painted at {cx}, {cy}"
    );
}

/// At the end of another list: the list's in-flow `::after`, and the list
/// grows to hold it.
#[test]
fn at_the_end_of_another_list_the_list_grows_to_hold_it() {
    let mut f = fixture(Setup::column());
    let c2 = lists(&f)[1];
    let (_, _, _, rest) = box_of(&f, c2);
    grab(&mut f, "b");
    let over = point(&f, item(&f, "f"), 0.5, 0.8);
    move_to(&mut f, over);
    assert_eq!(box_of(&f, c2).3, rest + 40.0, "the list holds the slot");
    lands_in_its_slot(&mut f, "b", over);
    assert_eq!(cards(&f, "c2"), ["e", "f", "b"]);
}

/// Collapsed in its own list, the card's room is still there at the end.
#[test]
fn at_the_end_of_its_own_list_it_is_the_room_the_card_left() {
    let mut f = fixture(Setup::column());
    let c1 = lists(&f)[0];
    let (_, _, _, rest) = box_of(&f, c1);
    grab(&mut f, "b");
    let over = point(&f, item(&f, "d"), 0.5, 0.8);
    move_to(&mut f, over);
    assert_eq!(box_of(&f, c1).3, rest, "the list does not grow");
    lands_in_its_slot(&mut f, "b", over);
    assert_eq!(cards(&f, "c1"), ["a", "c", "d", "b"]);
}

#[test]
fn an_empty_list_draws_it_where_the_first_card_goes() {
    let mut f = fixture(Setup {
        c2: &[],
        ..Setup::column()
    });
    grab(&mut f, "a");
    let c2 = lists(&f)[1];
    let over = point(&f, c2, 0.5, 0.5);
    lands_in_its_slot(&mut f, "a", over);
    assert_eq!(cards(&f, "c2"), ["a"]);
}

/// A wrapping row, three cards a line; `a` collapsed in its own list and
/// carried to after `c`, the line's last: the cards before the gap slid back
/// onto the first line, and the slot is the room after them there (not
/// before `d`, past the list's start).
#[test]
fn a_wrapping_row_puts_it_on_the_line_the_card_lands_on() {
    let mut f = fixture(Setup {
        c1: &["a", "b", "c", "d", "e", "f"],
        c2: &["g"],
        ..Setup::row("width: 110px; flex-wrap: wrap")
    });
    grab(&mut f, "a");
    let over = point(&f, item(&f, "c"), 0.8, 0.5);
    let (list, (x, y, _, _)) = lands_in_its_slot(&mut f, "a", over);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "d", "e", "f"]);
    let (lx, ly, _, _) = box_of(&f, lists(&f)[list]);
    assert_eq!((x - lx, y - ly), (80.0, 0.0));
}

/// A grid, three columns. Carried from the other list to after `c`, the
/// row's last, the slot is the next row's first cell, where `d` was. And
/// `a`, collapsed in its own grid and carried to after `c`, lands in the
/// first row's last cell (the cards before the gap slid back into it), not
/// in a cell before the second row's start.
#[test]
fn a_grid_puts_it_in_the_cell_the_card_lands_in() {
    let mut f = fixture(Setup {
        c1: &["a", "b", "c", "d", "e"],
        c2: &["g"],
        ..Setup::row("width: 110px; display: grid; grid-template-columns: repeat(3, 30px)")
    });
    let d_was = box_of(&f, item(&f, "d"));
    grab(&mut f, "g");
    let over = point(&f, item(&f, "c"), 0.8, 0.5);
    let (_, at) = lands_in_its_slot(&mut f, "g", over);
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "g", "d", "e"]);
    assert!(near(at, d_was), "the slot {at:?} is d's cell {d_was:?}");

    grab(&mut f, "a");
    let over = point(&f, item(&f, "c"), 0.8, 0.5);
    let (list, (x, y, _, _)) = lands_in_its_slot(&mut f, "a", over);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "g", "d", "e"]);
    let (lx, ly, _, _) = box_of(&f, lists(&f)[list]);
    assert_eq!((x - lx, y - ly), (80.0, 0.0));
}

/// Right to left, the first card at the right. Carried from the other list
/// between two cards, and collapsed in its own list and carried to its end,
/// the left: each time the slot is where the card lands, on the side the
/// row ends on.
#[test]
fn right_to_left_puts_it_where_the_card_lands() {
    let mut f = fixture(Setup::row("width: 190px; direction: rtl"));
    grab(&mut f, "e");
    let over = point(&f, item(&f, "b"), 0.5, 0.5);
    lands_in_its_slot(&mut f, "e", over);
    assert_eq!(cards(&f, "c1").len(), 5);

    grab(&mut f, "b");
    let keys = cards(&f, "c1");
    let last = item(&f, keys.last().unwrap());
    let over = point(&f, last, 0.8, 0.5);
    let (list, (x, _, _, _)) = lands_in_its_slot(&mut f, "b", over);
    assert_eq!(cards(&f, "c1"), ["a", "e", "c", "d", "b"]);
    let (lx, _, _, _) = box_of(&f, lists(&f)[list]);
    assert_eq!(x - lx, 0.0, "at the left end");
}

/// Right to left, collapsed in its own list and carried between two cards:
/// the cards before the place close up toward the row's start (the right),
/// and the slot is the room where the card lands.
#[test]
fn right_to_left_a_collapsed_card_lands_in_its_slot() {
    let mut f = fixture(Setup::row("width: 150px; direction: rtl"));
    grab(&mut f, "a");
    let over = point(&f, item(&f, "c"), 0.5, 0.5);
    lands_in_its_slot(&mut f, "a", over);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "d"]);
}

/// Right to left, a card carried in from the other list and one collapsed in
/// its own list: the cards make room the way the row runs, so none is drawn
/// over the slot (#3065 review: the shifts were drawn toward larger x, and
/// the card after the gap slid onto it).
#[test]
fn right_to_left_no_card_is_drawn_over_the_slot() {
    let overlaps = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
        a.0 < b.0 + b.2 - 0.5 && b.0 < a.0 + a.2 - 0.5
    };
    let mut f = fixture(Setup::row("width: 190px; direction: rtl"));
    for (k, over) in [("e", "b"), ("a", "c")] {
        grab(&mut f, k);
        let over = point(&f, item(&f, over), 0.5, 0.5);
        move_to(&mut f, over);
        f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        let (_, at) = slot(&f);
        for key in cards(&f, "c1").iter().filter(|c| *c != k) {
            let b = box_of(&f, item(&f, key));
            assert!(!overlaps(at, b), "{key} at {b:?} is over the slot {at:?}");
        }
        lands_in_its_slot(&mut f, k, over);
    }
}

/// Grab `k` and carry it over `g`, the other list's one card, near its start:
/// how far its own list's `next` (collapsing toward it) and `g` (opening the
/// gap) move along x. Each is the carried card's 30px and the 10px spacing.
fn gap_of(f: &mut Fixture, k: &str, next: &str) -> (f32, f32) {
    let (next_was, g_was) = (box_of(f, item(f, next)).0, box_of(f, item(f, "g")).0);
    grab(f, k);
    let over = point(f, item(f, "g"), 0.2, 0.5);
    move_to(f, over);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let moved = (
        box_of(f, item(f, next)).0 - next_was,
        box_of(f, item(f, "g")).0 - g_was,
    );
    release_at(f, over);
    moved
}

/// A wrapping row, three cards a line. `c` is its line's last, and the next
/// card, `d`, starts the next line: the spacing is measured against `b`, on
/// `c`'s line, not `d` (#3065: against `d` it took in the line's length, 50px,
/// and the gap opened by 80px).
#[test]
fn a_wrapping_row_measures_the_spacing_on_the_grabbed_cards_line() {
    let mut f = fixture(Setup {
        c1: &["a", "b", "c", "d", "e"],
        c2: &["g"],
        ..Setup::row("width: 110px; flex-wrap: wrap")
    });
    assert_eq!(gap_of(&mut f, "c", "d"), (-40.0, 40.0));
}

/// The same in a grid of three columns: `c` ends its row.
#[test]
fn a_grid_measures_the_spacing_in_the_grabbed_cards_row() {
    let mut f = fixture(Setup {
        c1: &["a", "b", "c", "d", "e"],
        c2: &["g"],
        ..Setup::row("width: 110px; display: grid; grid-template-columns: repeat(3, 30px)")
    });
    assert_eq!(gap_of(&mut f, "c", "d"), (-40.0, 40.0));
}

/// `a b c / d e f / h`: `h` is alone on its line, so it has no neighbour
/// there, and the one before it, `f`, ends the line above. The spacing is the
/// list's, measured between two cards that share a line, not against `f`
/// (#3065: 50px, and the gap opened by 80px).
#[test]
fn a_card_alone_on_its_line_takes_the_lists_spacing() {
    let mut f = fixture(Setup {
        c1: &["a", "b", "c", "d", "e", "f", "h"],
        c2: &["g"],
        ..Setup::row("width: 110px; flex-wrap: wrap")
    });
    assert_eq!(gap_of(&mut f, "h", "a"), (0.0, 40.0));
}
