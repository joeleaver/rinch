//! Round-2 review probes for PR #1592 (drag auto-scroll on the web): the end
//! and slow bands at a fractional devicePixelRatio, and the lock bookkeeping.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const LIST_TOP: i32 = 100;

thread_local! {
    /// The overlay node of the last `mount`, so a fixture can release its lock
    /// through the verb the overlay components use.
    static OVERLAY: std::cell::RefCell<Option<rinch_core::dom::NodeHandle>> =
        const { std::cell::RefCell::new(None) };
}
const X: i32 = 100;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
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

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn precise_top(el: &web_sys::Element) -> f64 {
    js_sys::Reflect::get(el, &"scrollTop".into())
        .unwrap()
        .as_f64()
        .unwrap()
}

/// A fixed list at (0,100) of `rows` rows of `row_h`, `list_h` tall, and a
/// fixed source; with `lock_overlay` an empty fixed overlay that holds the
/// page lock (the list is outside it).
fn mount(
    list_h: String,
    row_h: String,
    rows: usize,
    lock_overlay: bool,
) -> (web_sys::Element, rinch_web::RootHandle) {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let wrap = scope.create_element("div");
            let source = scope.create_element("div");
            source.set_attribute("id", "r2-src");
            source.set_attribute("draggable", "true");
            source.set_attribute(
                "style",
                "position: fixed; left: 500px; top: 40px; width: 100px; height: 40px",
            );
            wrap.append_child(&source);
            let list = scope.create_element("div");
            list.set_attribute("id", "r2-list");
            list.set_attribute(
                "style",
                &format!(
                    "position: fixed; left: 0px; top: {LIST_TOP}px; width: 300px; \
                     height: {list_h}; overflow-y: auto; margin: 0; padding: 0"
                ),
            );
            for _ in 0..rows {
                let row = scope.create_element("div");
                row.set_attribute("style", &format!("height: {row_h}"));
                list.append_child(&row);
            }
            wrap.append_child(&list);
            let overlay = scope.create_element("div");
            overlay.set_attribute("id", "r2-overlay");
            overlay.set_attribute(
                "style",
                "position: fixed; left: 700px; top: 0px; width: 10px; height: 10px",
            );
            wrap.append_child(&overlay);
            if lock_overlay {
                overlay.set_scroll_locked(true);
            }
            OVERLAY.with(|o| *o.borrow_mut() = Some(overlay.clone()));
            wrap
        },
    );
    (host, root)
}

fn start_at(x: i32, y: i32) -> web_sys::Element {
    let source = document().get_element_by_id("r2-src").unwrap();
    pointer(&source, "pointerdown", 520, 55);
    pointer(&source, "pointermove", 540, 70);
    let target = document().element_from_point(x as f32, y as f32).unwrap();
    pointer(&target, "pointermove", x, y);
    target
}

/// F1 at the current devicePixelRatio: the strongest band runs the list to
/// its end and the loop stops there, whatever the fractional sizes.
#[wasm_bindgen_test]
async fn r2_the_loop_stops_at_the_end_at_this_dpr() {
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let mut report = Vec::new();
    let mut spinning = Vec::new();
    for (lh, rh) in [
        ("200px", "40px"),
        ("200.4px", "40px"),
        ("200.6px", "40px"),
        ("201.3px", "40px"),
        ("200px", "40.03px"),
        ("200px", "40.02px"),
        ("199.7px", "40.017px"),
    ] {
        let (host, root) = mount(lh.into(), rh.into(), 20, false);
        let list = document().get_element_by_id("r2-list").unwrap();
        let h = list.get_bounding_client_rect().height() as i32;
        let y = LIST_TOP + h - 2;
        let target = start_at(X, y);
        sleep_ms(1500).await;
        let running = rinch_web::__drag_auto_scroll_running();
        let line = format!(
            "dpr {dpr} list {lh} rows {rh}: top {} (i32 {}) sh {} ch {} running {running}",
            precise_top(&list),
            list.scroll_top(),
            list.scroll_height(),
            list.client_height()
        );
        console_log!("{}", line);
        if running {
            spinning.push(line.clone());
        }
        report.push(line);
        pointer(&target, "pointerup", X, y);
        root.unmount();
        host.remove();
    }
    assert!(
        spinning.is_empty(),
        "spins at the end: {spinning:#?}\nall: {report:#?}"
    );
}

/// The slow band at the current devicePixelRatio: a pointer a few px inside
/// the band's inner edge scrolls slowly but does scroll.
#[wasm_bindgen_test]
async fn r2_a_slow_band_still_moves_at_this_dpr() {
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let mut stuck = Vec::new();
    for from_end in [20, 24, 26, 28] {
        let (host, root) = mount("200px".into(), "40px".into(), 20, false);
        let list = document().get_element_by_id("r2-list").unwrap();
        let y = LIST_TOP + 200 - from_end;
        let target = start_at(X, y);
        sleep_ms(2000).await;
        let line = format!(
            "dpr {dpr} {from_end}px from the end: top {} after 2s, running {}",
            precise_top(&list),
            rinch_web::__drag_auto_scroll_running()
        );
        console_log!("{}", line);
        if precise_top(&list) < 1.0 {
            stuck.push(line);
        }
        pointer(&target, "pointerup", X, y);
        root.unmount();
        host.remove();
    }
    assert!(stuck.is_empty(), "a slow band moved nothing: {stuck:#?}");
}

/// F2: the overlay holding the lock closes while the drag rests in the
/// list's band: the list (outside the overlay) may scroll from then on.
#[wasm_bindgen_test]
async fn r2_an_overlay_closed_during_the_drag_lets_the_list_scroll() {
    let (host, root) = mount("200px".into(), "40px".into(), 20, true);
    let list = document().get_element_by_id("r2-list").unwrap();
    let overlay = document().get_element_by_id("r2-overlay").unwrap();
    let y = LIST_TOP + 198;
    let target = start_at(X, y);
    sleep_ms(200).await;
    let locked_top = list.scroll_top();
    let running_locked = rinch_web::__drag_auto_scroll_running();
    // Release through the same NodeHandle verb the overlay uses.
    let _ = overlay;
    OVERLAY.with(|o| o.borrow().as_ref().unwrap().set_scroll_locked(false));
    sleep_ms(200).await;
    let top_after = list.scroll_top();
    let running_after = rinch_web::__drag_auto_scroll_running();
    rinch_web::__reset_scroll_lock();
    // A move restarts it, whatever the loop did.
    pointer(&target, "pointermove", X, y + 1);
    sleep_ms(200).await;
    let top_moved = list.scroll_top();
    let line = format!(
        "locked: top {locked_top} running {running_locked}; unlocked, still: top {top_after} running {running_after}; after a move: {top_moved}"
    );
    console_log!("{}", line);
    pointer(&target, "pointerup", X, y + 1);
    root.unmount();
    host.remove();
    assert_eq!(locked_top, 0, "{line}");
    // The unlock restarts the loop with the pointer still (#1592 round 2),
    // as desktop's next frame does.
    assert!(top_after > 0, "{line}");
    assert!(top_moved > 0, "{line}");
}
