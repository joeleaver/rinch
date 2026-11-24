//! The hidden capture textarea: creating it, focusing and blurring it for an
//! editor, its listeners, and which editor owns the keyboard.

use super::clipboard::{on_copy, on_cut, on_paste};
use super::context_menu::{HIDDEN_CAPTURE_STYLE, end_context_menu_cycle, on_select};
use super::install::add_target_listener;
use super::mirror::clear_mirror;
use super::text_input::{on_before_input, on_composition_end, on_composition_update, on_input};
use super::*;

// ── Hidden capture target (clipboard + IME) ───────────────────────────────────

/// Get-or-create the shared hidden capture `<textarea>`, appending it to `<body>`
/// and attaching its clipboard + composition listeners on first use. `None` only
/// if the DOM rejects the element (e.g. no `<body>` yet).
pub(super) fn ensure_capture_target(
    doc: &web_sys::Document,
) -> Option<web_sys::HtmlTextAreaElement> {
    if let Some(ta) = capture_target() {
        return Some(ta);
    }
    let ta: web_sys::HtmlTextAreaElement = doc.create_element("textarea").ok()?.dyn_into().ok()?;
    ta.set_attribute("data-pm-capture", "true").ok();
    ta.set_attribute("aria-hidden", "true").ok();
    ta.set_attribute("tabindex", "-1").ok();
    ta.set_autocomplete("off");
    ta.set_spellcheck(false);
    ta.set_attribute("autocorrect", "off").ok();
    ta.set_attribute("autocapitalize", "off").ok();
    ta.set_attribute("style", HIDDEN_CAPTURE_STYLE).ok();
    doc.body()?.append_child(&ta).ok()?;
    install_capture_listeners(&ta);
    CAPTURE.with(|c| *c.borrow_mut() = Some(ta.clone()));
    Some(ta)
}

/// The capture textarea, if it has been created.
pub(super) fn capture_target() -> Option<web_sys::HtmlTextAreaElement> {
    CAPTURE.with(|c| c.borrow().clone())
}

/// When `target` is the capture textarea, the document of the editor it is
/// typing for, the focused one (#2811). The textarea lives in the page's
/// `<body>`, outside every root, so its events cannot be placed by where they
/// land.
pub(crate) fn capture_target_doc(target: Option<&web_sys::Node>) -> Option<u64> {
    let ta = CAPTURE.with(|c| c.borrow().clone())?;
    if target? != AsRef::<web_sys::Node>::as_ref(&ta) {
        return None;
    }
    let focused = focused_editor()?;
    registry::all_editors()
        .into_iter()
        .find(|&(_, container, _)| container == focused)
        .and_then(|(doc, _, _)| rinch_core::doc_identity(doc))
}

/// Focus the capture target so the browser routes clipboard / IME events to it.
/// `preventScroll` keeps focusing the off-screen target from jumping the page.
pub(super) fn focus_capture_target(doc: &web_sys::Document, handle: &EditorHandle) {
    if let Some(ta) = ensure_capture_target(doc) {
        // Before the focus, not after: focusing a writable field is what raises a
        // soft keyboard, and a read-only editor must not flash one up.
        sync_capture_read_only(handle);
        let opts = web_sys::FocusOptions::new();
        opts.set_prevent_scroll(true);
        let _ = ta.focus_with_options(&opts);
        // Start from an empty field; the `refresh_caret` that ends every focusing
        // `handle_mousedown` fills it from the caret's block. Clear the mirror with it —
        // an emptied field behind a live mirror reads as "the block was deleted".
        clear_mirror(&ta);
    }
}

/// Give the editor at `container_nid` the keyboard without a press —
/// [`EditorHandle::focus`], through [`registry::set_focus_handler`]. The focusing
/// half of [`handle_mousedown`]: the editor becomes the focused one, the capture
/// textarea takes the browser's focus (`preventScroll`, so nothing jumps; any
/// other field blurs natively and commits its `change`), and the overlay pass
/// draws the editor's caret or selection where it already is. Nothing is
/// selected and nothing scrolls.
///
/// [`handle_mousedown`]: super::pointer::handle_mousedown
pub(super) fn focus_editor(container_nid: u64) {
    let Some(handle) = registry::editor_for(container_nid) else {
        return;
    };
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    end_context_menu_cycle();
    set_focused_editor(Some(container_nid));
    set_goal_x(None);
    focus_capture_target(&doc, &handle);
    refresh_caret();
}

/// Take the keyboard away from the editor at `container_nid` —
/// [`EditorHandle::blur`], through [`registry::set_blur_handler`]. Only if that
/// editor is the focused one: then the capture textarea is blurred, as a press
/// on page content that takes no focus would blur it, and the browser's focus
/// goes to the body. The editor is released here rather than left to
/// [`on_capture_blur`], which keeps the editor while the page itself lacks the
/// focus (a window switch); a blur from code is meant whatever the window is
/// doing. Any other editor or field that holds the keyboard is left alone, and
/// the selection is kept for a later focus.
pub(super) fn blur_editor(container_nid: u64) {
    if focused_editor() != Some(container_nid) {
        return;
    }
    end_context_menu_cycle();
    // The textarea is blurred while the editor is still the focused one: a
    // browser ends a live composition inside `ta.blur()` (`compositionend`,
    // before `blur`), and that has to reach this editor to commit the text
    // and take its preedit down, as it does when a press moves focus away.
    if let Some(ta) = capture_target() {
        let field_active = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.active_element().as_ref() == Some(ta.as_ref()));
        if field_active {
            let _ = ta.blur();
        }
    }
    // Released here whatever `on_capture_blur` did (it keeps the editor while
    // the page itself lacks the focus).
    set_focused_editor(None);
    set_goal_x(None);
    refresh_caret();
}

/// The editor at `container_nid` has unmounted ([`registry::set_unregister_listener`]).
///
/// If it was the focused one, the capture textarea still holds the keyboard,
/// its block's text as the mirror (U+FFFC for an image, `\n` for a break) and
/// the model selection over it: a Ctrl+C there would be the browser's native
/// copy of a document that is no longer on the page, and the text would stay
/// in the DOM (issue #1112). Release the editor, empty the field with its
/// mirror, and blur it — what a browser does when a focused element leaves the
/// document. Releasing first keeps the `blur` this raises from refreshing an
/// editor that is gone.
///
/// If it was not the focused one, the field is left alone while it holds the
/// keyboard — it is then another editor's, mirror and all. But when the field
/// is **not** `document.activeElement`, no editor is reading it: focus left the
/// editor for somewhere else before the unmount, and the blur released the
/// editor while leaving its block in the field. Empty it then too, so that text
/// does not outlive the editor either. Nothing is lost: the next focus empties
/// the field anyway ([`focus_capture_target`]) and refills it from that editor's
/// caret block.
pub(super) fn on_editor_unregistered(_doc_key: u64, container_nid: u64) {
    let ta = capture_target();
    if focused_editor() == Some(container_nid) {
        set_focused_editor(None);
        end_context_menu_cycle();
        COMPOSING.with(|c| c.set(false));
        if let Some(ta) = ta {
            clear_mirror(&ta);
            let _ = ta.blur();
        }
        return;
    }
    let Some(ta) = ta else {
        return;
    };
    let field_active = web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.active_element().as_ref() == Some(ta.as_ref()));
    if !field_active {
        clear_mirror(&ta);
    }
}

/// Blur the capture target (when focus leaves the editor for another field).
pub(super) fn blur_capture_target() {
    end_context_menu_cycle();
    if let Some(ta) = capture_target() {
        let _ = ta.blur();
    }
}

/// Attach the clipboard, IME composition, and `beforeinput` listeners to the capture
/// target, once when it is created. These events only fire on the focused editable, so
/// scoping them to our textarea avoids hijacking copy/paste — or a soft keyboard's
/// edits — for any other page input.
fn install_capture_listeners(ta: &web_sys::HtmlTextAreaElement) {
    let target: &web_sys::EventTarget = ta.as_ref();
    add_target_listener(target, "copy", |e: web_sys::ClipboardEvent| on_copy(&e));
    add_target_listener(target, "cut", |e: web_sys::ClipboardEvent| on_cut(&e));
    add_target_listener(target, "paste", |e: web_sys::ClipboardEvent| on_paste(&e));
    add_target_listener(target, "beforeinput", |e: web_sys::InputEvent| {
        on_before_input(&e);
    });
    add_target_listener(target, "input", |_e: web_sys::InputEvent| on_input());
    add_target_listener(target, "select", |_e: web_sys::Event| on_select());
    add_target_listener(target, "selectionchange", |_e: web_sys::Event| on_select());
    add_target_listener(
        target,
        "compositionstart",
        |_e: web_sys::CompositionEvent| {
            COMPOSING.with(|c| c.set(true));
        },
    );
    add_target_listener(
        target,
        "compositionupdate",
        |e: web_sys::CompositionEvent| {
            on_composition_update(&e);
        },
    );
    add_target_listener(target, "compositionend", |e: web_sys::CompositionEvent| {
        on_composition_end(&e);
    });
    // Focus leaving the textarea for somewhere else in the page releases the
    // editor (issue #271): it owns no key from then on (`handle_keydown`), so it
    // must paint no caret either — desktop hides a blurred editor's overlays
    // (`focus.rs`). Typed as a plain `Event`, since page script can dispatch a
    // bare one named `blur`.
    add_target_listener(target, "blur", |_e: web_sys::Event| on_capture_blur());
}

/// The capture textarea lost focus. Release the editor unless the textarea is
/// still the document's focused element or the page itself lost focus — what a
/// *window* blur leaves (alt-tab away: focus comes back to it on return, as
/// desktop's `WindowFocus(false)` keeps the claim, #226). No guard for a #814
/// menu cycle: a press or key that moves focus ends the cycle first
/// (`end_context_menu_cycle`), and the `editor_native_context_menu` suite passes
/// without one.
fn on_capture_blur() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let still_focused = capture_target().is_some_and(|ta| {
        let ta: &web_sys::Element = ta.as_ref();
        doc.active_element().as_ref() == Some(ta)
    });
    let page_has_focus = doc.has_focus().unwrap_or(false);
    if still_focused || !page_has_focus {
        return;
    }
    if focused_editor().is_some() {
        set_focused_editor(None);
        refresh_caret();
    }
}

/// The editor holding the keyboard, as `(container id, container element)`:
/// the focused editor, while the capture textarea is `doc`'s focused element
/// (issue #1126).
///
/// The capture field lives in `<body>`, outside every editor, so
/// `document.activeElement` alone says the focus is outside whatever holds the
/// editor. Every question rinch asks of the focus means the editor there, which
/// is what this answers. The container may be detached (its overlay's subtree
/// already taken out); `contains` still answers for it.
pub(crate) fn keyboard_editor(doc: &web_sys::Document) -> Option<(u64, web_sys::Element)> {
    let ta = capture_target()?;
    let ta: &web_sys::Element = ta.as_ref();
    if doc.active_element().as_ref() != Some(ta) {
        return None;
    }
    let id = focused_editor()?;
    registry::editor_for(id)?;
    let el = crate::web_document::registered_node(id)?
        .dyn_into::<web_sys::Element>()
        .ok()?;
    Some((id, el))
}

/// `doc.activeElement`, except that the capture textarea of the editor that
/// holds the keyboard answers that editor's container ([`keyboard_editor`]).
/// What "where is the focus" means to rinch's own focus rules: the overlay
/// restore, Tab containment, `NodeHandle::active_element`, a removal's focus
/// release.
pub(crate) fn effective_active_element(doc: &web_sys::Document) -> Option<web_sys::Element> {
    keyboard_editor(doc)
        .map(|(_, el)| el)
        .or_else(|| doc.active_element())
}

/// Take the keyboard from the editor at `container_nid`, if it holds it —
/// [`EditorHandle::blur`]'s path: the capture field is blurred (a live
/// composition ends into this editor first) and the editor is released.
pub(crate) fn release_editor(container_nid: u64) {
    blur_editor(container_nid);
}

/// Give the editor at `container_nid` the keyboard, as [`EditorHandle::focus`]
/// does. Answers whether it holds it afterwards.
/// Refused, as a browser's `focus()` refuses, for an editor whose container is
/// detached or not rendered.
pub(crate) fn give_editor_keyboard(doc: &web_sys::Document, container_nid: u64) -> bool {
    let shown = crate::web_document::registered_node(container_nid)
        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| el.is_connected() && crate::event_delegation::element_is_visible(&el));
    if !shown {
        return false;
    }
    focus_editor(container_nid);
    keyboard_editor(doc).is_some_and(|(id, _)| id == container_nid)
}

/// Whether an editor holds the keyboard right now: one is focused and the
/// capture textarea is the document's focused element.
pub(super) fn editor_owns_keyboard(doc: &web_sys::Document) -> bool {
    let on_capture = capture_target().is_some_and(|ta| {
        let ta: &web_sys::Element = ta.as_ref();
        doc.active_element().as_ref() == Some(ta)
    });
    on_capture && focused_handle().is_some()
}

/// Whether a press at `target` lands on a `data-rid` handler with nothing
/// focusable anywhere on its ancestor chain — a DOM menu-bar item, a
/// `div { onclick }` toolbar button. (A `DropdownMenu` item is a `<button>`, so it is focusable.) The
/// browser would move focus to `<body>` for such a press; desktop keeps the
/// editor focused for it (`click_handling.rs`: a press on a `data-rid` preserves
/// editor focus, only a focusable node claims it), and so does this module, by
/// `preventDefault`ing the `mousedown`.
pub(super) fn is_non_focusable_handler_press(target: &web_sys::Element) -> bool {
    if target.closest("[data-rid]").ok().flatten().is_none() {
        return false;
    }
    // Any focusable on the chain — the handler itself, inside it, or an
    // ancestor of it (a `Tree` chevron inside its `tabindex` row) — is what
    // the browser focuses, as desktop does: leave the press alone.
    target
        .closest(crate::event_delegation::PRESS_FOCUSABLE_SELECTOR)
        .ok()
        .flatten()
        .is_none()
}
