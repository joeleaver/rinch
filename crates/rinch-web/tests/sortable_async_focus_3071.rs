//! A keyboard drop the app applies later (#3071 review 6), in a real
//! browser: the moved card gets the focus once the redraw that applies the
//! drop is drawn.
//!
//! rinch-web blurs a focused node before it removes it (or moves it, where
//! the browser has no `moveBefore`), while the node is still in place, so
//! the browser's `focusout` (no `relatedTarget`) tells the sortable's focus
//! request that the focus fell to `<body>` with the lifted card still drawn,
//! before the redraw has drawn anything. That must not end the request: a
//! redraw keyed by position draws the card again as a new node. A redraw
//! keyed by card moves the card's own node, which keeps the focus where
//! `moveBefore` is (both browsers here). A blur with no redraw (a click on
//! blank space) does end the request.
//!
//! One keyboard list, c1 [a b c d], whose `ondrop` records the drop and
//! applies nothing; the test applies it a turn later, as an app does after
//! an await. `a` is dropped last, so a keyed redraw moves `a`'s own node.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test sortable_async_focus_3071
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-3071";

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
    static CARDS: Cell<Option<Signal<Vec<String>>>> = const { Cell::new(None) };
    static DROPS: RefCell<Vec<SortMove>> = const { RefCell::new(Vec::new()) };
}

/// How the app draws its cards.
#[derive(Clone, Copy)]
enum Rows {
    /// Keyed by position: a moved card is drawn again as a new node.
    ByIndex,
    /// Keyed by card: a moved card keeps its node, which the redraw moves.
    ByCard,
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(rows: Rows) -> Signal<Vec<String>> {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    DROPS.with(|d| d.borrow_mut().clear());
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 300px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let cards = Signal::new(["a", "b", "c", "d"].map(String::from).to_vec());
            CARDS.with(|c| c.set(Some(cards)));
            let rows = move || {
                cards
                    .get()
                    .into_iter()
                    .enumerate()
                    .map(|(i, card)| match rows {
                        Rows::ByIndex => (i.to_string(), card),
                        Rows::ByCard => (card.clone(), card),
                    })
                    .collect::<Vec<_>>()
            };
            rsx! {
                SortableList {
                    keyboard: true, group: "cards", id: "c1",
                    style: "--rinch-sortable-gap: 0px; width: 150px; margin: 0; padding: 0",
                    ondrop: move |m: SortMove| DROPS.with(|d| d.borrow_mut().push(m)),
                    for (row, card) in rows() {
                        SortableItem {
                            key: row.clone(), value: card.clone(),
                            style: "height: 30px; flex-shrink: 0",
                            {card.clone()}
                        }
                    }
                }
            }
        },
    );
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    CARDS.with(|c| c.get()).unwrap()
}

fn item(key: &str) -> web_sys::HtmlElement {
    document()
        .query_selector(&format!("[{HOST_ATTR}] [data-sortable-key='{key}']"))
        .unwrap()
        .unwrap_or_else(|| panic!("no item {key}"))
        .dyn_into()
        .unwrap()
}

/// The keys of the drawn items, in order.
fn drawn() -> Vec<String> {
    let found = document()
        .query_selector_all(&format!("[{HOST_ATTR}] [data-sortable-key]"))
        .unwrap();
    (0..found.length())
        .filter_map(|i| found.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .filter_map(|e| e.get_attribute("data-sortable-key"))
        .collect()
}

/// The `data-sortable-key` of the focused element, or its tag when it has
/// none (`BODY`).
fn focused() -> String {
    let el = document().active_element().unwrap();
    el.get_attribute("data-sortable-key")
        .unwrap_or_else(|| el.tag_name())
}

/// A keydown of `k` on the focused element: whether it was consumed.
fn key(k: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(k);
    init.set_code(if k == " " { "Space" } else { k });
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    document()
        .active_element()
        .unwrap()
        .dispatch_event(&ev)
        .unwrap();
    ev.default_prevented()
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

/// Lift `a` by keyboard and drop it last, the app applying nothing yet, and
/// let the event loop turn: the drop's request waits, `a` still focused.
async fn drop_a_last(rows: Rows) -> Signal<Vec<String>> {
    let cards = mount(rows);
    item("a").focus().unwrap();
    assert_eq!(focused(), "a", "control: a focused");
    assert!(key(" "), "Space lifts");
    for _ in 0..3 {
        assert!(key("ArrowDown"));
    }
    assert!(key("Enter"), "Enter drops");
    assert_eq!(DROPS.with(|d| d.borrow().len()), 1, "control: dropped");
    sleep_ms(20).await;
    assert_eq!(drawn(), ["a", "b", "c", "d"], "control: not applied yet");
    assert_eq!(focused(), "a", "control: a keeps the focus until applied");
    cards
}

/// Apply the recorded drop, as the app does after an await.
fn apply_drop(cards: Signal<Vec<String>>) {
    let m = DROPS.with(|d| d.borrow()[0].clone());
    cards.update(|c| {
        c.retain(|k| !m.keys.contains(k));
        let at = m
            .place
            .after
            .as_ref()
            .and_then(|a| c.iter().position(|k| k == a))
            .map_or(0, |i| i + 1);
        for (i, k) in m.keys.iter().enumerate() {
            c.insert(at + i, k.clone());
        }
    });
}

async fn applied_later_focuses_the_card(rows: Rows) {
    let cards = drop_a_last(rows).await;
    let old = item("a");
    // Every focus change the redraw makes, as the browser reports it.
    let seen = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
    let log = {
        let seen = seen.clone();
        wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::FocusEvent)>::new(
            move |e: web_sys::FocusEvent| seen.borrow_mut().push(e.type_()),
        )
    };
    for name in ["focusin", "focusout"] {
        document()
            .add_event_listener_with_callback_and_bool(name, log.as_ref().unchecked_ref(), true)
            .unwrap();
    }
    apply_drop(cards);
    let during = seen.borrow().clone();
    for name in ["focusin", "focusout"] {
        document()
            .remove_event_listener_with_callback_and_bool(name, log.as_ref().unchecked_ref(), true)
            .unwrap();
    }
    assert_eq!(drawn(), ["b", "c", "d", "a"], "control: applied");
    // A redraw by position removes the focused card: rinch-web blurs it
    // first. A keyed move uses `moveBefore` where the browser has it (Chrome
    // and Firefox here), which keeps the focus and fires nothing; where it
    // has not, the move is a removal and blurs it like the removal does
    // (that order is `focus_tests.rs`'s
    // `a_keyed_move_after_the_web_blur_focuses_the_item`).
    if matches!(rows, Rows::ByIndex) {
        assert_eq!(
            during.first().map(String::as_str),
            Some("focusout"),
            "control: the redraw blurred the lifted card first ({during:?})"
        );
    }
    match rows {
        Rows::ByIndex => assert!(!old.is_same_node(Some(&item("a"))), "control: drawn again"),
        Rows::ByCard => assert!(old.is_same_node(Some(&item("a"))), "control: moved"),
    }
    sleep_ms(20).await;
    assert_eq!(
        focused(),
        "a",
        "the redraw that applied the drop lost the focus"
    );
}

/// Keyed by position: the redraw removes `a`'s node (blurring it first) and
/// draws `a` again last.
#[wasm_bindgen_test]
async fn a_redraw_by_position_gives_the_dropped_card_the_focus() {
    applied_later_focuses_the_card(Rows::ByIndex).await;
}

/// Keyed by card: the redraw moves `a`'s own node last (`moveBefore`, the
/// focus kept; a guard, which passes on the old rule too).
#[wasm_bindgen_test]
async fn a_keyed_move_gives_the_dropped_card_the_focus() {
    applied_later_focuses_the_card(Rows::ByCard).await;
}

/// The focus falling to `<body>` with no redraw (a click on blank space)
/// while `a` is still drawn ends the request: the drop applied after that
/// leaves the focus where the user put it.
#[wasm_bindgen_test]
async fn a_blur_to_the_body_ends_the_request() {
    let cards = drop_a_last(Rows::ByIndex).await;
    item("a").blur().unwrap();
    assert_eq!(focused(), "BODY", "control: the focus fell to the body");
    sleep_ms(20).await;
    apply_drop(cards);
    assert_eq!(drawn(), ["b", "c", "d", "a"], "control: applied");
    sleep_ms(20).await;
    assert_eq!(focused(), "BODY", "the drop took the focus back");
}
