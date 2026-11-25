//! The DOM menu bar, mounted above a web app.
//!
//! The bar itself is [`rinch::menu::render_with_menu_bar`] — the *same*
//! renderer the Linux desktop uses, because both backends build through a
//! `RenderScope` and the browser already dispatches the `data-rid` and
//! `data-onenter` attributes it emits (see `event_delegation`). So clicking an
//! entry, hovering across to the next menu, and clicking outside to dismiss all
//! work here with no web-specific code at all.
//!
//! Keyboard shortcuts are the one thing that does not: a chord arrives as a bare
//! keystroke with no menu under the pointer, so it has to be matched against the
//! menus the app declared. That is what this module adds — a capture-phase
//! `keydown` on the **window**, mirroring the desktop event loop, which checks
//! every chord *before* the app sees the key and consumes the keystroke only
//! when a callback actually ran.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use rinch::menu::Menu;
use rinch_core::dom::{NodeHandle, RenderScope};

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::{Closure, JsValue};

use crate::editor_input::add_capture_on;

thread_local! {
    /// The window listener is page-global and installed exactly once, however
    /// many roots mount with a menu bar.
    static SHORTCUTS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Arm `menus`' chords, render the bar, and wrap `content` in it.
///
/// The order matters only in that both happen: the bar invokes an entry's
/// callback straight from the [`Menu`], and the registry
/// (`register_menu_shortcuts`) is what a keystroke goes through. A rebuild
/// releases the previous build's chords rather than accumulating them.
///
/// The chords come back down when this root unmounts. They are page-global — an
/// island mounted into somebody else's page arms them against the whole
/// document — so leaving them armed after the island is gone would take a key
/// combination away from that page for the rest of the session, which is the
/// harm `set_suppress_native_context_menu`'s doc argues an island must never
/// inflict. `scope.on_cleanup` is where the bar's Escape handle is already
/// released two lines into `build_overlay`; the chords now travel with it.
pub(crate) fn wrap(
    scope: &mut RenderScope,
    menus: &[(&str, Menu)],
    content: NodeHandle,
) -> NodeHandle {
    let refs: Vec<(&str, &Menu)> = menus.iter().map(|(label, menu)| (*label, menu)).collect();
    let chords = rinch::menu::register_menu_shortcuts(&refs);
    // Held until disposal, or the registration above releases itself on the spot
    // (see `MenuBarChords`). Releasing is "only if the slot is still mine", so a
    // *later* bar's chords survive this root's unmount.
    scope.on_cleanup(move || drop(chords));
    install_shortcut_dispatch();
    rinch::menu::render_with_menu_bar(scope, &refs, content, 0)
}

/// What kind of control the key event's own target is, for the chord yield
/// question (`rinch::menu::chord_yields_to`, #1169, #1290):
///
/// - `Text`: a `<textarea>`, an `<input>` whose type takes text, or anything
///   `contenteditable` — the rich-text editor's capture `<textarea>` is the
///   first of those, read-only or not, as it still owns the keys it would type;
/// - `Select`: a `<select>`;
/// - `AllKeys`: a `<canvas>` (a web `RenderSurface`);
/// - `Activator`: any other focused element (a button, a link, a checkbox, a
///   `tabindex` control);
/// - `None`: the page itself (`<body>`, `<html>`).
///
/// The target is `composedPath()[0]`, the element a keystroke is dispatched at
/// (the focused one), not `document.activeElement`: for a field inside a shadow
/// root `activeElement` is the shadow **host**, and the chord ate the `/` typed
/// into a third-party web component's input (review of #1285, measured in
/// Chrome 153). `activeElement` is the fallback for an event with no path.
fn key_target_owner(event: &web_sys::KeyboardEvent) -> rinch::menu::KeyOwner {
    use rinch::menu::KeyOwner;
    let target = event
        .composed_path()
        .get(0)
        .dyn_into::<web_sys::Element>()
        .ok()
        .or_else(|| {
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
        });
    let Some(target) = target else {
        return KeyOwner::None;
    };
    if let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>() {
        return if rinch::menu::input_type_takes_text(&input.type_()) {
            KeyOwner::Text
        } else {
            KeyOwner::Activator
        };
    }
    if target
        .dyn_ref::<web_sys::HtmlElement>()
        .is_some_and(|el| el.is_content_editable())
    {
        return KeyOwner::Text;
    }
    match target.tag_name().to_ascii_lowercase().as_str() {
        "textarea" => KeyOwner::Text,
        "select" => KeyOwner::Select,
        "canvas" => KeyOwner::AllKeys,
        "body" | "html" => KeyOwner::None,
        _ => KeyOwner::Activator,
    }
}

/// Whether AltGr typed `event.key` (#1291) — desktop's rule,
/// [`rinch::platform::is_alt_graph_text`], asked with the browser's facts.
/// The browser reports AltGr on Windows as Ctrl+Alt and says which it was
/// through `getModifierState("AltGraph")`. The press is text when Ctrl and Alt
/// are held (Meta is not), its `key` is one printable character rather than a
/// key name, and that character is not simply the one the key types with no
/// modifier ([`unmodified_key`]). A German or Polish layout reports right-Alt+1
/// as AltGraph with `key` `"1"`: it types nothing, so an app's `Mod-Alt-1`
/// still runs, as on desktop.
pub(crate) fn web_alt_graph_text(event: &web_sys::KeyboardEvent) -> bool {
    let key = event.key();
    if !is_one_printable_char(&key) {
        return false;
    }
    let unmodified = unmodified_key(&event.code());
    rinch::platform::is_alt_graph_text(
        event.get_modifier_state("AltGraph"),
        rinch::platform::Modifiers {
            shift: event.shift_key(),
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        },
        Some(&key),
        unmodified.as_deref(),
    )
}

fn is_one_printable_char(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| !c.is_control() && chars.next().is_none())
}

thread_local! {
    /// What each physical key (`code`) types with no modifier on the user's
    /// layout, as far as this page has seen: winit's `key_without_modifiers`,
    /// which a browser does not report. Filled by [`note_unmodified_key`] from
    /// every unmodified press (which follows a layout switch), and seeded once
    /// from the Keyboard API's layout map where the browser has one (Chromium);
    /// a seeded entry never replaces one a press taught.
    static UNMODIFIED_KEYS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static LAYOUT_MAP_ASKED: Cell<bool> = const { Cell::new(false) };
}

/// Learn what `event`'s physical key types with no modifier, when the press
/// says: no Shift, Ctrl, Alt, Meta or AltGraph held, and a `key` that is one
/// printable character (not `"Dead"`, `"Process"` or a key name). Every
/// keydown listener that asks [`web_alt_graph_text`] calls this first.
pub(crate) fn note_unmodified_key(event: &web_sys::KeyboardEvent) {
    ask_layout_map_once();
    if event.shift_key()
        || event.ctrl_key()
        || event.alt_key()
        || event.meta_key()
        || event.get_modifier_state("AltGraph")
    {
        return;
    }
    let (code, key) = (event.code(), event.key());
    if code.is_empty() || !is_one_printable_char(&key) {
        return;
    }
    UNMODIFIED_KEYS.with(|m| m.borrow_mut().insert(code, key));
}

/// What the key `code` types with no modifier: learned from a press or the
/// layout map, else the US position of a letter or digit key (`KeyQ` → `q`,
/// `Digit1` → `1`: right on most layouts for the digits and most letters),
/// else `None`, and the AltGr text is then taken at its word, as desktop does
/// when it does not know.
fn unmodified_key(code: &str) -> Option<String> {
    if let Some(known) = UNMODIFIED_KEYS.with(|m| m.borrow().get(code).cloned()) {
        return Some(known);
    }
    let position = code
        .strip_prefix("Key")
        .or_else(|| code.strip_prefix("Digit"))?;
    let mut chars = position.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphanumeric())
        .then(|| c.to_ascii_lowercase().to_string())
}

/// Seed [`UNMODIFIED_KEYS`] from `navigator.keyboard.getLayoutMap()` once per
/// page, where the browser has it (Chromium; Firefox and Safari do not), so
/// the first AltGr press on a layout whose keys differ from US (French
/// `Digit1` types `&`) is judged right before any plain press taught it.
fn ask_layout_map_once() {
    if LAYOUT_MAP_ASKED.with(|a| a.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let get = |target: &JsValue, name: &str| js_sys::Reflect::get(target, &name.into()).ok();
    let Some(keyboard) = get(&window.into(), "navigator")
        .and_then(|navigator| get(&navigator, "keyboard"))
        .filter(|k| k.is_object())
    else {
        return;
    };
    let Some(get_layout_map) =
        get(&keyboard, "getLayoutMap").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    let Ok(promise) = get_layout_map
        .call0(&keyboard)
        .and_then(|p| p.dyn_into::<js_sys::Promise>())
    else {
        return;
    };
    let on_map = Closure::once(move |map: JsValue| {
        let Some(for_each) =
            get(&map, "forEach").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        else {
            return;
        };
        let seed = Closure::<dyn FnMut(JsValue, JsValue)>::new(|key: JsValue, code: JsValue| {
            if let (Some(key), Some(code)) = (key.as_string(), code.as_string())
                && is_one_printable_char(&key)
            {
                UNMODIFIED_KEYS.with(|m| {
                    m.borrow_mut().entry(code).or_insert(key);
                });
            }
        });
        let _ = for_each.call1(&map, seed.as_ref());
    });
    // A refusal (a cross-origin frame, a denied permission) leaves the
    // fallbacks; handling it keeps it out of the console.
    let on_refused = Closure::once(|_: JsValue| {});
    let _ = promise.then2(&on_map, &on_refused);
    on_map.forget();
    on_refused.forget();
}

/// The single ASCII letter `key` (a `KeyboardEvent.key`) spells, lowercased —
/// or `None` for anything else: a non-letter key, a multi-character name
/// (`"Enter"`, `"ArrowLeft"`), or a non-Latin character (Cyrillic, Thai, …).
/// Feeds `match_shortcut_code`'s layout-character branch (#1170); `None` is
/// exactly "this layout types no Latin letter for the pressed key", which is
/// what falls back to physical-key matching there.
fn web_typed_letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase())
}

/// Install the `keydown` listener that fires menu shortcuts.
///
/// **Capture phase, on `window`** — not on `document`, and the difference is the
/// whole point. The desktop shell's chord check `return`s before the keystroke
/// becomes a `PlatformEvent::KeyDown`, so *no* input target ever sees a chord
/// the menus claimed. `stopPropagation()` from a `document` capture listener
/// cannot say that: per the DOM dispatch algorithm it aborts the walk to the
/// **next** node in the path, and leaves every other listener on `document`
/// itself to run. `editor_input`'s keymap is one of those, so a focused editor
/// acted on `Ctrl+Z` a second time after the menu item had already run it — the
/// menus guide's own example.
///
/// `window` is one node further out than `document`, so stopping there stops
/// everything on `document`, whatever order the listeners were registered in.
/// Registration order is not something this module can rely on: `wrap` runs
/// inside the build closure, ahead of `ensure_event_delegation` and
/// `editor_input::install` on a page whose *first* mount carries a menu bar, and
/// behind both on a page where a plain `mount()` came first.
///
/// One observer must go on seeing a consumed chord, and it is on `window` for
/// that reason: the pointer-gesture flag (`event_delegation`), whose whole job
/// is to watch the raw input stream. Same-node listeners all run — stopping
/// propagation is not `stopImmediatePropagation` — so it is unaffected.
///
/// A browser also adds a requirement the desktop does not have: a chord the app
/// claimed must call `preventDefault`, or the browser's own handling of that
/// combination runs alongside the app's. (A handful of chords are the browser's
/// alone — see the guide — and `preventDefault` does not reach those.)
///
/// Both of those happen **only** when a callback actually ran.
/// `match_shortcut_code` answers `false` for a chord nothing is listening to —
/// an item with no `on_click`, a disabled one, one whose component has since
/// unmounted — and a keystroke nobody claimed belongs to the page.
fn install_shortcut_dispatch() {
    if SHORTCUTS_INSTALLED.with(|f| f.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    add_capture_on(
        window.as_ref(),
        "keydown",
        |event: web_sys::KeyboardEvent| {
            // What an unmodified press types, for the AltGr question below.
            note_unmodified_key(&event);
            // `code` is the physical key's W3C name — "KeyK", "Digit0", "F5" — which
            // is exactly what a shortcut string parses into, so no translation.
            // It is still the only thing a *digit or punctuation* chord
            // matches against (unaffected by #1170 below): `key` for those
            // carries the shifted character, so Ctrl+Shift+K reports "K" on
            // every layout and tells punctuation nothing about which key was
            // pressed.
            let code = event.code();
            if code.is_empty() {
                return;
            }
            // A keystroke the focused control acts on is the control's
            // (#1169, #1290): matching it would `preventDefault` the very `/`
            // the user is typing, or the ArrowDown a `<select>` moves by. So
            // is AltGr text typed into a field, which Windows reports as
            // Ctrl+Alt (#1291). The desktop shell asks the same question of
            // its focus arbiter.
            if rinch::menu::chord_yields_to(
                key_target_owner(&event),
                event.ctrl_key(),
                event.meta_key(),
                event.alt_key(),
                &code,
                web_alt_graph_text(&event),
            ) {
                return;
            }
            // `key` *is* what a letter chord matches against (issue #1170):
            // lowercased, it is the layout's character for the pressed key —
            // "z" from the key labelled Z wherever the layout put it — so
            // `rinch::menu::match_shortcut_code` tries it first and falls back
            // to `code` only when it names no single Latin letter (a digit, a
            // punctuation mark, or a non-Latin character on a Cyrillic/Thai/…
            // layout). Shift does not change it: the browser's `key` for a
            // letter under Ctrl is already unshifted on every tested layout,
            // and lowercasing covers Caps Lock.
            let typed_letter = web_typed_letter(&event.key());
            if rinch::menu::match_shortcut_code(
                event.ctrl_key(),
                event.meta_key(),
                event.alt_key(),
                event.shift_key(),
                &code,
                typed_letter,
            ) {
                event.prevent_default();
                // The app must not *also* act on a key the menu consumed — the
                // desktop shell returns instead of forwarding a matched chord, and
                // this is the browser's way of saying the same thing. From
                // `window` this reaches `editor_input`'s keymap and the bubble
                // delegate, which sit on `document`, one node further along the
                // path. The one listener it does **not** reach is the
                // pointer-gesture observer, which is on `window` beside this one
                // for exactly that reason (see above) — measured in Chrome 153: a
                // consumed chord still clears the flag.
                event.stop_propagation();
            }
        },
    );
}
