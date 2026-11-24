//! Installing the editor's document listeners and its web stylesheet.

use super::capture::{blur_editor, focus_editor, on_editor_unregistered};
use super::context_menu::{
    MenuTrigger, end_context_menu_cycle, handle_contextmenu, on_foreign_history_input, park_around,
    park_right_of, point_over_editor, repark_on_release,
};
use super::keyboard::{handle_keydown, on_node_view_host};
use super::motion::key_menu_point;
use super::pictures::{on_drag_over, on_drop};
use super::pointer::{
    handle_link_click, handle_mousedown, handle_mousemove, update_image_hover, update_link_hover,
};
use super::touch::{handle_touch_tap, note_touch_down, pointer_targets_editor};
use super::*;

// ── Installation ─────────────────────────────────────────────────────────────

/// Add a capture-phase `document` listener leaked for the page lifetime.
pub(crate) fn add_capture<E: JsCast + 'static>(
    doc: &web_sys::Document,
    name: &str,
    handler: impl Fn(E) + 'static,
) {
    add_capture_on(doc.as_ref(), name, handler);
}

/// [`add_capture`] against any target. `window` is the one other target rinch
/// uses: it is one node further out than `document` on every event path, so a
/// listener there runs ahead of *every* document listener whatever order they
/// were registered in — and a `stopPropagation()` from it stops all of them.
/// That is what the menu bar's chord check needs (`menu_bar.rs`), and what
/// keeps the pointer-gesture observer seeing a key the chord check consumed.
pub(crate) fn add_capture_on<E: JsCast + 'static>(
    target: &web_sys::EventTarget,
    name: &str,
    handler: impl Fn(E) + 'static,
) {
    let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
        if let Ok(ev) = e.dyn_into::<E>() {
            handler(ev);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    target
        .add_event_listener_with_callback_and_bool(name, closure.as_ref().unchecked_ref(), true)
        .ok();
    closure.forget();
}

/// Add a (bubble-phase) listener to a specific `target`, leaked for its lifetime.
/// Used for the capture textarea's clipboard / composition events, which target it
/// directly (no need for document-level capture). Each runs as the focused
/// editor's root's code (#2811, `crate::mark_event_root`).
pub(super) fn add_target_listener<E: JsCast + 'static>(
    target: &web_sys::EventTarget,
    name: &str,
    handler: impl Fn(E) + 'static,
) {
    let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
        let _root = crate::mark_event_root(&e);
        if let Ok(ev) = e.dyn_into::<E>() {
            handler(ev);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    target
        .add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())
        .ok();
    closure.forget();
}

/// A small web-only override of the shared editor default stylesheet. The shared
/// stylesheet (`rinch_editor_view`'s `styles.rs`) gives `li` no `display` of its own
/// and hangs the *desktop* renderer's generated marker span (rinch-dom has no
/// `list-item`) outside the item, while the editor's UA sheet makes `li` a block — so
/// on the web, where the browser draws the native `::marker`, bullets and numbers would
/// vanish. Restoring `display: list-item` brings them back. Scoped one level deeper (`ul/ol > li`) than the
/// base `li` rule so it wins by specificity regardless of `<style>` source order.
const EDITOR_WEB_CSS: &str =
    "[data-pm-editor] ul > li, [data-pm-editor] ol > li { display: list-item; }";

/// Inject [`EDITOR_WEB_CSS`] into the document head once (idempotent), so the browser
/// renders native list markers despite the shared stylesheet's desktop `li` flex rule.
fn ensure_editor_web_styles(doc: &web_sys::Document) {
    if doc
        .query_selector("style[data-rinch-editor-web]")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Ok(style) = doc.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("data-rinch-editor-web", "true");
    style.set_text_content(Some(EDITOR_WEB_CSS));
    if let Some(head) = doc.head() {
        let _ = head.append_child(&style);
    }
}

/// What must be in place before a mount's build runs, because the build itself
/// reports to it: a node view mounted by an editor in the build tells the
/// node-view host listener its host, once, there and then. Installed after the
/// build (with the rest of [`install`](fn@install)), the page's first mount told nobody, so
/// its views were never observed and a view that grew left the caret and the
/// highlight below it where they were (#2075). Called from `mount_tree` before
/// the build. Setting the listener again is harmless.
pub(crate) fn install_before_build() {
    // A node view that resizes moves what is below it (the caret, a highlight).
    registry::set_node_view_host_listener(on_node_view_host);
}

/// Install the editor input listeners once. Called from `mount_tree` alongside
/// `ensure_event_delegation`. Idempotent.
pub(crate) fn install(browser_doc: &web_sys::Document) {
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }

    // Patch the one editor default-stylesheet rule that is desktop-specific (see below).
    ensure_editor_web_styles(browser_doc);

    // The caret is refreshed from the input handlers below, and a remote
    // collaboration delta arrives through none of them: without this a peer
    // shortening the caret's line leaves the caret painted out past the text.
    // `EditorHandle::set_read_only` arrives through none of them either, and the
    // same refresh is what carries it to the capture textarea's `readonly`.
    registry::set_overlay_refresher(refresh_caret);
    // A selection moved by app code — a timer, an effect, a toolbar button's
    // click — arrives through none of them either (#1001): the handle owes an
    // overlay pass, and this runs it once the current task's code is done.
    registry::set_overlay_pass_scheduler(schedule_overlay_pass);
    // A selection moved before this ran — during the first mount's build —
    // was owed with no scheduler to tell, and a later owe of an already-owed
    // document tells none either: run that pass now it can be scheduled.
    if registry::any_overlay_pass_owed() {
        schedule_overlay_pass();
    }
    // `EditorHandle::focus`: the keyboard reaches an editor only through the
    // capture textarea, so a programmatic focus has to go through the same
    // steps a press does.
    registry::set_focus_handler(focus_editor);
    // `EditorHandle::blur`: its other half.
    registry::set_blur_handler(blur_editor);
    // An editor unmounting while it holds the keyboard must take its text out
    // of the capture textarea with it (#1112).
    registry::set_unregister_listener(on_editor_unregistered);
    // The node-view host listener was installed before the build
    // (`install_before_build`).

    let doc = browser_doc.clone();
    add_capture(browser_doc, "keydown", move |e: web_sys::KeyboardEvent| {
        // The editor's hooks run as its root's code (#2811).
        let _root = crate::mark_event_root(&e);
        if handle_keydown(&e, &doc) {
            e.prevent_default();
            e.stop_propagation();
        }
    });
    // Files dragged from outside the page onto an editor whose app takes
    // pictures itself (`EditorHandle::on_image_input`).
    add_capture(browser_doc, "dragover", |e: web_sys::DragEvent| {
        on_drag_over(&e);
    });
    let doc = browser_doc.clone();
    add_capture(browser_doc, "drop", move |e: web_sys::DragEvent| {
        let _root = crate::mark_event_root(&e);
        on_drop(&e, &doc);
    });
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mousedown", move |e: web_sys::MouseEvent| {
        let _root = crate::mark_event_root(&e);
        if handle_mousedown(&e, &doc) {
            e.prevent_default();
            e.stop_propagation();
        }
    });
    // Mirror the mousedown gate onto `pointerdown`. The generic event delegation
    // (`event_delegation.rs`) now delegates on `pointerdown` (bubble phase) — a
    // separate dispatch that the `stop_propagation()` on `mousedown` above cannot
    // stop. Consume an in-editor pointerdown here in the capture phase (document
    // capture fires before the generic bubble handler) so it never reaches the
    // generic click / `data-onmousedown` dispatch — which would otherwise fire an
    // ancestor's `onclick` when an editor is nested inside a clickable card/row.
    // Caret placement still happens from the `mousedown` handler above; we only
    // suppress the redundant generic delegation, never `prevent_default` (which
    // could swallow the compatibility `mousedown` the editor relies on).
    add_capture(
        browser_doc,
        "pointerdown",
        move |e: web_sys::PointerEvent| {
            if pointer_targets_editor(&e) {
                // Remember the contact so `pointerup` can tell a tap from a scroll and
                // raise the on-screen keyboard inside the gesture (see `handle_touch_tap`).
                note_touch_down(&e);
                e.stop_propagation();
            }
        },
    );
    let doc = browser_doc.clone();
    add_capture(browser_doc, "pointerup", move |e: web_sys::PointerEvent| {
        handle_touch_tap(&e, &doc);
    });
    add_capture(
        browser_doc,
        "pointercancel",
        move |e: web_sys::PointerEvent| {
            // Only the recorded contact's own cancellation ends its tap — matching the
            // pointer-id check in `handle_touch_tap`. A second finger's cancel (or any
            // unrelated pointercancel on the page) must not discard a live tap.
            TOUCH_TAP.with(|c| {
                if c.get().is_some_and(|(id, _, _)| id == e.pointer_id()) {
                    c.set(None);
                }
            });
        },
    );
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mousemove", move |e: web_sys::MouseEvent| {
        if handle_mousemove(&e, &doc) {
            e.prevent_default();
        } else {
            if registry::link_hover_wanted() {
                update_link_hover(&e, &doc);
            }
            if registry::image_hover_wanted() {
                update_image_hover(&e);
            }
        }
    });
    // The pointer leaving the window leaves whatever link or image it was over.
    add_capture(browser_doc, "mouseout", |e: web_sys::MouseEvent| {
        if e.related_target().is_none() {
            if registry::link_hover_wanted() {
                registry::set_link_hover(None, None);
            }
            if registry::image_hover_wanted() {
                registry::set_image_hover(None, None);
            }
        }
    });
    // Whether an editor link navigates, and keyboard activation of one (see
    // `handle_link_click`).
    add_capture(browser_doc, "click", |e: web_sys::MouseEvent| {
        // A tap's `click` ends it: any compatibility `mousedown` came before it.
        SERVICED_TAP.with(|c| c.set(None));
        let _root = crate::mark_event_root(&e);
        handle_link_click(&e);
    });
    add_capture(browser_doc, "auxclick", |e: web_sys::MouseEvent| {
        if e.button() == 1 {
            let _root = crate::mark_event_root(&e);
            handle_link_click(&e);
        }
    });
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mouseup", move |e: web_sys::MouseEvent| {
        registry::end_drag(None);
        // A right-button release: on a platform whose `contextmenu` follows the
        // release, the browser hit-tests for its menu right after this event — for
        // the editor's menu only if the release is still over that editor.
        if e.button() == 2 {
            let (x, y) = (e.client_x() as f32, e.client_y() as f32);
            repark_on_release(MenuTrigger::Pointer, |container_nid| {
                point_over_editor(&doc, x, y, container_nid).then(|| park_around(x, y))
            });
        }
    });
    // The menu key's release, for the same platform (issue #814). One whose press
    // went to an app's `data-oncontextmenu` is withheld from the browser as well.
    add_capture(browser_doc, "keyup", move |e: web_sys::KeyboardEvent| {
        if e.key() != "ContextMenu" {
            return;
        }
        if MENU_KEY_TO_APP.with(|c| c.replace(false)) {
            e.prevent_default();
            return;
        }
        repark_on_release(MenuTrigger::Key, |_| {
            focused_handle()
                .and_then(|(id, handle)| key_menu_point(&handle, id))
                .map(|(x, y)| park_right_of(x, y))
        });
    });
    // The browser's `contextmenu` for a context press or key (issue #814). Capture
    // phase, so the generic delegation's own `contextmenu` listener sees the same
    // target it always would.
    add_capture(browser_doc, "contextmenu", |e: web_sys::MouseEvent| {
        let _root = crate::mark_event_root(&e);
        handle_contextmenu(&e);
    });
    // Focus moving to any other element ends a menu cycle: whatever the menu was, it
    // is gone, and the field must not keep the sentinels.
    add_capture(browser_doc, "focusin", |e: web_sys::FocusEvent| {
        let to_capture = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| el.has_attribute("data-pm-capture"));
        if !to_capture {
            end_context_menu_cycle();
        }
    });
    // Undo / Redo chosen from the browser's own menu while the editor holds focus.
    add_capture(browser_doc, "beforeinput", |e: web_sys::InputEvent| {
        on_foreign_history_input(&e);
    });

    // Bubble-phase refresh: after an *outside* click that wasn't consumed in capture
    // (e.g. a toolbar button dispatched `handle.command`), re-render the focused
    // editor's caret/selection. Registered after `ensure_event_delegation`, so it runs
    // once the button's handler has applied its edit.
    let refresh = Closure::wrap(Box::new(move |_e: web_sys::Event| {
        if focused_editor().is_some() {
            refresh_caret();
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    // The same after a command run with no pointer event while the editor keeps
    // the keyboard (issue #271): assistive technology or `element.click()` on a
    // toolbar `<button>` fires only a `click`, and no input event this module
    // refreshes from. (A command run by Enter or Space on a focused toolbar
    // control needs no refresh: focus left the capture textarea, whose `blur`
    // released the editor, so it paints no caret to move.)
    for name in ["mousedown", "click"] {
        browser_doc
            .add_event_listener_with_callback(name, refresh.as_ref().unchecked_ref())
            .ok();
    }
    refresh.forget();

    // Caret blink (530 ms half-period — the platform default).
    if let Some(win) = web_sys::window() {
        let tick = Closure::wrap(Box::new(blink_tick) as Box<dyn FnMut()>);
        win.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            530,
        )
        .ok();
        tick.forget();
    }
}
