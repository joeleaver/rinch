//! Browser-driven tests for the menu bar's keyboard chords.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Two properties a unit test on the host cannot reach, because both are about
//! the *listener* rather than the registry:
//!
//! * **An unmounted island leaves nothing armed.** `mount_into_with_menu_bar`
//!   arms its chords page-globally, and `RootHandle::unmount` has to give them
//!   back — a removed widget that goes on eating `Ctrl+K` from its host page is
//!   the same bug `set_suppress_native_context_menu`'s doc argues an island must
//!   never inflict, with a different key.
//! * **A chord the menu consumed reaches no other listener.** The desktop shell
//!   `return`s before a matched chord becomes a `PlatformEvent::KeyDown`; the
//!   browser's way of saying that is to stop the event before it reaches
//!   `document`, where `editor_input`'s keymap and the bubble delegate both sit.
//!
//! `defaultPrevented` on a synthetic, cancelable `keydown` is the observable for
//! "the app claimed this key"; a counter behind the item's `on_click` is the
//! observable for "the callback ran".
//!
//! Every chord here carries **Alt** as well as Ctrl, so a stray registration
//! from another test file in this page cannot answer to it.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{Menu, MenuItem, RootHandle};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own teardown).
const HOST_MARKER: &str = "data-menubar-test-host";

fn purge_stale_hosts() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

/// Dispatch a cancelable `Ctrl+Alt+<code>` keydown on `document.body` — an
/// ordinary page element, so the event walks window → document → … → body
/// exactly as a real keystroke does. Returns whether anything called
/// `preventDefault`.
fn press(code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_code(code);
    // The *typed character* is deliberately not the code: `match_shortcut_code`
    // reads `KeyboardEvent.code`, so a fixture whose `key` disagreed would still
    // pass — and would stop failing if the listener ever switched to `key`.
    init.set_key("Unidentified");
    init.set_ctrl_key(true);
    init.set_alt_key(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    document().body().unwrap().dispatch_event(&event).unwrap();
    event.default_prevented()
}

/// An island under a menu bar whose single item carries `chord`, plus the
/// counter that item's callback increments.
struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    fired: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount(chord: &str) -> Self {
        purge_stale_hosts();
        let fired = Rc::new(Cell::new(0u32));
        let counter = fired.clone();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();

        // Built here rather than inside the render, which is the in-tree idiom
        // (`main`, before the event loop) and therefore the case with no owner:
        // the callback keeps app lifetime, so nothing but the release under test
        // can make it stop firing (#183).
        let menu = Menu::new().item(
            MenuItem::new("Go")
                .shortcut(chord)
                .on_click(move || counter.set(counter.get() + 1)),
        );
        let root = rinch_web::mount_into_with_menu_bar(
            &host,
            ThemeProviderProps::default(),
            vec![("File", menu)],
            |scope: &mut RenderScope| {
                let div = scope.create_element("div");
                let text = scope.create_text("content");
                div.append_child(&text);
                div
            },
        );
        Self { root, host, fired }
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// The positive control for [`an_unmounted_island_gives_its_chords_back`]: while
/// the island is mounted its chord both runs the item and takes the keystroke.
#[wasm_bindgen_test]
fn a_mounted_islands_chord_fires_and_is_consumed() {
    let fixture = Fixture::mount("Ctrl+Alt+K");

    let prevented = press("KeyK");

    assert_eq!(fixture.fired.get(), 1, "the item's on_click must run");
    assert!(
        prevented,
        "a chord the app claimed must not also reach the browser"
    );
    fixture.teardown();
}

/// The chords an island armed are page-global, so unmounting it must give them
/// back: otherwise the host page loses that key combination for the rest of the
/// session — the (ownerless, app-lifetime) callback still fires *and* the
/// keystroke is still suppressed.
#[wasm_bindgen_test]
fn an_unmounted_island_gives_its_chords_back() {
    let fixture = Fixture::mount("Ctrl+Alt+L");
    let fired = fixture.fired.clone();

    assert!(press("KeyL"), "control: armed while mounted");
    assert_eq!(fired.get(), 1, "control: the item ran while mounted");

    fixture.teardown();

    let prevented = press("KeyL");
    assert_eq!(
        fired.get(),
        1,
        "the unmounted island's item must not run again"
    );
    assert!(
        !prevented,
        "the unmounted island must not go on eating the host page's keystroke"
    );
}

/// Two islands share one page-global chord registry, and the second to arm wins
/// ("One page, one set of chords"). So the *first* island unmounting must leave
/// the second's chords alone: release is "if the slot is still mine", never "if
/// I ever armed".
#[wasm_bindgen_test]
fn one_island_unmounting_leaves_a_later_islands_chords_armed() {
    let first = Fixture::mount("Ctrl+Alt+M");
    let second = Fixture::mount("Ctrl+Alt+N");

    first.teardown();

    let prevented = press("KeyN");
    assert!(prevented, "the second island is still mounted and armed");
    assert_eq!(second.fired.get(), 1);

    second.teardown();
    assert!(!press("KeyN"), "and releases its own on unmount");
}

/// A `document` capture-phase `keydown` listener, standing in for the ones
/// rinch-web installs there — `editor_input`'s keymap, and the bubble delegate
/// one phase later. Registered *after* the menu bar's listener, which is the
/// ordering a same-node `stopPropagation()` cannot reach, and the one a page
/// whose first mount is a plain `mount()` actually produces.
struct Bystander {
    ran: Rc<Cell<u32>>,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl Bystander {
    fn install() -> Self {
        let ran = Rc::new(Cell::new(0u32));
        let seen = ran.clone();
        let closure = Closure::wrap(Box::new(move |_: web_sys::Event| {
            seen.set(seen.get() + 1);
        }) as Box<dyn FnMut(web_sys::Event)>);
        document()
            .add_event_listener_with_callback_and_bool(
                "keydown",
                closure.as_ref().unchecked_ref(),
                true,
            )
            .unwrap();
        Self { ran, closure }
    }

    fn remove(self) {
        document()
            .remove_event_listener_with_callback_and_bool(
                "keydown",
                self.closure.as_ref().unchecked_ref(),
                true,
            )
            .unwrap();
    }
}

/// The desktop shell `return`s on a matched chord, so no input target ever sees
/// that key. The web has to say the same thing, and `stopPropagation()` from a
/// `document` capture listener does not: it aborts the walk to the *next* node,
/// leaving every other listener on `document` itself to run. `editor_input`'s
/// keymap is one of those, so an app with a menu bar and a focused editor would
/// act twice on a chord both declare — `Ctrl+Z` is the menus guide's own
/// example.
#[wasm_bindgen_test]
fn a_chord_the_menu_consumed_reaches_no_other_document_listener() {
    let fixture = Fixture::mount("Ctrl+Alt+J");
    let bystander = Bystander::install();

    // Control first: a chord nothing claimed belongs to the page, and every
    // listener on it must still run.
    let prevented = press("KeyY");
    assert!(!prevented, "control: Ctrl+Alt+Y is claimed by nobody");
    assert_eq!(
        bystander.ran.get(),
        1,
        "control: an unclaimed key still reaches the other document listeners"
    );

    let prevented = press("KeyJ");
    assert!(prevented, "the menu claimed Ctrl+Alt+J");
    assert_eq!(fixture.fired.get(), 1, "and ran its item");
    assert_eq!(
        bystander.ran.get(),
        1,
        "a consumed chord must not reach another document listener"
    );

    bystander.remove();
    fixture.teardown();
}
