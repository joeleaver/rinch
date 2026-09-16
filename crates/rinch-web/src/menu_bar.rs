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

use std::cell::Cell;

use rinch::menu::Menu;
use rinch_core::dom::{NodeHandle, RenderScope};

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
            // `code` is the physical key's W3C name — "KeyK", "Digit0", "F5" — which
            // is exactly what a shortcut string parses into, so no translation.
            // `key` would be wrong here: it carries the *typed character*, so
            // Ctrl+Shift+K reports "K" on one layout and something else on another.
            let code = event.code();
            if code.is_empty() {
                return;
            }
            if rinch::menu::match_shortcut_code(
                event.ctrl_key(),
                event.meta_key(),
                event.alt_key(),
                event.shift_key(),
                &code,
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
