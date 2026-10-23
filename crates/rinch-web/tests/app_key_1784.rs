//! Browser-driven tests for issue #1784 on the web backend: app-wide key
//! listeners (`on_app_key`) hear every key that nothing ahead of them
//! consumed — with nothing focused, with a field focused, and with a render
//! surface focused for the keys it does not claim — in registration order; a
//! consumed press is `preventDefault`ed and stops propagating.
//!
//! Every fixture mounts through `rinch_web::mount_into`, which installs the
//! document key delegation; the listener a mounted component registers is the
//! positive control that a genuine event reached rinch.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test app_key_1784
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, create_render_surface, set_focused_surface,
};
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::app_key_listener_count;
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

const HOST_MARKER: &str = "data-app-key-1784-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    surface: RenderSurfaceHandle,
    show: Signal<bool>,
    branch_log: Log,
}

fn listening_branch(__scope: &mut RenderScope, log: Log) -> NodeHandle {
    let l = log.clone();
    on_app_key(move |k| {
        l.borrow_mut().push(format!("branch:{}", k.key));
        false
    });
    rsx! { span { "listening" } }
}

impl Fixture {
    fn mount() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let branch_log: Log = Default::default();
        let surface = create_render_surface();
        let show = Signal::new(true);
        let (bl, sf) = (branch_log.clone(), surface.clone());
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |__scope: &mut RenderScope| {
                let bl = bl.clone();
                let node = rsx! {
                    div {
                        input { id: "ak-in", oninput: move |_v: String| {} }
                        div {
                            id: "ak-tile",
                            tabindex: "0",
                            style: "width: 50px; height: 20px",
                            onkeydown: move |k| k.key == "F2",
                        }
                        if show.get() {
                            {listening_branch(__scope, bl.clone())}
                        }
                    }
                };
                let canvas = RenderSurface {
                    surface: Some(sf.clone()),
                }
                .render(__scope, &[]);
                node.append_child(&canvas);
                node
            },
        );
        Self {
            root,
            host,
            surface,
            show,
            branch_log,
        }
    }

    fn teardown(self) {
        set_focused_surface(None);
        rinch_core::clear_keyboard_interceptor();
        self.root.unmount();
        self.host.remove();
    }
}

fn focus(id: &str) {
    document()
        .get_element_by_id(id)
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .focus()
        .unwrap();
}

fn blur_all() {
    if let Some(el) = document().active_element()
        && let Ok(h) = el.dyn_into::<web_sys::HtmlElement>()
    {
        h.blur().unwrap();
    }
}

/// A key event where the browser sends it — the focused element, or `<body>`
/// with nothing focused. Answers whether it was cancelled.
fn key(kind: &str, key: &str, code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
    let target: web_sys::EventTarget = document()
        .active_element()
        .unwrap_or_else(|| document().body().unwrap().into())
        .into();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

fn press(k: &str, code: &str) -> bool {
    let prevented = key("keydown", k, code);
    key("keyup", k, code);
    prevented
}

fn listen(tag: &'static str, log: &Log, takes: &'static [&'static str]) -> AppKeyListener {
    let l = log.clone();
    on_app_key(move |k: &KeyEventData| {
        l.borrow_mut().push(format!("{tag}:{:?}:{}", k.kind, k.key));
        takes.contains(&k.key.as_str())
    })
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// A `window` keydown listener: what a consumed key must not reach.
struct WindowHits {
    hits: Rc<RefCell<u32>>,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl WindowHits {
    fn install() -> Self {
        let hits = Rc::new(RefCell::new(0));
        let h = hits.clone();
        let closure = Closure::wrap(Box::new(move |_e: web_sys::Event| {
            *h.borrow_mut() += 1;
        }) as Box<dyn FnMut(_)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
            .unwrap();
        Self { hits, closure }
    }
    fn hits(&self) -> u32 {
        *self.hits.borrow()
    }
    fn remove(self) {
        web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("keydown", self.closure.as_ref().unchecked_ref())
            .unwrap();
    }
}

/// With nothing focused a listener hears the press and the release (the
/// mounted branch's listener is the positive control: it hears them first),
/// and a consumed press is cancelled and kept from `window`.
#[wasm_bindgen_test]
fn with_nothing_focused_listeners_hear_presses_and_releases() {
    let f = Fixture::mount();
    blur_all();
    let log: Log = Default::default();
    let app = listen("app", &log, &["Delete"]);
    let win = WindowHits::install();

    assert!(!press("z", "KeyZ"), "an unconsumed key is not cancelled");
    assert_eq!(take(&f.branch_log), ["branch:z", "branch:z"]);
    assert_eq!(take(&log), ["app:Down:z", "app:Up:z"]);
    assert_eq!(win.hits(), 1);

    assert!(press("Delete", "Delete"), "a consumed press is cancelled");
    assert_eq!(take(&log), ["app:Down:Delete", "app:Up:Delete"]);
    assert_eq!(win.hits(), 1, "and stopped before window");

    win.remove();
    app.remove();
    f.teardown();
}

/// Registration order; a consumed press stops the later listener, the release
/// reaches it anyway.
#[wasm_bindgen_test]
fn listeners_hear_in_order_and_a_consumed_press_stops_the_later_one() {
    let f = Fixture::mount();
    blur_all();
    let log = f.branch_log.clone();
    let a = listen("app", &log, &["Escape"]);
    let late_log: Log = Default::default();
    let late = listen("late", &late_log, &[]);
    press("q", "KeyQ");
    assert_eq!(
        take(&log),
        ["branch:q", "app:Down:q", "branch:q", "app:Up:q"]
    );
    take(&late_log);
    press("Escape", "Escape");
    assert_eq!(take(&late_log), ["late:Up:Escape"]);
    a.remove();
    late.remove();
    f.teardown();
}

/// A key in a focused field reaches the listener; a consumed one is cancelled
/// (so a trusted key would not be typed).
#[wasm_bindgen_test]
fn a_focused_field_offers_its_keys() {
    let f = Fixture::mount();
    focus("ak-in");
    let log: Log = Default::default();
    let app = listen("app", &log, &["x"]);
    assert!(!key("keydown", "a", "KeyA"));
    assert!(key("keydown", "x", "KeyX"));
    assert_eq!(take(&log), ["app:Down:a", "app:Down:x"]);
    app.remove();
    f.teardown();
}

/// Everything ahead of the listeners wins: the element's own `onkeydown`, the
/// dismiss stack and the interceptor.
#[wasm_bindgen_test]
fn earlier_stages_that_consume_a_press_keep_it_from_the_listeners() {
    let f = Fixture::mount();
    focus("ak-tile");
    let log: Log = Default::default();
    let app = listen("app", &log, &[]);

    press("F2", "F2");
    assert_eq!(take(&log), ["app:Up:F2"]);

    let dismiss = push_dismiss_handler(0, || true);
    press("Escape", "Escape");
    assert_eq!(take(&log), ["app:Up:Escape"]);
    drop(dismiss);

    rinch_core::set_keyboard_interceptor(|k| k.key == "F3");
    press("F3", "F3");
    assert_eq!(take(&log), ["app:Up:F3"]);
    press("F4", "F4");
    assert_eq!(take(&log), ["app:Down:F4", "app:Up:F4"]);
    rinch_core::clear_keyboard_interceptor();
    app.remove();
    f.teardown();
}

/// A focused render surface keeps the keys it claims, press and release; the
/// listeners hear the rest.
#[wasm_bindgen_test]
fn a_focused_surface_keeps_its_claimed_keys() {
    let f = Fixture::mount();
    blur_all();
    set_focused_surface(Some(f.surface.id()));
    f.surface.set_key_handler(|k| k.code == "KeyW");
    let log: Log = Default::default();
    let app = listen("app", &log, &[]);
    assert!(press("w", "KeyW"), "the surface claimed it");
    press("k", "KeyK");
    assert_eq!(take(&log), ["app:Down:k", "app:Up:k"]);
    app.remove();
    f.teardown();
}

/// Unmounting the component that registered a listener releases it.
#[wasm_bindgen_test]
fn unmounting_the_registering_component_releases_its_listener() {
    let f = Fixture::mount();
    blur_all();
    press("q", "KeyQ");
    assert_eq!(take(&f.branch_log), ["branch:q", "branch:q"]);
    let before = app_key_listener_count();
    f.show.set(false);
    assert_eq!(app_key_listener_count(), before - 1);
    press("q", "KeyQ");
    assert!(take(&f.branch_log).is_empty());
    f.teardown();
}
