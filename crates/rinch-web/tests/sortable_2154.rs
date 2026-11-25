//! Sortable drag presentation options (#2154) in a real browser, as desktop
//! pins them (`crates/rinch/src/app/sortable_2154_tests/`): the default
//! preview (a translucent copy at the grab offset), the gap, a refused place,
//! a collapsed source, a keyboard lift's gap, and the drag-state signal.
//!
//! Two vertical lists side by side, c1 [a b c d] and c2 [e f]: 30px cards
//! with 10px between them, so a gap is 40px. c2 refuses `d`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test sortable_2154
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2154";
const STEP: f64 = 40.0;

type Board = Vec<(String, Vec<String>)>;

#[derive(Clone)]
struct Options {
    preview: SortPreview,
    indicator: SortIndicator,
    source: SortSource,
}

impl Default for Options {
    fn default() -> Self {
        let list = SortableList::default();
        Self {
            preview: list.preview,
            indicator: list.indicator,
            source: list.source,
        }
    }
}

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
    static BOARD: Cell<Option<Signal<Board>>> = const { Cell::new(None) };
    static SEEN: RefCell<Vec<Option<SortDragState>>> = const { RefCell::new(Vec::new()) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn judge(m: &SortMove) -> SortVerdict {
    if m.to_list == "c2" && m.keys.iter().any(|k| k == "d") {
        SortVerdict::Refuse("No d in c2".into())
    } else {
        SortVerdict::Accept
    }
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

fn mount(options: Options) -> Signal<Board> {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    SEEN.with(|s| s.borrow_mut().clear());
    // A drag a failed test left open ends here, not in the next test.
    if let Some(root) = document().document_element() {
        pointer(&root, "pointerup", (0, 0));
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    // Geometry without a running clock: no slide.
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 600px; \
         --rinch-sortable-reflow-duration: 0ms",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            let board: Signal<Board> = Signal::new(vec![
                (
                    "c1".into(),
                    ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect(),
                ),
                ("c2".into(), vec!["e".into(), "f".into()]),
            ]);
            BOARD.with(|b| b.set(Some(board)));
            __scope.create_effect(move || {
                let state = sortable_drag();
                SEEN.with(|s| s.borrow_mut().push(state));
            });
            let o = options.clone();
            rsx! {
                div { style: "display: flex; gap: 100px; align-items: flex-start; padding: 20px",
                    for (id, cards) in board.get() {
                        SortableList {
                            keyboard: true,
                            key: id.clone(), group: "cards", id: id.clone(),
                            style: "--rinch-sortable-gap: 10px; width: 150px; margin: 0; padding: 0",
                            judge: Some(Rc::new(judge) as SortJudge),
                            preview: o.preview.clone(), indicator: o.indicator, source: o.source,
                            ondrop: move |m: SortMove| board.update(|b| apply(b, &m)),
                            for card in cards.clone() {
                                SortableItem {
                                    key: card.clone(), value: card.clone(),
                                    style: "height: 30px; flex-shrink: 0; margin: 0; padding: 0; box-sizing: border-box",
                                    {card.clone()}
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
        .unwrap_or_else(|| document().document_element().unwrap())
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

fn cards(board: Signal<Board>, id: &str) -> Vec<String> {
    board.get().into_iter().find(|(l, _)| l == id).unwrap().1
}

fn ghost() -> Option<web_sys::Element> {
    document().query_selector(".rinch-sortable__ghost").unwrap()
}

/// The preview's one box (the wrapper and the stand-ins draw none).
fn ghost_body() -> web_sys::Element {
    document()
        .query_selector(".rinch-sortable__ghost-body")
        .unwrap()
        .expect("a preview body")
}

/// How far `key`'s item is painted from where layout put it, down.
fn slid(key: &str) -> f64 {
    let el = item(key);
    rect(&el).y() - (el.offset_top() as f64 + rect(&el.offset_parent().unwrap()).y())
}

fn count(class: &str) -> u32 {
    document()
        .query_selector_all(&format!(".{class}"))
        .unwrap()
        .length()
}

fn assert_clean() {
    assert!(ghost().is_none(), "the preview is gone");
    for class in [
        "rinch-sortable__item--shifted",
        "rinch-sortable--reflowing",
        "rinch-sortable__item--collapsed",
        "rinch-sortable__item--drop-after",
        "rinch-sortable__item--drop-before",
    ] {
        assert_eq!(count(class), 0, "{class} left behind");
    }
    assert_eq!(sortable_drag(), None);
    assert_eq!(sortable_drag_pointer(), None);
}

#[path = "sortable_2154/copies.rs"]
mod copies;

#[path = "sortable_2154/presentation.rs"]
mod presentation;

#[path = "sortable_2154/preview_styling.rs"]
mod preview_styling;
