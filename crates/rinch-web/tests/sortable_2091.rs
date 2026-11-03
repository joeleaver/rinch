//! Sortable lists (#2091) in a real browser: rinch-web's pointer-driven drag
//! suite and key delegation drive `SortableList` the way desktop's event path
//! does (`crates/rinch/src/app/sortable_2091_tests.rs`). A horizontal row of
//! three 150px columns in a 300px `overflow-x: auto` strip; each column a
//! 90px `overflow-y: auto` list of 30px cards: c1 [a b c d], c2 [e], c3 [].
//! c2 refuses a second card.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test sortable_2091
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2091";

#[derive(Clone, Debug, PartialEq)]
struct Col {
    id: String,
    cards: Vec<String>,
}

type Offer = Option<(SortMove, SortVerdict)>;

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
    static BOARD: Cell<Option<Signal<Vec<Col>>>> = const { Cell::new(None) };
    static DROPS: RefCell<Vec<SortMove>> = const { RefCell::new(Vec::new()) };
    static OFFERS: RefCell<Vec<Offer>> = const { RefCell::new(Vec::new()) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn initial() -> Vec<Col> {
    let col = |id: &str, cards: &[&str]| Col {
        id: id.into(),
        cards: cards.iter().map(|c| c.to_string()).collect(),
    };
    vec![
        col("c1", &["a", "b", "c", "d"]),
        col("c2", &["e"]),
        col("c3", &[]),
    ]
}

fn apply(board: &mut Vec<Col>, m: &SortMove) {
    if m.group == "cards" {
        for c in board.iter_mut() {
            c.cards.retain(|k| !m.keys.contains(k));
        }
        let to = board.iter_mut().find(|c| c.id == m.to_list).unwrap();
        let at = m
            .place
            .after
            .as_ref()
            .and_then(|a| to.cards.iter().position(|k| k == a))
            .map_or(0, |i| i + 1);
        for (i, k) in m.keys.iter().enumerate() {
            to.cards.insert(at + i, k.clone());
        }
    } else {
        let mut moved = Vec::new();
        for key in &m.keys {
            if let Some(i) = board.iter().position(|c| &c.id == key) {
                moved.push(board.remove(i));
            }
        }
        let at = m
            .place
            .after
            .as_ref()
            .and_then(|a| board.iter().position(|c| &c.id == a))
            .map_or(0, |i| i + 1);
        for (i, c) in moved.into_iter().enumerate() {
            board.insert(at + i, c);
        }
    }
}

fn judge(m: &SortMove) -> SortVerdict {
    if m.to_list == "c2" && m.from_list != "c2" {
        SortVerdict::Refuse("c2 is full".into())
    } else {
        SortVerdict::Accept
    }
}

fn on_drop(board: Signal<Vec<Col>>, m: SortMove) {
    DROPS.with(|d| d.borrow_mut().push(m.clone()));
    board.update(|b| apply(b, &m));
}

fn mount() -> Signal<Vec<Col>> {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    DROPS.with(|d| d.borrow_mut().clear());
    OFFERS.with(|d| d.borrow_mut().clear());
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 600px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let board = Signal::new(initial());
            BOARD.with(|b| b.set(Some(board)));
            rsx! {
                SortableList {
                    group: "columns", id: "row", axis: SortAxis::Horizontal,
                    style: "--rinch-sortable-gap: 0px; width: 300px; overflow-x: auto; margin: 0; padding: 0",
                    ondrop: move |m: SortMove| on_drop(board, m),
                    ondropchange: move |o: Offer| OFFERS.with(|v| v.borrow_mut().push(o)),
                    for c in board.get() {
                        SortableItem {
                            key: c.id.clone(), value: c.id.clone(), handle: true,
                            style: "flex: 0 0 150px; width: 150px",
                            SortableHandle { style: "height: 20px" }
                            SortableList {
                                group: "cards", id: c.id.clone(),
                                style: "--rinch-sortable-gap: 0px; height: 90px; overflow-y: auto; margin: 0; padding: 0",
                                judge: Some(Rc::new(judge) as SortJudge),
                                ondrop: move |m: SortMove| on_drop(board, m),
                                ondropchange: move |o: Offer| OFFERS.with(|v| v.borrow_mut().push(o)),
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
        },
    );
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    BOARD.with(|b| b.get()).unwrap()
}

fn item(key: &str) -> web_sys::HtmlElement {
    document()
        .query_selector(&format!("[{HOST_ATTR}] [data-sortable-key='{key}']"))
        .unwrap()
        .unwrap_or_else(|| panic!("no item {key}"))
        .dyn_into()
        .unwrap()
}

fn rect(el: &web_sys::Element) -> web_sys::DomRect {
    el.get_bounding_client_rect()
}

/// A point `fx`, `fy` of the way across and down `el`.
fn point(el: &web_sys::Element, fx: f64, fy: f64) -> (i32, i32) {
    let r = rect(el);
    (
        (r.x() + r.width() * fx) as i32,
        (r.y() + r.height() * fy) as i32,
    )
}

fn pointer(el: &web_sys::Element, name: &str, (x, y): (i32, i32)) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(if name == "pointermove" { -1 } else { 0 });
    init.set_buttons(if name == "pointerup" { 0 } else { 1 });
    init.set_client_x(x);
    init.set_client_y(y);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

fn at(p: (i32, i32)) -> web_sys::Element {
    document()
        .element_from_point(p.0 as f32, p.1 as f32)
        .expect("something under the pointer")
}

fn press(p: (i32, i32)) {
    pointer(&at(p), "pointerdown", p);
}

fn move_to(p: (i32, i32)) {
    pointer(&at(p), "pointermove", p);
}

fn release(p: (i32, i32)) {
    pointer(&at(p), "pointerup", p);
}

/// Press at `p` and cross the mouse threshold.
fn start(p: (i32, i32)) {
    press(p);
    move_to((p.0 + 8, p.1 + 1));
}

fn last_offer() -> Offer {
    OFFERS.with(|o| o.borrow().last().cloned().flatten())
}

fn drops() -> Vec<SortMove> {
    DROPS.with(|d| d.borrow().clone())
}

fn cards(board: Signal<Vec<Col>>, id: &str) -> Vec<String> {
    board.get().into_iter().find(|c| c.id == id).unwrap().cards
}

fn place(after: Option<&str>, before: Option<&str>, index: usize) -> SortPlace {
    SortPlace {
        after: after.map(str::to_string),
        before: before.map(str::to_string),
        index,
    }
}

fn shown(class: &str) -> usize {
    document()
        .query_selector_all(&format!("[{HOST_ATTR}] .{class}"))
        .unwrap()
        .length() as usize
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
fn a_card_moves_within_its_column() {
    let board = mount();
    start(point(&item("a"), 0.5, 0.5));
    assert!(
        item("a")
            .class_name()
            .contains("rinch-sortable__item--dragging")
    );
    let p = point(&item("c"), 0.5, 0.75);
    move_to(p);
    let (m, verdict) = last_offer().expect("an offer");
    assert_eq!(verdict, SortVerdict::Accept);
    assert_eq!((m.from_list.as_str(), m.to_list.as_str()), ("c1", "c1"));
    assert_eq!(m.place, place(Some("c"), Some("d"), 2));
    assert!(
        item("c")
            .class_name()
            .contains("rinch-sortable__item--drop-after")
    );
    release(p);
    assert_eq!(drops().len(), 1);
    assert_eq!(cards(board, "c1"), ["b", "c", "a", "d"]);
    assert_eq!(shown("rinch-sortable__item--drop-after"), 0);
    assert_eq!(shown("rinch-sortable__item--dragging"), 0);
}

#[wasm_bindgen_test]
fn a_refused_move_shows_its_reason_and_does_not_drop() {
    let board = mount();
    start(point(&item("a"), 0.5, 0.5));
    let p = point(&item("e"), 0.5, 0.75);
    move_to(p);
    assert_eq!(
        last_offer().map(|(m, v)| (m.to_list, m.place, v)),
        Some((
            "c2".to_string(),
            place(Some("e"), None, 1),
            SortVerdict::Refuse("c2 is full".into())
        ))
    );
    let reason = document()
        .query_selector(&format!("[{HOST_ATTR}] .rinch-sortable__reason--shown"))
        .unwrap()
        .expect("the reason is shown");
    assert_eq!(reason.text_content().as_deref(), Some("c2 is full"));
    let r = rect(&reason);
    assert!(r.width() > 0.0 && r.height() > 0.0, "it has a box");
    let e = rect(&item("e"));
    assert!(
        r.bottom() <= e.bottom() + 0.5 && r.bottom() > e.top() + e.height() / 2.0,
        "at e's bottom edge, inside it: {} vs {}..{}",
        r.bottom(),
        e.top(),
        e.bottom()
    );
    release(p);
    assert!(drops().is_empty());
    assert_eq!(cards(board, "c2"), ["e"]);
    assert_eq!(shown("rinch-sortable__reason--shown"), 0);
}

#[wasm_bindgen_test]
fn a_column_drags_by_its_handle_and_a_card_press_is_not_a_column_drag() {
    let board = mount();
    let handle = document()
        .query_selector(&format!(
            "[{HOST_ATTR}] [data-sortable-key='c1'] > .rinch-sortable__handle"
        ))
        .unwrap()
        .unwrap();
    start(point(&handle, 0.5, 0.5));
    let p = point(&item("e"), 0.8, 0.5);
    move_to(p);
    assert_eq!(
        last_offer().map(|(m, _)| (m.group, m.place)),
        Some(("columns".to_string(), place(Some("c2"), Some("c3"), 1)))
    );
    release(p);
    let order: Vec<String> = board.get().into_iter().map(|c| c.id).collect();
    assert_eq!(order, ["c2", "c1", "c3"]);
}

#[wasm_bindgen_test]
fn the_keyboard_lifts_moves_and_drops() {
    let board = mount();
    let a = item("a");
    a.focus().unwrap();
    let key = |k: &str| {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(k);
        init.set_code(if k == " " { "Space" } else { k });
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        document()
            .active_element()
            .unwrap()
            .dispatch_event(&ev)
            .unwrap();
        ev.default_prevented()
    };
    assert!(key(" "), "Space lifts, and is consumed");
    assert!(a.class_name().contains("rinch-sortable__item--lifted"));
    assert!(key("ArrowDown"));
    assert_eq!(
        last_offer().unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    assert!(key("ArrowRight"));
    let (m, v) = last_offer().unwrap();
    assert_eq!(
        (m.to_list.as_str(), v),
        ("c2", SortVerdict::Refuse("c2 is full".into()))
    );
    assert!(key("ArrowRight"));
    let (m, v) = last_offer().unwrap();
    assert_eq!(
        (m.to_list.as_str(), &m.place, v),
        ("c3", &place(None, None, 0), SortVerdict::Accept)
    );
    assert!(key("Enter"));
    assert_eq!(cards(board, "c3"), ["a"]);
    assert_eq!(shown("rinch-sortable__item--lifted"), 0);

    // Escape cancels the next lift.
    item("b").focus().unwrap();
    assert!(key("Enter"));
    assert!(key("ArrowDown"));
    assert!(key("Escape"));
    assert_eq!(shown("rinch-sortable__item--lifted"), 0);
    assert_eq!(drops().len(), 1);
    assert_eq!(cards(board, "c1"), ["b", "c", "d"]);
}

#[wasm_bindgen_test]
async fn a_drag_near_the_edges_scrolls_the_row_sideways_and_the_column_down() {
    let _board = mount();
    let row = document()
        .query_selector(&format!("[{HOST_ATTR}] [data-sortable-group='columns']"))
        .unwrap()
        .unwrap();
    let c1 = document()
        .query_selector(&format!("[{HOST_ATTR}] [data-sortable-group='cards']"))
        .unwrap()
        .unwrap();
    start(point(&item("b"), 0.5, 0.5));
    // The row's right edge, in the middle of a card list's height.
    let r = rect(&row);
    let l = rect(&c1);
    move_to((
        (r.right() - 3.0) as i32,
        (l.top() + l.height() / 2.0) as i32,
    ));
    let mut left = 0;
    for _ in 0..40 {
        sleep_ms(25).await;
        left = row.scroll_left();
        if left > 20 {
            break;
        }
    }
    assert!(left > 20, "the row scrolled right: {left}");
    // c1's bottom edge, at its left: the column scrolls down. (The row
    // scrolled; find c1's list where it is now.)
    row.set_scroll_left(0);
    let l = rect(&c1);
    move_to(((l.left() + 10.0) as i32, (l.bottom() - 3.0) as i32));
    let mut top = 0;
    for _ in 0..40 {
        sleep_ms(25).await;
        top = c1.scroll_top();
        if top > 5 {
            break;
        }
    }
    assert!(top > 5, "the column scrolled down: {top}");
    release(((l.left() + 10.0) as i32, (l.bottom() - 3.0) as i32));
}
