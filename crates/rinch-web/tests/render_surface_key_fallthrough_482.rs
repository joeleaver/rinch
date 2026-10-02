//! A focused `RenderSurface` used to swallow every `keydown` unconditionally
//! (issue #482): `event.preventDefault()` + `event.stopPropagation()` ran
//! whatever the key was and whatever the surface did with it, so a host's own
//! `window`-level keybindings (and the browser's own shortcuts — reload,
//! find, …) went deaf the moment a canvas took focus. The same delegation
//! also skipped the document-level keyboard interceptor entirely whenever a
//! surface was focused (issue #484's keydown/keyup asymmetry — `keyup`
//! already reached it unconditionally).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Each fixture mounts a `RenderSurface` through `rinch_web::mount_into` —
//! that is what installs the document-level `keydown` listener under test
//! (`mount_tree` → `ensure_event_delegation`); a version of this file that
//! dispatched events without mounting anything would link none of it and
//! pass with the fix reverted. The `surface_saw` counter on every fixture is
//! the positive control: it proves a genuine `KeyboardEvent` reached rinch's
//! listener and was forwarded, so a `0` elsewhere in the same test means
//! "did not propagate", not "nothing ran".
#![cfg(target_arch = "wasm32")]

use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::{KeyEventData, clear_keyboard_interceptor, set_keyboard_interceptor};
use rinch::render_surface::{
    RenderSurface, RenderSurfaceHandle, SurfaceEvent, SurfaceKeyData, create_render_surface,
    set_focused_surface,
};
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Dispatch a bubbling, cancelable `keydown` on `document` and answer whether
/// anything prevented it. `document` is the delegation's own listener target,
/// so this reaches it in the "at target" phase; a bubbling event whose target
/// is a `Document` continues on to `window` for the bubble phase unless
/// something calls `stopPropagation`, which is exactly the behaviour under
/// test.
fn dispatch_keydown(key: &str, code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
        .expect("construct KeyboardEvent");
    document().dispatch_event(&event).unwrap();
    event.default_prevented()
}

/// A `window`-level `keydown` listener standing in for a host's own
/// `window.addEventListener('keydown', ...)` binding — the thing issue #482
/// reported going deaf. Torn down explicitly rather than `forget()`-leaked,
/// since every fixture in this file shares one page.
struct WindowListener {
    count: Rc<Cell<u32>>,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl WindowListener {
    fn install() -> Self {
        let count = Rc::new(Cell::new(0u32));
        let c = count.clone();
        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            c.set(c.get() + 1);
        }) as Box<dyn FnMut(_)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
            .unwrap();
        Self { count, closure }
    }

    fn hits(&self) -> u32 {
        self.count.get()
    }

    fn remove(self) {
        web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("keydown", self.closure.as_ref().unchecked_ref())
            .unwrap();
    }
}

struct Mounted {
    root: RootHandle,
    host: web_sys::Element,
}

impl Mounted {
    /// Mount a page whose only content is `surface`, rendered through the
    /// `RenderSurface` component exactly as an app would.
    fn new(surface: RenderSurfaceHandle) -> Self {
        let host = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                RenderSurface {
                    surface: Some(surface.clone()),
                }
                .render(scope, &[])
            },
        );
        Self { root, host }
    }

    /// Unmounting runs the component's `on_cleanup`, which unregisters the
    /// surface and tears down its canvas listeners — no explicit
    /// `unregister` call needed here.
    fn teardown(self) {
        set_focused_surface(None);
        clear_keyboard_interceptor();
        self.root.unmount();
        self.host.remove();
    }
}

/// The reported defect, directly: an unclaimed key must not be
/// `defaultPrevented`, and must keep propagating past `document` to a
/// `window`-level host listener.
#[wasm_bindgen_test]
fn an_unclaimed_key_reaches_a_window_level_host_listener() {
    let surface = create_render_surface();
    let surface_id = surface.id();
    let surface_saw = Rc::new(Cell::new(0u32));
    {
        let sink = surface_saw.clone();
        surface.set_event_handler(move |event| {
            if matches!(event, SurfaceEvent::KeyDown(_)) {
                sink.set(sink.get() + 1);
            }
        });
    }

    let mounted = Mounted::new(surface);
    set_focused_surface(Some(surface_id));
    let window_listener = WindowListener::install();

    let prevented = dispatch_keydown("a", "KeyA");

    assert_eq!(
        surface_saw.get(),
        1,
        "positive control: the surface's own handler must see the key \
         (input delivery is unaffected by issue #482's fix)"
    );
    assert!(
        !prevented,
        "an unclaimed key must not be defaultPrevented (issue #482)"
    );
    assert_eq!(
        window_listener.hits(),
        1,
        "and it must keep propagating to a window-level host listener"
    );

    window_listener.remove();
    mounted.teardown();
}

/// A surface that explicitly claims the key via `set_key_handler` must still
/// stop it there — the fix is additive, not "a focused surface never
/// swallows a key again".
#[wasm_bindgen_test]
fn a_key_the_surface_claims_still_stops_at_the_surface() {
    let surface = create_render_surface();
    let surface_id = surface.id();
    let surface_saw = Rc::new(Cell::new(0u32));
    {
        let sink = surface_saw.clone();
        surface.set_event_handler(move |event| {
            if matches!(event, SurfaceEvent::KeyDown(_)) {
                sink.set(sink.get() + 1);
            }
        });
    }
    surface.set_key_handler(|data: &SurfaceKeyData| data.key == "a");

    let mounted = Mounted::new(surface);
    set_focused_surface(Some(surface_id));
    let window_listener = WindowListener::install();

    let prevented = dispatch_keydown("a", "KeyA");

    assert_eq!(
        surface_saw.get(),
        1,
        "positive control: the surface still receives the key for input"
    );
    assert!(prevented, "a claimed key must be defaultPrevented");
    assert_eq!(
        window_listener.hits(),
        0,
        "and must not reach a window-level host listener"
    );

    window_listener.remove();
    mounted.teardown();
}

/// Issue #484's keydown leg: the document-level interceptor must see the key
/// before a focused surface does, exactly as `keyup` already did. A host's
/// `set_keyboard_interceptor` claim wins, and the surface — claiming or not —
/// must never be asked about a key the interceptor already took.
#[wasm_bindgen_test]
fn the_interceptor_sees_the_key_before_a_focused_surface_does() {
    let surface = create_render_surface();
    let surface_id = surface.id();
    let surface_saw = Rc::new(Cell::new(0u32));
    {
        let sink = surface_saw.clone();
        surface.set_event_handler(move |event| {
            if matches!(event, SurfaceEvent::KeyDown(_)) {
                sink.set(sink.get() + 1);
            }
        });
    }
    // Claims nothing, so a defect here reaching for the surface's own
    // `set_key_handler` instead would not be covered by this fixture alone —
    // the surface's `surface_saw == 0` assertion below is what distinguishes
    // "the interceptor claimed it first" from "the surface claimed it".
    surface.set_key_handler(|_| false);

    let interceptor_saw = Rc::new(Cell::new(0u32));
    {
        let sink = interceptor_saw.clone();
        set_keyboard_interceptor(move |_: &KeyEventData| {
            sink.set(sink.get() + 1);
            true
        });
    }

    let mounted = Mounted::new(surface);
    set_focused_surface(Some(surface_id));
    let window_listener = WindowListener::install();

    let prevented = dispatch_keydown("a", "KeyA");

    assert_eq!(
        interceptor_saw.get(),
        1,
        "positive control: the interceptor must be asked"
    );
    assert_eq!(
        surface_saw.get(),
        0,
        "the interceptor claimed the key; the focused surface must never see it"
    );
    assert!(prevented, "a key the interceptor claims must be defaultPrevented");
    assert_eq!(window_listener.hits(), 0);

    window_listener.remove();
    mounted.teardown();
}
