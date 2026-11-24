//! Keyboard input: the editor's key bindings, node views' keys and
//! `keydown`.

use super::capture::blur_editor;
use super::context_menu::{
    MenuTrigger, end_context_menu_cycle, park_right_of, start_context_menu_cycle,
};
use super::motion::{
    key_menu_app_handler, key_menu_point, move_to_line_edge, page_step, vertical_step,
};
use super::*;

// ── Keyboard ─────────────────────────────────────────────────────────────────

/// Translate a `keydown` (its logical `key` and physical `code`) + modifier state into
/// an editor-core `KeyBinding` for keymap lookup.
///
/// **Letters resolve LOGICALLY** (`event.key()`), so `Mod-b` is correct on every layout
/// — on Dvorak/AZERTY the key that types `b` fires bold, not the physical QWERTY-B
/// position. **Digits, symbols, and named keys resolve PHYSICALLY** (`event.code()`),
/// because the logical key for e.g. `Shift+8` is `"*"`, not `"8"` — the physical
/// `Digit8` is what the keymap's `Mod-Shift-8` binding needs. `None` for keys with no
/// bindable identity (they fall through to text insertion / native clipboard).
///
/// (Desktop keys everything physically off the platform `KeyCode`; unifying that for
/// letters would mean plumbing winit's logical key through `PlatformEvent::KeyDown`.)
fn editor_key_binding(
    key: &str,
    code: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<rinch_editor_core::KeyBinding> {
    use rinch_editor_core::{Key, KeyBinding, Modifiers};
    // A single ASCII letter from the *logical* key (layout-correct).
    let logical_letter = {
        let mut it = key.chars();
        match (it.next(), it.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
            _ => None,
        }
    };
    let k = if let Some(c) = logical_letter {
        Key::Char(c)
    } else if let Some(d) = code.strip_prefix("Digit").filter(|s| s.len() == 1) {
        Key::Char(d.as_bytes()[0] as char)
    } else {
        match code {
            "Enter" | "NumpadEnter" => Key::Enter,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Space" => Key::Space,
            "ArrowLeft" => Key::ArrowLeft,
            "ArrowRight" => Key::ArrowRight,
            "ArrowUp" => Key::ArrowUp,
            "ArrowDown" => Key::ArrowDown,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "Minus" => Key::Char('-'),
            "Equal" => Key::Char('='),
            _ => return None,
        }
    };
    Some(KeyBinding::new(
        k,
        Modifiers {
            primary: ctrl,
            shift,
            alt,
        },
    ))
}

/// Handle a `keydown`. Returns whether the editor consumed the key (so the listener
/// `preventDefault`s + stops it). Mirrors `dispatch_new_editor_key`.
/// A key whose target is inside a node view's component, offered to the view
/// ([`EditorHandle::offer_node_view_key`]): `Some(consumed)` when the target is in
/// one, `None` otherwise.
fn offer_node_view_key(event: &web_sys::KeyboardEvent) -> Option<bool> {
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())?;
    let (editor_el, host) = crate::event_delegation::node_view_around(&target)?;
    let container_nid = get_nid(&editor_el.into())?.0;
    let host_nid = get_nid(&host.into())?.0;
    let handle = registry::editor_for(container_nid)?;
    let key = event.key();
    let offered = rinch_editor_view::EditorKey {
        key: &key,
        primary: if is_mac() {
            event.meta_key()
        } else {
            event.ctrl_key()
        },
        ctrl: event.ctrl_key(),
        meta: event.meta_key(),
        shift: event.shift_key(),
        alt: event.alt_key(),
        repeat: event.repeat(),
    };
    Some(handle.offer_node_view_key(host_nid, &offered))
}

/// The page's node-view `ResizeObserver` and the hosts it watches, each with its
/// editor's document key.
type NodeViewResize = (web_sys::ResizeObserver, Vec<(web_sys::Element, u64)>);

thread_local! {
    /// One `ResizeObserver` per page, watching every mounted node view's host: a
    /// view that grows or shrinks owes its editor an overlay pass, so the caret
    /// and the selection highlight below it follow. Keyed by host element →
    /// document key.
    static NODE_VIEW_RESIZE: RefCell<Option<NodeViewResize>> =
        const { RefCell::new(None) };
}

/// [`registry::set_node_view_host_listener`]'s listener: observe a mounted
/// view's host, forget an unmounted one's.
pub(super) fn on_node_view_host(
    doc_key: u64,
    _container: u64,
    host: &rinch_core::dom::NodeHandle,
    mounted: bool,
) {
    let Some(el) = crate::web_document::element_of(host) else {
        return;
    };
    NODE_VIEW_RESIZE.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            let cb = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(
                |entries: js_sys::Array| {
                    let keys: Vec<u64> = NODE_VIEW_RESIZE.with(|cell| {
                        let cell = cell.borrow();
                        let Some((_, hosts)) = cell.as_ref() else {
                            return Vec::new();
                        };
                        entries
                            .iter()
                            .filter_map(|e| e.dyn_into::<web_sys::ResizeObserverEntry>().ok())
                            .filter_map(|e| {
                                let t = e.target();
                                hosts.iter().find(|(h, _)| *h == t).map(|(_, k)| *k)
                            })
                            .collect()
                    });
                    for key in keys {
                        registry::owe_overlay_pass_for(key);
                    }
                },
            );
            let Ok(observer) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) else {
                return;
            };
            cb.forget();
            *cell = Some((observer, Vec::new()));
        }
        let Some((observer, hosts)) = cell.as_mut() else {
            return;
        };
        if mounted {
            if !hosts.iter().any(|(h, _)| *h == el) {
                observer.observe(&el);
                hosts.push((el, doc_key));
            }
        } else if let Some(i) = hosts.iter().position(|(h, _)| *h == el) {
            observer.unobserve(&el);
            hosts.remove(i);
        }
    });
}

pub(super) fn handle_keydown(event: &web_sys::KeyboardEvent, doc: &web_sys::Document) -> bool {
    // A key reaching the page means the browser's menu is closed (it takes the
    // keyboard while open); the cycle ends before the key is read, so a typed
    // character lands in a mirrored field rather than the sentinel.
    end_context_menu_cycle();
    if event.key() == "ContextMenu" {
        MENU_KEY_TO_APP.with(|c| c.set(false));
    }
    // A key inside a node view's component (an input in an app's sheet drawn in
    // an editor) is the view's `on_key`'s first; an Escape it leaves gives the
    // keyboard back to the editor, caret after the node.
    if let Some(consumed) = offer_node_view_key(event) {
        if consumed {
            refresh_caret();
        }
        return consumed;
    }
    // The editor owns a key only while its capture textarea holds the keyboard
    // (issue #271). This listener is on `document` in the capture phase, so it
    // sees every key on the page: a key whose target is anything else — a search
    // box, a `<button>` or `tabindex` control the user tabbed or clicked to, the
    // page body after a click on blank space — belongs to that element. Routing
    // it to the last editor clicked used to split that editor's paragraph on
    // Enter and swallow the key, so keyboard activation went inert page-wide once
    // any editor had been clicked. (A toolbar that must not take the keyboard
    // from the editor says so with `data-nofocus`, which keeps the capture
    // textarea focused through the press.)
    let on_capture = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|t| t.has_attribute("data-pm-capture"));
    if !on_capture {
        return false;
    }
    // During an IME composition, yield every key to the textarea + IME — the composed
    // text arrives via the composition events, so inserting here would double up.
    if event.is_composing() || COMPOSING.with(|c| c.get()) {
        return false;
    }
    let Some((container_nid, handle)) = focused_handle() else {
        return false;
    };
    let key = event.key();
    // The app's `EditorHandle::on_key` sees the key before anything below acts on
    // it, and may take it: an open autocomplete popup's arrows, Enter and Escape.
    // A consumed key is `preventDefault`ed and stopped like one the editor
    // handles. `Unidentified` / `Process` are a soft keyboard's and an input
    // method's: their input arrives through `beforeinput` and composition.
    if key != "Unidentified" && key != "Process" {
        let offered = rinch_editor_view::EditorKey {
            key: &key,
            primary: if is_mac() {
                event.meta_key()
            } else {
                event.ctrl_key()
            },
            ctrl: event.ctrl_key(),
            meta: event.meta_key(),
            shift: event.shift_key(),
            alt: event.alt_key(),
            repeat: event.repeat(),
        };
        if handle.offer_key(&offered) {
            // The callback may have edited or moved the selection.
            refresh_caret();
            return true;
        }
    }
    // The PHYSICAL key, for keymap lookup — `event.key()` is the *resolved* char ("*" for
    // Shift+8, "¡" for Alt+1) which would never match the keymap's physical-key bindings.
    let code = event.code();
    let ctrl = event.ctrl_key() || event.meta_key();
    let shift = event.shift_key();
    let alt = event.alt_key();

    // The goal column survives a run of Up / Down and Page Up / Down.
    if !matches!(
        key.as_str(),
        "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown"
    ) {
        set_goal_x(None);
    }

    // The keyboard's menu key, and Shift+F10: the browser fires `contextmenu` at the
    // focused element — this textarea — as the key's default action, and places the
    // menu at the element's box. Parked at the caret first, the menu opens at the
    // caret rather than at the textarea's resting corner (issue #814); the key is
    // left to the browser, or no menu would open at all.
    //
    // Unless an app's `data-oncontextmenu` claims the editor, as a right-click there
    // finds: the browser's `contextmenu` would go to this textarea, outside the app's
    // subtree, and never reach the handler. So the handler is dispatched here, with
    // its click context at the caret, and the key is consumed so the browser opens
    // no menu of its own — nor at the release, where Windows opens it (by reading
    // Chromium), which is what `MENU_KEY_TO_APP` withholds.
    if key == "ContextMenu" || (key == "F10" && shift) {
        let point = key_menu_point(&handle, container_nid);
        if let Some((menu_el, id)) = key_menu_app_handler(&handle, container_nid) {
            set_click_context_for(
                &menu_el,
                point,
                Default::default(),
                rinch_core::events::MouseButton::Right,
                modifiers_from_key_event(event),
            );
            if key == "ContextMenu" {
                MENU_KEY_TO_APP.with(|c| c.set(true));
            }
            rinch_core::events::dispatch_event(id);
            return true;
        }
        if let Some((x, y)) = point {
            start_context_menu_cycle(
                &handle,
                container_nid,
                MenuTrigger::Key,
                park_right_of(x, y),
            );
        }
        return false;
    }

    let handled = match key.as_str() {
        // 1. Cursor movement / selection extension — geometry-dependent (browser layout),
        //    so it stays view-owned and never touches the keymap.
        "ArrowUp" => vertical_step(&handle, container_nid, doc, false, shift),
        "ArrowDown" => vertical_step(&handle, container_nid, doc, true, shift),
        "ArrowLeft" => handle.move_cursor(
            if ctrl {
                CursorMotion::WordLeft
            } else {
                CursorMotion::CharLeft
            },
            shift,
        ),
        "ArrowRight" => handle.move_cursor(
            if ctrl {
                CursorMotion::WordRight
            } else {
                CursorMotion::CharRight
            },
            shift,
        ),
        // Page Up / Down (#2020), Chrome's bindings: on macOS the plain key
        // scrolls a page and leaves the caret, Option moves it and Shift
        // extends; elsewhere the key moves and Shift extends. Ctrl (Cmd) and
        // Alt there are the browser's.
        "PageUp" | "PageDown" if is_mac() && !event.meta_key() && !event.ctrl_key() => page_step(
            &handle,
            container_nid,
            doc,
            key == "PageDown",
            shift,
            !alt && !shift,
        ),
        "PageUp" | "PageDown" if !is_mac() && !ctrl && !alt => {
            page_step(&handle, container_nid, doc, key == "PageDown", shift, false)
        }
        "Home" if ctrl => handle.move_cursor(CursorMotion::DocStart, shift),
        "End" if ctrl => handle.move_cursor(CursorMotion::DocEnd, shift),
        // The visual line's edges, as on desktop (#301).
        "Home" => move_to_line_edge(&handle, false, shift),
        "End" => move_to_line_edge(&handle, true, shift),
        // 2. Tab. Offered to the editor first (`tab_key`: a table cell, a list
        //    binding that applies, per its `TabBehavior`); when it has no use for
        //    it — a read-only editor (#833), `MoveFocus`, a plain paragraph
        //    (#2210: the keymap's `sinkListItem` did nothing there and the
        //    matched binding was consumed, a keyboard trap) — Tab and Shift+Tab
        //    move the focus on from the editor's place in the page.
        //    Shift+Tab with no Tab stop before the editor (and no trap) leaves the
        //    page backward, as from a native `<textarea>` that is the first
        //    stop: the focus goes to the document, so the capture textarea is
        //    blurred and the key consumed. The browser's own Shift+Tab would start
        //    from the capture textarea at the end of `<body>` and land on the
        //    page's last control instead. Forward past the last stop is left to
        //    the browser, whose Tab from there leaves the page as native does.
        //    (The guard runs the editor's use of the key, so a used Tab takes
        //    the `handled` path below and the caret is redrawn.)
        "Tab" if !ctrl && !alt && handle.tab_key(shift) => true,
        "Tab" if !ctrl && !alt => {
            let Some(container) =
                node_by_nid(container_nid).and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                return false;
            };
            if crate::event_delegation::tab_from_editor(doc, &container, shift) {
                return true;
            }
            if shift {
                blur_editor(container_nid);
                return true;
            }
            return false;
        }
        "Tab" if handle.tab_cell(shift) => true,
        _ => {
            // 3. THE KEYMAP — the single source of truth for every command key. Letters
            //    resolve logically (`key`, layout-correct), digits/symbols physically
            //    (`code`). A matched binding is consumed even if its command no-op'd
            //    here (Ctrl+Shift+R on a right-aligned paragraph must not reload the
            //    page), except a fall-through one (`Plugin::fall_through_keys`: the
            //    base keymap's Mod+Enter in a paragraph), which is left unconsumed so
            //    the page's `keydown` listeners hear it (#2993).
            if let Some(binding) = editor_key_binding(&key, &code, ctrl, shift, alt)
                && handle.dispatch_key(binding).is_some()
            {
                true
            }
            // 4. Plain text insertion — a single non-control char, no modifier. Ctrl/Alt
            //    combos (incl. Ctrl+C/X/V) stay UNCONSUMED so the browser fires its native
            //    ClipboardEvent — never bind clipboard keys in any plugin keymap.
            else if ctrl || alt {
                false
            } else if key.chars().count() == 1 && !key.chars().next().unwrap().is_control() {
                // A read-only editor refuses the text but still owns the key, as
                // on desktop: left unconsumed, a typed letter would go on to the
                // page's own shortcuts and Space would scroll it.
                handle.insert_text(&key) || handle.refuses_edits()
            } else {
                false
            }
        }
    };
    if handled {
        refresh_caret();
    }
    handled
}
