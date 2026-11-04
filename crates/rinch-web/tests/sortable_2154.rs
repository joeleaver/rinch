//! Sortable drag presentation options (#2154) in a real browser, as desktop
//! pins them (`crates/rinch/src/app/sortable_2154_tests.rs`): the default
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

#[wasm_bindgen_test]
fn the_default_preview_is_a_translucent_copy_at_the_grab_offset() {
    let board = mount(Options::default());
    let b = item("b");
    let r = rect(&b);
    let press_at = point(&b, 0.2, 0.3);
    let (gx, gy) = (press_at.0 as f64 - r.x(), press_at.1 as f64 - r.y());
    press(press_at);
    move_to((press_at.0 + 8, press_at.1 + 2));
    assert!(sortable_drag().is_some(), "the drag went live");
    // A real event reached rinch: the source is unchanged, a ghost is up.
    assert!(!b.class_name().contains("--dragging"), "{}", b.class_name());
    for to in [(300, 250), (420, 77), (40, 300)] {
        move_to(to);
        let g = ghost().expect("a preview");
        let gr = rect(&g);
        assert!(
            (gr.x() - (to.0 as f64 - gx)).abs() < 1.0 && (gr.y() - (to.1 as f64 - gy)).abs() < 1.0,
            "ghost at {},{} for {to:?}",
            gr.x(),
            gr.y()
        );
        assert!((gr.width() - r.width()).abs() < 1.0 && (gr.height() - r.height()).abs() < 1.0);
    }
    let g = ghost().unwrap();
    // On the document's body: rinch-web's is the mount's root, outside
    // every list.
    assert!(g.closest("[data-sortable-list]").unwrap().is_none());
    assert_eq!(g.text_content().as_deref(), Some("b"));
    assert!(
        g.query_selector("[data-sortable-key], [data-onclick], [tabindex]")
            .unwrap()
            .is_none()
    );
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&g)
        .unwrap()
        .unwrap();
    assert_eq!(style.get_property_value("opacity").unwrap(), "0.75");
    assert_eq!(style.get_property_value("pointer-events").unwrap(), "none");
    // The ghost is never what is under the pointer.
    let over = point(&item("e"), 0.5, 0.75);
    move_to(over);
    assert_eq!(
        sortable_drag().unwrap().target,
        Some(SortTarget {
            list: "c2".into(),
            index: 1
        })
    );
    release(over);
    assert_eq!(cards(board, "c2"), ["e", "b", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_gap_opens_at_the_place_closes_on_leave_and_not_at_a_refused_place() {
    let board = mount(Options {
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    let a = point(&item("a"), 0.5, 0.5);
    press(a);
    move_to((a.0 + 8, a.1 + 2));
    move_to(point(&item("e"), 0.5, 0.75));
    assert_eq!((slid("e"), slid("f")), (0.0, STEP));
    assert_eq!(count("rinch-sortable__item--drop-after"), 0, "no caret");
    move_to((590, 590));
    assert_eq!((slid("e"), slid("f")), (0.0, 0.0));
    let over = point(&item("c"), 0.5, 0.75);
    move_to(over);
    assert_eq!(slid("d"), STEP);
    release(over);
    assert_eq!(cards(board, "c1"), ["b", "c", "a", "d"]);
    assert_clean();

    // d into c2 is refused: no gap, the refused caret and the reason.
    let d = point(&item("d"), 0.5, 0.5);
    press(d);
    move_to((d.0 + 8, d.1 + 2));
    move_to(point(&item("e"), 0.5, 0.75));
    assert_eq!((slid("e"), slid("f")), (0.0, 0.0));
    assert!(item("e").class_name().contains("--refused"));
    let reason = document()
        .query_selector(".rinch-sortable__reason--shown")
        .unwrap()
        .expect("the reason");
    assert_eq!(reason.text_content().as_deref(), Some("No d in c2"));
    // Released off every list: nothing moves.
    move_to((590, 590));
    release((590, 590));
    assert_eq!(cards(board, "c2"), ["e", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_collapsed_source_closes_up_off_the_lists() {
    let board = mount(Options {
        source: SortSource::Collapse,
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    let b = point(&item("b"), 0.5, 0.5);
    press(b);
    move_to((b.0 + 8, b.1 + 2));
    assert!(
        item("b")
            .class_name()
            .contains("rinch-sortable__item--collapsed")
    );
    move_to((590, 590));
    assert_eq!((slid("a"), slid("c"), slid("d")), (0.0, -STEP, -STEP));
    let over = point(&item("f"), 0.5, 0.2);
    move_to(over);
    assert_eq!((slid("c"), slid("e"), slid("f")), (-STEP, 0.0, STEP));
    release(over);
    assert_eq!(cards(board, "c2"), ["e", "b", "f"]);
    assert_clean();
}

#[wasm_bindgen_test]
fn a_keyboard_lift_moves_the_gap() {
    let board = mount(Options {
        indicator: SortIndicator::Gap,
        ..Options::default()
    });
    item("a").focus().unwrap();
    assert!(key(" "));
    assert!(ghost().is_none(), "no pointer, no preview");
    assert!(sortable_drag().unwrap().keyboard);
    assert!(key("ArrowDown"));
    assert_eq!((slid("b"), slid("c"), slid("d")), (0.0, STEP, STEP));
    assert!(key("ArrowDown"));
    assert_eq!((slid("c"), slid("d")), (0.0, STEP));
    assert!(key("Escape"));
    assert_clean();
    item("a").focus().unwrap();
    assert!(key(" "));
    assert!(key("ArrowDown"));
    assert!(key(" "));
    assert_eq!(cards(board, "c1"), ["b", "a", "c", "d"]);
    assert_clean();
    let seen = SEEN.with(|s| s.borrow().clone());
    assert!(
        seen.iter().flatten().any(|s| s.keyboard
            && s.target
                == Some(SortTarget {
                    list: "c1".into(),
                    index: 1
                })),
        "{seen:?}"
    );
}
