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
//! `keydown` on the document, mirroring the desktop event loop, which checks
//! every chord *before* the app sees the key and consumes the keystroke only
//! when a callback actually ran.

use std::cell::Cell;

use rinch::menu::Menu;
use rinch_core::dom::{NodeHandle, RenderScope};

use crate::editor_input::add_capture;

thread_local! {
    /// The document listener is page-global and installed exactly once, however
    /// many roots mount with a menu bar.
    static SHORTCUTS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Arm `menus`' chords, render the bar, and wrap `content` in it.
///
/// The order matters only in that both happen: the bar invokes an entry's
/// callback straight from the [`Menu`], and the registry
/// (`register_menu_shortcuts`) is what a keystroke goes through. A rebuild
/// releases the previous build's chords rather than accumulating them.
pub(crate) fn wrap(
    scope: &mut RenderScope,
    menus: &[(&str, Menu)],
    content: NodeHandle,
) -> NodeHandle {
    let refs: Vec<(&str, &Menu)> = menus.iter().map(|(label, menu)| (*label, menu)).collect();
    rinch::menu::register_menu_shortcuts(&refs);
    install_shortcut_dispatch();
    rinch::menu::render_with_menu_bar(scope, &refs, content, 0)
}

/// Install the document `keydown` listener that fires menu shortcuts.
///
/// Capture phase, on `document`, so it runs ahead of every app listener — the
/// position the desktop shell's chord check occupies in `rinch_runtime`, which
/// tests a keystroke against the menus before translating it into a
/// `PlatformEvent::KeyDown`. A browser adds one requirement the desktop does not
/// have: `Ctrl+N`, `Ctrl+K` and friends are the *browser's* shortcuts too, so a
/// chord the app claimed must call `preventDefault` or the app's "New Store"
/// also opens a new browser window.
///
/// Both of those happen **only** when a callback actually ran.
/// `match_shortcut_code` answers `false` for a chord nothing is listening to —
/// an item with no `on_click`, a disabled one, one whose component has since
/// unmounted — and a keystroke nobody claimed belongs to the page.
fn install_shortcut_dispatch() {
    if SHORTCUTS_INSTALLED.with(|f| f.replace(true)) {
        return;
    }
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    add_capture(&doc, "keydown", |event: web_sys::KeyboardEvent| {
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
            // this is the browser's way of saying the same thing.
            event.stop_propagation();
        }
    });
}
