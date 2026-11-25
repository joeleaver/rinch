//! Key delegation: the order rinch-web offers a key to its handlers (issue
//! #3062; the table is `docs/internals/focus-and-keyboard.md` "The key handler
//! order").
//!
//! The rule is Chrome's: everything rinch does with a key is a *default
//! action*, so the app hears every key first, and a key the page has
//! `preventDefault`ed is left alone. Three listeners carry it:
//!
//! 1. **`window`, capture phase** ([`ensure_window_key_capture`]): the menu
//!    bar's chords, then Escape ending an element drag, then the app's
//!    keyboard interceptor (`set_keyboard_interceptor`). `window` is one node
//!    further out than `document` on every event path, so these run ahead of
//!    every `document` listener (the editor's and the bubble delegate's) and
//!    ahead of any element listener the page registered.
//! 2. **`document`, capture phase** (`editor_input`): the rich-text editor's
//!    `on_key`, then the popups' key entries and the dismiss stack, then the
//!    app-wide listeners ([`offer_popup_and_app_keys`]), then its keymap.
//! 3. **`document`, bubble phase** ([`install_key_delegation`]): the focused
//!    element's `onkeydown` and its ancestors', the key entries and the
//!    dismiss stack, a focused render surface, the app-wide listeners, then
//!    the Tab trap, keyboard activation and `onsubmit`.
//!
//! Each backs off a press whose default is already prevented: the page's own
//! listener (or rinch's, earlier on the path) has taken it.

use std::cell::Cell;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_core::events;

use super::{
    COMMITTED_VALUE_PROP, FOCUS_VALUE_PROP, cancel_web_drag, dispatch_form_value_attr,
    editor_owning, form_control_value, get_expando_string, handle_trapped_tab, set_expando,
    try_keyboard_activation,
};
use crate::editor_input::add_capture_on;

thread_local! {
    /// The `window` key listeners are page-global, installed exactly once
    /// however many roots mount.
    static WINDOW_KEYS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// The expando a press carries once [`offer_popup_and_app_keys`] has offered
/// it, so the bubble delegate does not offer it again.
const POPUP_AND_APP_KEYS_OFFERED: &str = "__rinchPopupAndAppKeysOffered";

/// `event` as rinch's key data, of `kind`.
fn key_data(event: &web_sys::KeyboardEvent, kind: events::KeyEventKind) -> events::KeyEventData {
    events::KeyEventData::new(event.key(), event.code())
        .with_modifiers(
            event.ctrl_key() || event.meta_key(),
            event.shift_key(),
            event.alt_key(),
            event.meta_key(),
        )
        .with_kind(kind)
}

/// Install the `window` capture-phase `keydown` and `keyup` listeners, once
/// per page: from `setup_event_delegation` and from a root's menu bar
/// (`menu_bar::wrap`), whichever comes first.
///
/// A press, unless its default is already prevented, goes to: the menu bar's
/// chord (`menu_bar::dispatch_menu_chord`), `preventDefault`ed and stopped;
/// then Escape cancelling an element drag, `preventDefault`ed only, so every
/// later rinch listener backs off it; then the keyboard interceptor, which
/// hears the key ahead of every element `onkeydown`, the rich-text editor and
/// a focused render surface. Before #3062 the interceptor ran from the bubble
/// delegate, after the element's `onkeydown` and the editor's keymap. A key it
/// consumes is `preventDefault`ed and stopped.
///
/// A release goes to the interceptor whatever its default: a release has no
/// default for rinch to yield, and a held-key tracker must see every one.
///
/// Two `window` capture listeners run in the order they were added. A page's
/// own one registered **before** this one (before the first root mounted or
/// built its menu bar) runs first and can `preventDefault` a press away from
/// rinch (not a release, which the interceptor hears regardless). One
/// registered after it runs after this whole listener — menu chord, drag
/// Escape and interceptor — and runs even for a press rinch consumed:
/// `stopPropagation` keeps the event from other nodes, not from the later
/// listeners on `window` itself, so that listener sees `defaultPrevented` set.
pub(crate) fn ensure_window_key_capture() {
    if WINDOW_KEYS_INSTALLED.with(|f| f.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let window: web_sys::EventTarget = window.into();
    add_capture_on(&window, "keydown", |event: web_sys::KeyboardEvent| {
        if event.default_prevented() {
            return;
        }
        if crate::menu_bar::dispatch_menu_chord(&event) {
            event.prevent_default();
            event.stop_propagation();
            return;
        }
        // Escape cancels an in-progress element drag (consumed only if one was
        // actually active, so Escape otherwise reaches the app normally).
        if event.key() == "Escape" && cancel_web_drag() {
            event.prevent_default();
            return;
        }
        // The key is the root's whose element it targets (#2811): the
        // interceptor it reaches is that root's.
        let _root = crate::mark_event_root(&event);
        if events::dispatch_key_interceptor(&key_data(&event, events::KeyEventKind::Down)) {
            event.prevent_default();
            event.stop_propagation();
        }
    });
    add_capture_on(&window, "keyup", |event: web_sys::KeyboardEvent| {
        let _root = crate::mark_event_root(&event);
        events::dispatch_key_interceptor(&key_data(&event, events::KeyEventKind::Up));
    });
}

/// Offer a press to the open popups' key entries and, for Escape, the dismiss
/// stack (`events::dispatch_popup_key`), then to the app-wide key listeners
/// (`on_app_key`) unless an input method is composing with it; answer whether
/// one consumed it. The rich-text editor's keydown calls it after its
/// `on_key` and before its keymap (#3062), as on desktop; the bubble delegate
/// calls it for every other key. A press is offered once: the call marks the
/// event, and the bubble delegate skips a marked one.
pub(crate) fn offer_popup_and_app_keys(event: &web_sys::KeyboardEvent) -> bool {
    js_sys::Reflect::set(
        event.as_ref(),
        &JsValue::from_str(POPUP_AND_APP_KEYS_OFFERED),
        &JsValue::TRUE,
    )
    .ok();
    let data = key_data(event, events::KeyEventKind::Down);
    if events::dispatch_popup_key(&data) {
        return true;
    }
    if event.is_composing() || event.key() == "Process" {
        return false;
    }
    // They hear the page's keys, every root's (see `on_app_key`), so they are
    // offered the key as no root's (#2811).
    let _page = rinch_core::push_dispatching_doc(0);
    events::dispatch_app_key(&data)
}

/// Whether [`offer_popup_and_app_keys`] has already been asked about `event`.
fn popup_and_app_keys_offered(event: &web_sys::KeyboardEvent) -> bool {
    js_sys::Reflect::get(
        event.as_ref(),
        &JsValue::from_str(POPUP_AND_APP_KEYS_OFFERED),
    )
    .is_ok_and(|v| v.is_truthy())
}

/// Install the bubble-phase `keydown` and `keyup` delegates on `browser_doc`
/// (step 3 of the module docs).
pub(super) fn install_key_delegation(browser_doc: &web_sys::Document) {
    let browser_doc_for_tab = browser_doc.clone();
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        // Anything at all can be dispatched at `document` under the name
        // "keydown" — `document.dispatchEvent(new Event("keydown"))` from page
        // script, a polyfill, an extension, a test harness. wasm-bindgen hands a
        // `Closure<dyn FnMut(web_sys::KeyboardEvent)>` whatever arrives, cast
        // **unchecked**, so on a plain `Event` the very first `event.key()`
        // returned `undefined` and the string marshaller threw `Cannot read
        // properties of undefined (reading 'length')` out of a document
        // listener — a console full of TypeErrors from a page that had done
        // nothing wrong.
        //
        // `dyn_ref` is the instanceof check that was missing. `add_capture` does
        // the same for the listeners registered through it; this one is
        // registered by hand, as is every other listener in
        // `setup_event_delegation`, and each takes a plain `Event` (issue
        // #811).
        //
        // Because it is `instanceof`, a `KeyboardEvent` built from another
        // realm's constructor (an iframe's `KeyboardEvent`, dispatched on this
        // document by script) is ignored, exactly as `add_capture` ignores it.
        // A trusted key press inside an iframe goes to the iframe's own
        // document.
        let Some(event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        // A press the page (or rinch's own `window` listener: a cancelled
        // drag) has `preventDefault`ed is taken: rinch does nothing with it
        // (#3062), as Chrome runs no default action for it.
        if event.default_prevented() {
            return;
        }

        // The key is the root's whose element it targets (#2811): its
        // handlers run as that root's code, and the key entries and dismiss
        // stack it reaches are that root's.
        let _root = crate::mark_event_root(event);
        let key_data = key_data(event, events::KeyEventKind::Down);

        // 1. The focused element's own `onkeydown`, then its ancestors' (issue
        //    #1678), after the interceptor (`window`) and before the key
        //    entries and the dismiss stack — desktop's order. A consumed key is
        //    the browser's `preventDefault` + `stopPropagation`: no line break,
        //    no submit, no activation, no Tab, and nothing after this sees it.
        if offer_element_key(event, "data-onkeydown", &key_data) {
            event.prevent_default();
            event.stop_propagation();
            return;
        }

        // 2. The open popups' key entries, then for Escape the dismiss stack.
        //    A key the rich-text editor's own listener already offered them
        //    (with the app-wide listeners) is not offered twice.
        let offered = popup_and_app_keys_offered(event);
        if !offered && events::dispatch_popup_key(&key_data) {
            event.prevent_default();
            event.stop_propagation();
            return;
        }

        // 3. If a render surface is focused, forward KeyDown + TextInput for
        //    input (always — unaffected by whether the key is "claimed"
        //    below) and ask the surface's own `set_key_handler` whether this
        //    key stops here.
        //
        //    Issue #482: this used to call `preventDefault`/
        //    `stopPropagation` unconditionally, so the moment a canvas took
        //    focus every key — including ones the surface did nothing with —
        //    went deaf to the host's own `window`-level keybindings and the
        //    browser's own shortcuts (reload, find, …). The default, with no
        //    `set_key_handler` registered, is "claims nothing": an unclaimed
        //    key now falls through to the Tab-trap/activation/submit handling
        //    below and keeps propagating to `window` and the browser.
        if let Some(surface_id) = rinch::render_surface::focused_surface_id() {
            let surface_key_data = surface_key_data(event);
            rinch::render_surface::dispatch_surface_event(
                surface_id,
                rinch::render_surface::SurfaceEvent::KeyDown(surface_key_data.clone()),
            );
            // Also dispatch TextInput for printable characters
            let key = event.key();
            if key.len() == 1 && !event.ctrl_key() && !event.meta_key() && !event.alt_key() {
                rinch::render_surface::dispatch_surface_event(
                    surface_id,
                    rinch::render_surface::SurfaceEvent::TextInput(key),
                );
            }
            if rinch::render_surface::dispatch_surface_key_event(surface_id, &surface_key_data) {
                // A claimed key is the surface's, release and all (#1784).
                events::withhold_app_key_release(&surface_key_data.code);
                event.prevent_default();
                event.stop_propagation();
                return;
            }
        }

        // 4. The app-wide key listeners (`on_app_key`, issue #1784): after
        //    everything that owns a key, before the Tab trap, activation and
        //    `onsubmit` below — desktop's order. This listener is on
        //    `document`, so it hears a key with nothing focused (it targets
        //    `<body>` and bubbles here). Not a key the rich-text editor holds
        //    (its listener offered them every key it did not take first, from
        //    between its `on_key` and its keymap), nor one an input method is
        //    composing with. They hear the page's keys, every root's (see
        //    `on_app_key`), so they are offered the key as no root's (#2811).
        if !offered
            && !event.is_composing()
            && event.key() != "Process"
            && !key_target_is_editor(event)
            && {
                let _page = rinch_core::push_dispatching_doc(0);
                events::dispatch_app_key(&key_data)
            }
        {
            event.prevent_default();
            event.stop_propagation();
            return;
        }

        if key_data.key == "Tab" && handle_trapped_tab(&browser_doc_for_tab, event.shift_key()) {
            // `trap_focus` (#474): an open overlay confines Tab to itself. The
            // browser would otherwise walk straight out of the dialog and into
            // the page behind it, so the move is made here and its own default
            // suppressed. With no live trap this is false and the browser's Tab
            // is left alone here. The one other place rinch moves web focus on
            // Tab is a read-only rich-text editor (#833, `tab_from_editor`),
            // which the editor's own keydown handles from its place in the page
            // before this listener sees the key.
            //
            // After the interceptor and the app's listeners, so they still see
            // Tab first, and before the activation branch, which only ever
            // answers Enter/Space.
            event.prevent_default();
        } else if try_keyboard_activation(event, &key_data.key) {
            // Enter/Space on a focused element the browser does not activate
            // itself (issue #240 — the `Tree` node shape). Consume the key:
            // this is what stops Space from also scrolling the page, on the
            // activating press and on every auto-repeat of it. A node with no
            // live handler fell through, leaving the key to the browser.
            event.prevent_default();
        } else if key_data.key == "Enter" && !event.shift_key() && !event.is_composing() {
            submit_on_enter(event);
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("keydown", keydown_closure.as_ref().unchecked_ref())
        .unwrap();
    keydown_closure.forget();

    // keyup: the element's `onkeyup`, the app-wide listeners, then a focused
    // render surface (games need key-release). The interceptor heard the
    // release first, from `window` ([`ensure_window_key_capture`]).
    //
    // The interceptor's release is issue #337's web leg. Releases reached no
    // app code on *either* backend — fixing only desktop would have turned a
    // shared gap into a divergence, which is the thing an app most notices,
    // since the whole point of an interceptor is that it is the one hook that
    // behaves the same everywhere.
    //
    // Releases do not back off a prevented default as presses do: a release
    // has no default action for rinch to yield, and a handler tracking held
    // keys must see every release its press reached.
    let keyup_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        // See the keydown listener: the cast is unchecked, so a plain
        // `Event("keyup")` would read `undefined` for `key` and throw.
        let Some(event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        let key_data = key_data(event, events::KeyEventKind::Up);
        // The element's own `onkeyup` (issue #1678); a consuming one stops
        // its ancestors only. As the target's root's code, like the press
        // (#2811).
        {
            let _root = crate::mark_event_root(event);
            offer_element_key(event, "data-onkeyup", &key_data);
        }
        // Every release reaches the app-wide listeners, but one whose press a
        // focused surface claimed (#1784). As no root's: they hear the page.
        // Asked first, as the dispatch clears the mark.
        let claimed = events::app_key_release_withheld(&key_data.code);
        events::dispatch_app_key(&key_data);

        if let Some(surface_id) = rinch::render_surface::focused_surface_id() {
            rinch::render_surface::dispatch_surface_event(
                surface_id,
                rinch::render_surface::SurfaceEvent::KeyUp(surface_key_data(event)),
            );
            // Only the release of a press the surface claimed is the
            // surface's (#3062, W1): the rest bubble on to the page's own
            // `window` listeners, as a `<canvas>`'s keyups do. This used to
            // stop every release while a surface held the keyboard, so the
            // page's keyup listeners went deaf; the desktop withholds only a
            // claimed key's release from the app's listeners.
            if claimed {
                event.prevent_default();
                event.stop_propagation();
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("keyup", keyup_closure.as_ref().unchecked_ref())
        .unwrap();
    keyup_closure.forget();
}

/// `event` as a render surface's key data.
fn surface_key_data(event: &web_sys::KeyboardEvent) -> rinch::render_surface::SurfaceKeyData {
    rinch::render_surface::SurfaceKeyData {
        key: event.key(),
        code: event.code(),
        ctrl: event.ctrl_key() || event.meta_key(),
        shift: event.shift_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

/// Enter (without Shift) fires the nearest ancestor's data-onsubmit, matching
/// a native form submit. Shift+Enter is left alone so multiline inputs (a
/// textarea) can insert a newline. During an IME composition the Enter that
/// commits the candidate is skipped (isComposing, by the caller) so it
/// confirms the text instead of submitting.
fn submit_on_enter(event: &web_sys::KeyboardEvent) {
    let Some(target) = event.target() else {
        return;
    };
    let Ok(el) = target.clone().dyn_into::<web_sys::Element>() else {
        return;
    };
    let mut current: Option<web_sys::Element> = Some(el.clone());
    while let Some(cur) = current {
        if let Some(handler_str) = cur.get_attribute("data-onsubmit")
            && let Ok(handler_id) = handler_str.parse::<usize>()
        {
            // Prevent the browser from also inserting a newline
            // (in a textarea) or running a native form submit.
            event.prevent_default();
            // preventDefault also suppresses the browser's native
            // change-on-Enter commit, so deliver the
            // `data-onchange` commit ourselves — before onsubmit,
            // matching desktop's change-before-submit ordering —
            // and only if the value was modified since focus
            // (mirroring the browser's only-if-modified rule).
            // Record the committed value on the control so the
            // `change` listener can skip the browser's deferred
            // duplicate at the eventual blur (issue #226).
            if let Some(value) = form_control_value(&target) {
                let focus_value = get_expando_string(&el, FOCUS_VALUE_PROP);
                if focus_value.as_deref() != Some(value.as_str())
                    && dispatch_form_value_attr(&target, "data-onchange", value.clone())
                {
                    set_expando(&el, COMMITTED_VALUE_PROP, &JsValue::from_str(&value));
                }
            }
            events::dispatch_event(events::EventHandlerId(handler_id));
            break;
        }
        current = cur.parent_element();
    }
}

/// Offer a key to the `attr` (`data-onkeydown` / `data-onkeyup`) handler of
/// the element the key was pressed in and then its ancestors', nearest first,
/// stopping at the first that consumes it (issue #1678). The ids are read
/// before any handler runs, as a browser fixes an event's path at dispatch.
///
/// Not offered — as on desktop, where these have no element claim to walk
/// from — for a key the rich-text editor holds (its keys are
/// `EditorHandle::on_key`'s; a field inside a node view's component is not
/// the editor's), one a focused render surface holds (`set_key_handler`), or
/// one an input method is composing with (`isComposing`, or the `Process` key
/// a composition sends). Known gap: Safari sends the Enter that confirms a
/// composition after `compositionend`, as `key: "Enter"`, `keyCode` 229 and
/// `isComposing` false, and that Enter is offered. Refusing every 229 Enter
/// would also refuse an Android soft keyboard's plain Enter, which can carry
/// 229.
fn offer_element_key(
    event: &web_sys::KeyboardEvent,
    attr: &str,
    data: &events::KeyEventData,
) -> bool {
    if event.is_composing() || event.key() == "Process" {
        return false;
    }
    if rinch::render_surface::focused_surface_id().is_some() {
        return false;
    }
    let Some(el) = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    if el.has_attribute("data-pm-capture") || editor_owning(&el).is_some() {
        return false;
    }
    let data = &data.clone().with_target(crate::web_document::get_nid(&el));
    let mut ids = Vec::new();
    let mut cur = Some(el);
    while let Some(e) = cur {
        if let Some(id) = e.get_attribute(attr).and_then(|v| v.parse::<usize>().ok()) {
            ids.push(events::EventHandlerId(id));
        }
        cur = e.parent_element();
    }
    !ids.is_empty() && events::dispatch_key_handler_chain(ids, data)
}

/// Whether a key event's target is a rich-text editor's: its capture
/// textarea, or an element inside an editor that is not a node view's
/// component (see [`editor_owning`]).
fn key_target_is_editor(event: &web_sys::KeyboardEvent) -> bool {
    event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| el.has_attribute("data-pm-capture") || editor_owning(&el).is_some())
}
