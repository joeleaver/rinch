//! Auto-scroll during an element drag in the browser: a drag whose pointer
//! rests in the band along the top or bottom edge of a scroll container under
//! it scrolls that container from `requestAnimationFrame` until the pointer
//! leaves the band or the drag ends, and the row scrolled under the still
//! pointer hears `data-ondragenter` and `data-ondragover`. Desktop's
//! counterpart is `crates/rinch/src/app/drag_auto_scroll_tests.rs`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test drag_auto_scroll
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const ROWS: usize = 20;
const ROW_HEIGHT: i32 = 40;
/// The list's box in the viewport: (0,100) to (300,300).
const LIST_TOP: i32 = 100;
const LIST_BOTTOM: i32 = 300;
const X: i32 = 100;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    host: web_sys::Element,
    root: rinch_web::RootHandle,
    list: web_sys::Element,
    source: web_sys::Element,
    /// Row index of each `data-ondragover`, in order.
    over: Rc<RefCell<Vec<usize>>>,
    /// Row index of each `data-ondragenter`, in order.
    entered: Rc<RefCell<Vec<usize>>>,
    drops: Rc<Cell<u32>>,
}

impl Fixture {
    /// A fixed 300x200 `overflow-y: auto` list at (0,100) of twenty 40px rows,
    /// each a drop target, and a fixed draggable source at (500,40).
    fn mount() -> Self {
        let over: Rc<RefCell<Vec<usize>>> = Rc::default();
        let entered: Rc<RefCell<Vec<usize>>> = Rc::default();
        let drops = Rc::new(Cell::new(0u32));
        let (o, e, d) = (over.clone(), entered.clone(), drops.clone());
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrap = scope.create_element("div");
                let source = scope.create_element("div");
                source.set_attribute("id", "das-src");
                source.set_attribute("draggable", "true");
                source.set_attribute(
                    "style",
                    "position: fixed; left: 500px; top: 40px; width: 100px; height: 40px",
                );
                wrap.append_child(&source);
                let list = scope.create_element("div");
                list.set_attribute("id", "das-list");
                list.set_attribute(
                    "style",
                    &format!(
                        "position: fixed; left: 0px; top: {LIST_TOP}px; width: 300px; \
                         height: {}px; overflow-y: auto; margin: 0; padding: 0",
                        LIST_BOTTOM - LIST_TOP
                    ),
                );
                for i in 0..ROWS {
                    let row = scope.create_element("div");
                    row.set_attribute("style", &format!("height: {ROW_HEIGHT}px"));
                    let d = d.clone();
                    let drop = scope.register_handler(move || d.set(d.get() + 1));
                    row.set_attribute("data-ondrop", &drop.0.to_string());
                    let o = o.clone();
                    let id = scope.register_handler(move || o.borrow_mut().push(i));
                    row.set_attribute("data-ondragover", &id.0.to_string());
                    let e = e.clone();
                    let id = scope.register_handler(move || e.borrow_mut().push(i));
                    row.set_attribute("data-ondragenter", &id.0.to_string());
                    list.append_child(&row);
                }
                wrap.append_child(&list);
                wrap
            },
        );
        let el = |id: &str| -> web_sys::Element {
            document()
                .get_element_by_id(id)
                .unwrap()
                .dyn_into()
                .unwrap()
        };
        Fixture {
            list: el("das-list"),
            source: el("das-src"),
            host,
            root,
            over,
            entered,
            drops,
        }
    }

    fn teardown(self) {
        // A failed assertion leaves a drag in flight; end it before the next
        // test's fixture.
        self.release(X, 50);
        self.root.unmount();
        self.host.remove();
    }

    /// Press the source and cross the mouse threshold.
    fn start_drag(&self) {
        pointer(&self.source, "pointerdown", 520, 55);
        pointer(&self.source, "pointermove", 540, 70);
    }

    /// A move to `(x, y)`, dispatched at the element there as a browser would.
    fn move_to(&self, x: i32, y: i32) {
        let target = document()
            .element_from_point(x as f32, y as f32)
            .expect("something is under the pointer");
        pointer(&target, "pointermove", x, y);
    }

    fn release(&self, x: i32, y: i32) {
        let target = document()
            .element_from_point(x as f32, y as f32)
            .unwrap_or_else(|| self.source.clone());
        pointer(&target, "pointerup", x, y);
    }
}

fn pointer(el: &web_sys::Element, name: &str, x: i32, y: i32) {
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

/// Resolve after `ms` of wall clock (animation frames run meanwhile).
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
async fn a_pointer_resting_in_the_bottom_band_scrolls_and_the_new_row_hears_dragover() {
    let f = Fixture::mount();
    f.start_drag();
    let y = LIST_BOTTOM - 2;
    f.move_to(X, y);
    let first_row = ((y - LIST_TOP) / ROW_HEIGHT) as usize;
    assert_eq!(f.entered.borrow().as_slice(), &[first_row]);
    assert!(
        rinch_web::__drag_auto_scroll_running(),
        "the band armed the loop"
    );

    // No further move: the frames alone scroll.
    let mut top = 0;
    for _ in 0..40 {
        sleep_ms(25).await;
        top = f.list.scroll_top();
        if top >= ROW_HEIGHT {
            break;
        }
    }
    assert!(top >= ROW_HEIGHT, "the list scrolled a row or more: {top}");
    let under = ((y - LIST_TOP + f.list.scroll_top()) / ROW_HEIGHT) as usize;
    assert!(under > first_row);
    assert!(
        f.entered.borrow().contains(&under),
        "the row scrolled under the pointer heard dragenter: {:?}",
        f.entered.borrow()
    );
    let last_over = *f.over.borrow().last().unwrap();
    assert!(
        last_over >= under,
        "dragover reached the row under the pointer: {last_over} vs {under}"
    );

    // The drop ends it, and lands on the row now under the pointer.
    f.release(X, y);
    assert_eq!(f.drops.get(), 1);
    assert!(
        !rinch_web::__drag_auto_scroll_running(),
        "the drop stopped the loop"
    );
    let dropped_at = f.list.scroll_top();
    sleep_ms(100).await;
    assert_eq!(
        f.list.scroll_top(),
        dropped_at,
        "nothing scrolls after the drop"
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn the_top_band_scrolls_up() {
    let f = Fixture::mount();
    f.list.set_scroll_top(300);
    f.start_drag();
    f.move_to(X, LIST_TOP + 4);
    sleep_ms(150).await;
    assert!(f.list.scroll_top() < 300, "{}", f.list.scroll_top());
    f.teardown();
}

#[wasm_bindgen_test]
async fn the_middle_and_the_end_do_not_scroll() {
    let f = Fixture::mount();
    f.start_drag();
    f.move_to(X, (LIST_TOP + LIST_BOTTOM) / 2);
    sleep_ms(100).await;
    assert_eq!(f.list.scroll_top(), 0, "the middle of the list is still");
    assert!(
        !rinch_web::__drag_auto_scroll_running(),
        "and the loop stopped itself"
    );
    f.teardown();

    let f = Fixture::mount();
    let end = f.list.scroll_height() - f.list.client_height();
    f.list.set_scroll_top(end);
    f.start_drag();
    f.move_to(X, LIST_BOTTOM - 4);
    sleep_ms(100).await;
    assert_eq!(
        f.list.scroll_top(),
        end,
        "a list at its end scrolls no further"
    );
    assert!(!rinch_web::__drag_auto_scroll_running());
    f.teardown();
}

#[wasm_bindgen_test]
async fn leaving_the_band_stops_it() {
    let f = Fixture::mount();
    f.start_drag();
    f.move_to(X, LIST_BOTTOM - 4);
    sleep_ms(100).await;
    assert!(f.list.scroll_top() > 0);
    f.move_to(X, (LIST_TOP + LIST_BOTTOM) / 2);
    sleep_ms(50).await;
    let left_at = f.list.scroll_top();
    sleep_ms(100).await;
    assert_eq!(f.list.scroll_top(), left_at);
    assert!(!rinch_web::__drag_auto_scroll_running());
    f.teardown();
}

// ── Review fixtures (PR #1592 review) ──────────────────────────────────────

/// A 3000px-tall page with the fixture's fixed list and source; `lock`
/// applies rinch's own `lock_scroll` (NodeHandle::set_scroll_locked).
fn mount_tall_page(lock: bool, list_height: &'static str) -> (web_sys::Element, rinch_web::RootHandle) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrap = scope.create_element("div");
            wrap.set_attribute("style", "height: 3000px");
            let source = scope.create_element("div");
            source.set_attribute("id", "das-src");
            source.set_attribute("draggable", "true");
            source.set_attribute(
                "style",
                "position: fixed; left: 500px; top: 40px; width: 100px; height: 40px",
            );
            wrap.append_child(&source);
            let list = scope.create_element("div");
            list.set_attribute("id", "das-list");
            list.set_attribute(
                "style",
                &format!(
                    "position: fixed; left: 0px; top: {LIST_TOP}px; width: 300px; \
                     height: {list_height}; overflow-y: auto; margin: 0; padding: 0"
                ),
            );
            for _ in 0..ROWS {
                let row = scope.create_element("div");
                row.set_attribute("style", &format!("height: {ROW_HEIGHT}px"));
                list.append_child(&row);
            }
            wrap.append_child(&list);
            if lock {
                list.set_scroll_locked(true);
            }
            wrap
        },
    );
    (host, root)
}

fn scroll_y() -> f64 {
    web_sys::window().unwrap().scroll_y().unwrap()
}

/// W1: a drag under a `lock_scroll` overlay must not scroll the page behind
/// it (desktop skips locked containers; the web's lock is `overflow: hidden`
/// on <html>, which programmatic scrollTop writes ignore).
#[wasm_bindgen_test]
async fn review_a_locked_page_is_not_auto_scrolled() {
    for lock in [false, true] {
        web_sys::window().unwrap().scroll_to_with_x_and_y(0.0, 0.0);
        let (host, root) = mount_tall_page(lock, "200px");
        let vh = web_sys::window().unwrap().inner_height().unwrap().as_f64().unwrap() as i32;
        let source = document().get_element_by_id("das-src").unwrap();
        pointer(&source, "pointerdown", 520, 55);
        pointer(&source, "pointermove", 540, 70);
        let target = document().element_from_point(600.0, (vh - 2) as f32).unwrap();
        pointer(&target, "pointermove", 600, vh - 2);
        sleep_ms(200).await;
        let y = scroll_y();
        pointer(&target, "pointerup", 600, vh - 2);
        root.unmount();
        host.remove();
        rinch_web::__reset_scroll_lock();
        web_sys::window().unwrap().scroll_to_with_x_and_y(0.0, 0.0);
        if lock {
            assert_eq!(y, 0.0, "the locked page scrolled behind the overlay");
        } else {
            assert!(y > 0.0, "positive control: an unlocked page scrolls ({y})");
        }
    }
}

/// W2: a list whose scroll range ends between pixels: at its end the loop
/// must stop (and the page take over), not spin for ever.
#[wasm_bindgen_test]
async fn review_a_list_ending_between_pixels_lets_the_loop_stop() {
    web_sys::window().unwrap().scroll_to_with_x_and_y(0.0, 0.0);
    let (host, root) = mount_tall_page(false, "200.4px");
    let list = document().get_element_by_id("das-list").unwrap();
    let source = document().get_element_by_id("das-src").unwrap();
    list.set_scroll_top(10_000);
    let precise = js_sys::Reflect::get(&list, &"scrollTop".into()).unwrap().as_f64().unwrap();
    pointer(&source, "pointerdown", 520, 55);
    pointer(&source, "pointermove", 540, 70);
    let y = LIST_TOP + 198;
    let target = document().element_from_point(X as f32, y as f32).unwrap();
    pointer(&target, "pointermove", X, y);
    sleep_ms(300).await;
    let running = rinch_web::__drag_auto_scroll_running();
    let info = format!(
        "scrollTop {precise} (i32 {}), scrollHeight {}, clientHeight {}",
        list.scroll_top(),
        list.scroll_height(),
        list.client_height()
    );
    pointer(&target, "pointerup", X, y);
    root.unmount();
    host.remove();
    assert!(!running, "the loop still runs at the list's end: {info}");
}

/// Who holds the page's scroll lock in [`mount_lock_case`].
#[derive(Clone, Copy, Debug)]
enum LockCase {
    /// Nobody: the positive control.
    None,
    /// A separate overlay; the list is outside it.
    Overlay,
    /// A separate overlay, and the list carries `data-scroll-lock-exempt`.
    OverlayListExempt,
    /// The list itself is the locking root.
    List,
}

/// The fixture's list and source, plus an empty fixed overlay, with the lock
/// held as `case` says.
fn mount_lock_case(case: LockCase) -> (web_sys::Element, rinch_web::RootHandle) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrap = scope.create_element("div");
            let source = scope.create_element("div");
            source.set_attribute("id", "das-src");
            source.set_attribute("draggable", "true");
            source.set_attribute(
                "style",
                "position: fixed; left: 500px; top: 40px; width: 100px; height: 40px",
            );
            wrap.append_child(&source);
            let overlay = scope.create_element("div");
            overlay.set_attribute(
                "style",
                "position: fixed; left: 700px; top: 0px; width: 10px; height: 10px",
            );
            wrap.append_child(&overlay);
            let list = scope.create_element("div");
            list.set_attribute("id", "das-list");
            list.set_attribute(
                "style",
                &format!(
                    "position: fixed; left: 0px; top: {LIST_TOP}px; width: 300px; \
                     height: 200px; overflow-y: auto; margin: 0; padding: 0"
                ),
            );
            for _ in 0..ROWS {
                let row = scope.create_element("div");
                row.set_attribute("style", &format!("height: {ROW_HEIGHT}px"));
                list.append_child(&row);
            }
            wrap.append_child(&list);
            match case {
                LockCase::None => {}
                LockCase::Overlay => overlay.set_scroll_locked(true),
                LockCase::OverlayListExempt => {
                    list.set_attribute("data-scroll-lock-exempt", "");
                    overlay.set_scroll_locked(true);
                }
                LockCase::List => list.set_scroll_locked(true),
            }
            wrap
        },
    );
    (host, root)
}

/// While the page is locked, a list outside the locking overlay is not
/// auto-scrolled; one inside it (the root itself), or one marked
/// `data-scroll-lock-exempt`, is — desktop's `scroll_locked_out` rule.
#[wasm_bindgen_test]
async fn a_lock_stops_containers_outside_the_overlay_unless_exempt() {
    for (case, scrolls) in [
        (LockCase::None, true),
        (LockCase::Overlay, false),
        (LockCase::OverlayListExempt, true),
        (LockCase::List, true),
    ] {
        let (host, root) = mount_lock_case(case);
        let list = document().get_element_by_id("das-list").unwrap();
        let source = document().get_element_by_id("das-src").unwrap();
        pointer(&source, "pointerdown", 520, 55);
        pointer(&source, "pointermove", 540, 70);
        let y = LIST_TOP + 198;
        let target = document().element_from_point(X as f32, y as f32).unwrap();
        pointer(&target, "pointermove", X, y);
        sleep_ms(200).await;
        let top = list.scroll_top();
        pointer(&target, "pointerup", X, y);
        root.unmount();
        host.remove();
        rinch_web::__reset_scroll_lock();
        assert_eq!(top > 0, scrolls, "{case:?}: the list is at {top}");
    }
}
