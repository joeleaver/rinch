//! A document-scoped slot is per root on the web (issue #2811): the keyboard
//! interceptor, the paste interceptor, the selection callback and the
//! selection-sync callback a root's code installs are that root's document's,
//! as a desktop window's are its document's.
//!
//! For each slot kind, two roots on one page:
//! - each root's install serves its own events, not the other's;
//! - a clear made by one root's code (its `on_cleanup` as it is unmounted, or
//!   one of its effects) leaves the other root's install in place.
//!
//! Before #2811 rinch-web marked no document while a root's code ran, so each
//! slot held one page-wide entry: the second root's install replaced the
//! first's for both, and any root's clear removed it for every root.
//!
//! Keys and pastes are real DOM events dispatched on an element inside the
//! root, so they go through the page's delegated listeners; the selection
//! slots have no event of their own on the web, and are read by a root's own
//! code (an effect), as an app reads them.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test per_root_slots_2811
//! ```
#![cfg(target_arch = "wasm32")]

#[path = "support/clipboard.rs"]
mod clipboard;

use rinch_core::element::ThemeProviderProps;
use rinch_core::events::{self, KeyEventData, PasteEventData, SelectionAction};
use rinch_core::reactive::{Effect, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

type Log = Rc<RefCell<Vec<String>>>;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

/// A mounted root and the browser element its events are dispatched on.
struct Root {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    target: web_sys::Element,
}

impl Root {
    /// Mount a root whose build runs `install`, then answers a `<textarea>`
    /// (the element events are dispatched on). `cleanup` is registered as the
    /// root's `on_cleanup`.
    fn mount(install: impl FnOnce() + 'static, cleanup: Option<fn()>) -> Self {
        let host = host();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
            install();
            if let Some(cleanup) = cleanup {
                scope.on_cleanup(cleanup);
            }
            scope.create_element("textarea")
        });
        let target = host.first_element_child().expect("the root's textarea");
        Root { root, host, target }
    }

    fn key(&self, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.target.dispatch_event(&ev).unwrap();
    }

    fn paste(&self, text: &str) {
        let dt = web_sys::DataTransfer::new().unwrap();
        dt.set_data("text/plain", text).unwrap();
        let ev = clipboard::clipboard_event("paste", &dt);
        self.target.dispatch_event(&ev).unwrap();
    }

    fn unmount(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn keyboard_interceptor(log: &Log, name: &'static str) -> impl FnOnce() + 'static {
    let log = log.clone();
    move || {
        events::set_keyboard_interceptor(move |k: &KeyEventData| {
            log.borrow_mut().push(format!("{name}:{}", k.key));
            false
        });
    }
}

fn paste_interceptor(log: &Log, name: &'static str) -> impl FnOnce() + 'static {
    let log = log.clone();
    move || {
        events::set_paste_interceptor(move |p: &PasteEventData| {
            log.borrow_mut()
                .push(format!("{name}:{}", p.text.clone().unwrap_or_default()));
            false
        });
    }
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

// ---------------------------------------------------------------------------
// The keyboard interceptor
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn each_roots_keyboard_interceptor_hears_its_own_keys() {
    let log: Log = Default::default();
    let k = Root::mount(keyboard_interceptor(&log, "K"), None);
    let g = Root::mount(keyboard_interceptor(&log, "G"), None);
    k.key("a");
    g.key("b");
    let heard = taken(&log);
    k.unmount();
    g.unmount();
    assert_eq!(
        heard,
        ["K:a", "G:b"],
        "each root's interceptor, its own keys"
    );
}

#[wasm_bindgen_test]
fn a_roots_keyboard_interceptor_survives_anothers_cleanup_clear() {
    let log: Log = Default::default();
    let k = Root::mount(keyboard_interceptor(&log, "K"), None);
    let g = Root::mount(|| {}, Some(events::clear_keyboard_interceptor));
    k.key("a");
    assert_eq!(
        taken(&log),
        ["K:a"],
        "control: K's interceptor hears K's key"
    );
    g.unmount();
    k.key("b");
    let heard = taken(&log);
    k.unmount();
    assert_eq!(heard, ["K:b"], "G's clear left K's interceptor");
}

// ---------------------------------------------------------------------------
// The paste interceptor
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn each_roots_paste_interceptor_hears_its_own_pastes() {
    let log: Log = Default::default();
    let k = Root::mount(paste_interceptor(&log, "K"), None);
    let g = Root::mount(paste_interceptor(&log, "G"), None);
    k.paste("one");
    g.paste("two");
    let heard = taken(&log);
    k.unmount();
    g.unmount();
    assert_eq!(
        heard,
        ["K:one", "G:two"],
        "each root's interceptor, its own pastes"
    );
}

/// The issue's reproduction (#2810's two-root test): G's cleanup clears the
/// paste interceptor as G is unmounted, and K's stays.
#[wasm_bindgen_test]
fn a_roots_paste_interceptor_survives_anothers_cleanup_clear() {
    let log: Log = Default::default();
    let k = Root::mount(paste_interceptor(&log, "K"), None);
    let g = Root::mount(|| {}, Some(events::clear_paste_interceptor));
    k.paste("one");
    assert_eq!(
        taken(&log),
        ["K:one"],
        "control: K's interceptor hears K's paste"
    );
    g.unmount();
    k.paste("two");
    let heard = taken(&log);
    k.unmount();
    assert_eq!(heard, ["K:two"], "G's clear left K's interceptor");
}

/// A clear from one of G's effects (G's code, run while G stays mounted)
/// leaves K's install too.
#[wasm_bindgen_test]
fn a_roots_paste_interceptor_survives_a_clear_from_anothers_effect() {
    let log: Log = Default::default();
    let clear = Signal::new(false);
    let k = Root::mount(paste_interceptor(&log, "K"), None);
    let g = Root::mount(
        move || {
            Effect::new(move || {
                if clear.get() {
                    events::clear_paste_interceptor();
                }
            });
        },
        None,
    );
    k.paste("one");
    assert_eq!(
        taken(&log),
        ["K:one"],
        "control: K's interceptor hears K's paste"
    );
    clear.set(true);
    k.paste("two");
    let heard = taken(&log);
    k.unmount();
    g.unmount();
    assert_eq!(heard, ["K:two"], "G's effect's clear left K's interceptor");
}

// ---------------------------------------------------------------------------
// The selection callback and the selection-sync callback
// ---------------------------------------------------------------------------

/// A root whose build installs a selection callback answering `range`, a
/// selection-sync callback logging what it is given, and an effect that
/// queries the selection and fires the sync each time `ask` changes (a root's
/// own code reading its slots).
fn selection_root(log: &Log, name: &'static str, range: u64, ask: Signal<u32>) -> Root {
    let log = log.clone();
    Root::mount(
        move || {
            events::set_selection_callback(move |_: SelectionAction| vec![(range, 0, 1)]);
            let synced = log.clone();
            events::set_selection_sync_callback(move |ranges| {
                synced.borrow_mut().push(format!("{name} sync:{ranges:?}"));
            });
            Effect::new(move || {
                if ask.get() > 0 {
                    let ranges = events::dispatch_selection(SelectionAction::QueryRanges);
                    log.borrow_mut().push(format!("{name} query:{ranges:?}"));
                    events::fire_selection_sync();
                }
            });
        },
        None,
    )
}

#[wasm_bindgen_test]
fn each_roots_selection_callbacks_answer_its_own_code() {
    let log: Log = Default::default();
    let ask = Signal::new(0u32);
    let k = selection_root(&log, "K", 1, ask);
    let g = selection_root(&log, "G", 2, ask);
    ask.set(1);
    let mut heard = taken(&log);
    heard.sort();
    k.unmount();
    g.unmount();
    assert_eq!(
        heard,
        [
            "G query:[(2, 0, 1)]",
            "G sync:[(2, 0, 1)]",
            "K query:[(1, 0, 1)]",
            "K sync:[(1, 0, 1)]",
        ],
        "each root's selection callback and sync callback, for its own code"
    );
}

fn clear_selection_slots() {
    events::clear_selection_callback();
    events::clear_selection_sync_callback();
}

#[wasm_bindgen_test]
fn a_roots_selection_callbacks_survive_anothers_cleanup_clear() {
    let log: Log = Default::default();
    let ask = Signal::new(0u32);
    let k = selection_root(&log, "K", 1, ask);
    let g = Root::mount(|| {}, Some(clear_selection_slots));
    ask.set(1);
    assert_eq!(
        taken(&log),
        ["K query:[(1, 0, 1)]", "K sync:[(1, 0, 1)]"],
        "control: K's callbacks answer K's code"
    );
    g.unmount();
    ask.set(2);
    let heard = taken(&log);
    k.unmount();
    assert_eq!(
        heard,
        ["K query:[(1, 0, 1)]", "K sync:[(1, 0, 1)]"],
        "G's clear left K's callbacks"
    );
}

/// Positive control that the root's textarea is what the events target: a
/// key and a paste dispatched on it reach rinch's delegated listeners (a
/// page-wide interceptor installed outside any root hears them).
#[wasm_bindgen_test]
fn control_events_on_a_roots_textarea_reach_rinch() {
    let log: Log = Default::default();
    let r = Root::mount(|| {}, None);
    keyboard_interceptor(&log, "page")();
    paste_interceptor(&log, "page")();
    r.key("x");
    r.paste("y");
    let heard = taken(&log);
    r.unmount();
    events::clear_keyboard_interceptor();
    events::clear_paste_interceptor();
    assert_eq!(heard, ["page:x", "page:y"]);
}

/// App keys stay the page's (#1784's web rule, kept by #2811): a listener
/// one root's code registers hears a key typed in another root, as a
/// `window` key listener would.
#[wasm_bindgen_test]
fn an_app_key_hears_another_roots_keys() {
    let log: Log = Default::default();
    let heard_by = log.clone();
    let k = Root::mount(
        move || {
            rinch_core::on_app_key(move |key: &KeyEventData| {
                heard_by.borrow_mut().push(format!("K app:{}", key.key));
                false
            });
        },
        None,
    );
    let g = Root::mount(|| {}, None);
    g.key("q");
    k.key("w");
    let heard = taken(&log);
    k.unmount();
    g.unmount();
    assert_eq!(heard, ["K app:q", "K app:w"]);
}

// ---------------------------------------------------------------------------
// Clicks dispatched at the release (a draggable, a touch tap in a scroller)
// ---------------------------------------------------------------------------

fn pointer(el: &web_sys::Element, name: &str, kind: &str) {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(7);
    init.set_is_primary(true);
    init.set_pointer_type(kind);
    init.set_button(0);
    init.set_buttons(if name == "pointerup" { 0 } else { 1 });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A root G whose clickable `#{id}` clears the paste interceptor from its
/// click handler. `scroller` puts it inside a scroll container (a touch tap
/// there clicks at the release); otherwise it is `draggable` (any press
/// there clicks at the release). Answers G and the clickable, and a count of
/// the handler's runs.
fn clearing_clickable(scroller: bool) -> (Root, web_sys::Element, Rc<std::cell::Cell<u32>>) {
    let host = host();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 300px",
    )
    .unwrap();
    let runs = Rc::new(std::cell::Cell::new(0u32));
    let seen = runs.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let outer = scope.create_element("div");
        let button = scope.create_element("div");
        button.set_attribute("style", "width: 100px; height: 30px");
        button.set_attribute("data-clear-2811", "");
        let id = scope.register_handler(move || {
            seen.set(seen.get() + 1);
            events::clear_paste_interceptor();
        });
        button.set_attribute("data-rid", &id.0.to_string());
        if scroller {
            outer.set_attribute("style", "overflow: auto; height: 40px; width: 200px");
            let tall = scope.create_element("div");
            tall.set_attribute("style", "height: 400px");
            tall.append_child(&button);
            outer.append_child(&tall);
        } else {
            button.set_attribute("draggable", "true");
            outer.append_child(&button);
        }
        outer
    });
    let target = host.query_selector("[data-clear-2811]").unwrap().unwrap();
    let el = target.clone();
    (Root { root, host, target }, el, runs)
}

fn a_release_click_in_another_root_leaves_the_interceptor(scroller: bool, kind: &str) {
    rinch_web::__reset_activation_state();
    let log: Log = Default::default();
    let k = Root::mount(paste_interceptor(&log, "K"), None);
    let (g, clickable, runs) = clearing_clickable(scroller);
    pointer(&clickable, "pointerdown", kind);
    assert_eq!(runs.get(), 0, "control: the click waits for the release");
    pointer(&clickable, "pointerup", kind);
    let ran = runs.get();
    k.paste("one");
    let heard = taken(&log);
    k.unmount();
    g.unmount();
    assert_eq!(ran, 1, "control: G's handler ran at the release");
    assert_eq!(
        heard,
        ["K:one"],
        "G's click handler's clear left K's interceptor"
    );
}

#[wasm_bindgen_test]
fn a_draggables_click_in_another_root_leaves_the_interceptor() {
    a_release_click_in_another_root_leaves_the_interceptor(false, "mouse");
}

#[wasm_bindgen_test]
fn a_touch_tap_in_another_roots_scroller_leaves_the_interceptor() {
    a_release_click_in_another_root_leaves_the_interceptor(true, "touch");
}
