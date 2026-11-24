//! Pointer input: links, hover, and mouse presses and moves in an editor.

use super::capture::{
    blur_capture_target, editor_owns_keyboard, focus_capture_target, is_non_focusable_handler_press,
};
use super::context_menu::{
    MenuTrigger, context_menu_claimed_by_app, end_context_menu_cycle, is_context_press,
    park_around, start_context_menu_cycle,
};
use super::motion::hit_affinity;
use super::touch::is_compat_mousedown_of_serviced_tap;
use super::*;

// ── Pointer ──────────────────────────────────────────────────────────────────

/// The link on the character under viewport point `(x, y)`, given the element the
/// pointer is over (`target`) inside the editor `editor_el`.
///
/// The browser's own hit test decides *whether* a link is under the pointer — the
/// `<a data-pm-mark="link">` it found — and the model decides which one, as a
/// whole run. The browser's caret-from-point call
/// (`event_delegation::caret_point_from_point`: `caretRangeFromPoint`, or
/// `caretPositionFromPoint` where that is missing) answers the nearest caret boundary, which is
/// the start **or the end** of the character under the pointer, so the character
/// is the one after that boundary if it carries the element's `href`, and else the
/// one before it. That is what keeps the space after a link off the link: no `<a>`
/// is under the pointer there.
fn link_under_pointer(
    doc: &web_sys::Document,
    target: &web_sys::Element,
    editor_el: &web_sys::Element,
    handle: &EditorHandle,
    container_nid: u64,
    x: f32,
    y: f32,
) -> Option<LinkSpan> {
    let anchor = target
        .closest("a[data-pm-mark='link']")
        .ok()
        .flatten()
        .filter(|a| editor_el.contains(Some(a)))?;
    let href = anchor.get_attribute("href").unwrap_or_default();
    let hit = resolve_editor_point(doc, x, y).filter(|hit| hit.container_nid == container_nid)?;
    let pos = hit.pos(handle)?;
    handle
        .link_at(pos)
        .filter(|link| link.href == href)
        .or_else(|| {
            let before = pos.0.checked_sub(1)?;
            handle.link_at(Pos(before)).filter(|link| link.href == href)
        })
}

/// The client rect of `link`'s run: a DOM `Range` from its first character to past
/// its last, measured by the browser (a wrapped link is the union of its lines).
fn link_client_rect(
    handle: &EditorHandle,
    doc: &web_sys::Document,
    link: &LinkSpan,
) -> Option<rinch_core::ElementBounds> {
    let (from_nid, from_byte) = handle.caret_address(link.from)?;
    let (to_nid, to_byte) = handle.caret_address(link.to)?;
    let (start, start_off) = find_text_node_at_byte_offset(&node_by_nid(from_nid)?, from_byte)?;
    let (end, end_off) = find_text_node_at_byte_offset(&node_by_nid(to_nid)?, to_byte)?;
    let range = doc.create_range().ok()?;
    range.set_start(&start, start_off).ok()?;
    range.set_end(&end, end_off).ok()?;
    let r = range.get_bounding_client_rect();
    Some(rinch_core::ElementBounds {
        x: r.x() as f32,
        y: r.y() as f32,
        width: r.width() as f32,
        height: r.height() as f32,
    })
}

/// Offer a primary press on a link to the editor's `on_link_click`. `true` when the
/// app claimed it — the caller then places no caret, starts no drag and leaves the
/// selection alone.
fn offer_link_click(
    event: &web_sys::MouseEvent,
    doc: &web_sys::Document,
    target: &web_sys::Element,
    editor_el: &web_sys::Element,
    handle: &EditorHandle,
    container_nid: u64,
) -> bool {
    let (x, y) = (event.client_x() as f32, event.client_y() as f32);
    let Some(link) = link_under_pointer(doc, target, editor_el, handle, container_nid, x, y) else {
        return false;
    };
    let (ctrl, meta) = (event.ctrl_key(), event.meta_key());
    handle.dispatch_link_click(&LinkClick {
        link,
        primary: if is_mac() { meta } else { ctrl },
        ctrl,
        meta,
        shift: event.shift_key(),
        alt: event.alt_key(),
    })
}

/// Report the link under the pointer to the editors' `on_link_hover` callbacks,
/// which the registry fires only on a change. Called for a `mousemove` that is not
/// a drag-select, and only while some editor has a hover callback. A move over no
/// link element costs one `closest` walk.
pub(super) fn update_link_hover(event: &web_sys::MouseEvent, doc: &web_sys::Document) {
    let hovered = (|| {
        let target = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())?;
        // Cheap first: no link element under the pointer, no link.
        target.closest("a[data-pm-mark='link']").ok().flatten()?;
        let editor_el = crate::event_delegation::editor_owning(&target)?;
        let container_nid = get_nid(&editor_el.clone().into())?.0;
        let handle = registry::editor_for(container_nid)?;
        let (x, y) = (event.client_x() as f32, event.client_y() as f32);
        let link = link_under_pointer(doc, &target, &editor_el, &handle, container_nid, x, y)?;
        let rect = link_client_rect(&handle, doc, &link).unwrap_or_default();
        Some((handle, LinkHover { link, rect }))
    })();
    registry::set_link_hover(None, hovered);
}

/// Report the image under the pointer to the editors' `on_image_hover`
/// callbacks, which the registry fires only on a change. Called for a
/// `mousemove` that is not a drag-select, and only while some editor has an
/// image hover callback. A move over no editor image costs one `closest` walk.
pub(super) fn update_image_hover(event: &web_sys::MouseEvent) {
    let hovered = (|| {
        let target = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())?;
        let img = target.closest("[data-pm-type='image']").ok().flatten()?;
        let editor_el = crate::event_delegation::editor_owning(&img)?;
        let container_nid = get_nid(&editor_el.clone().into())?.0;
        let handle = registry::editor_for(container_nid)?;
        let host_nid = get_nid(&img.clone().into())?.0;
        let (pos, attrs) = handle.image_at_host(host_nid)?;
        let r = img.get_bounding_client_rect();
        let rect = rinch_core::ElementBounds {
            x: r.x() as f32,
            y: r.y() as f32,
            width: r.width() as f32,
            height: r.height() as f32,
        };
        Some((handle, ImageHover { pos, attrs, rect }))
    })();
    registry::set_image_hover(None, hovered);
}

/// The first text node under `node`, in document order.
fn first_text_node(node: &web_sys::Node) -> Option<web_sys::Node> {
    let kids = node.child_nodes();
    for i in 0..kids.length() {
        let kid = kids.item(i)?;
        if kid.node_type() == web_sys::Node::TEXT_NODE {
            return Some(kid);
        }
        if let Some(found) = first_text_node(&kid) {
            return Some(found);
        }
    }
    None
}

/// The link whose `<a data-pm-mark="link">` element is `anchor`, as the model's
/// whole run: the link carrying the element's first character, if it has the
/// element's `href`. For a click with no pointer (keyboard activation), where
/// there is no point to ask the character under.
fn link_of_anchor(anchor: &web_sys::Element, handle: &EditorHandle) -> Option<LinkSpan> {
    if anchor.get_attribute("data-pm-mark").as_deref() != Some("link") {
        return None; // an `<a>` a plugin's widget drew: not a link mark
    }
    let href = anchor.get_attribute("href").unwrap_or_default();
    let textblock = {
        let mut cur = anchor.parent_element();
        loop {
            let el = cur?;
            if el.has_attribute("data-pm-editor") {
                return None;
            }
            if el.has_attribute("data-pm-type") {
                break el;
            }
            cur = el.parent_element();
        }
    };
    let textblock_nid = get_nid(&textblock.clone().into())?.0;
    let text = first_text_node(anchor)?;
    let offset = model_offset_in_block(&textblock, &text, 0);
    let pos = handle.pos_in_textblock(textblock_nid, offset)?;
    handle.link_at(pos).filter(|link| link.href == href)
}

/// A `click` (or middle-button `auxclick`) on an `<a href>` inside an editor.
///
/// **Keyboard activation first.** An editor link is an ordinary Tab stop (the
/// editor is not `contenteditable`), and Enter on a focused one arrives as a
/// `click` with `detail == 0` and no `mousedown` before it, so nothing has
/// offered it to `on_link_click` yet. It is offered here, with the link found
/// from the element rather than from a point; a claim prevents the navigation.
///
/// **Then the default action.** It is prevented when the editor is
/// **editable** — a click in an editable editor is an edit gesture (it placed
/// the caret), so the browser following the link is always wrong, which was a
/// bug before link events existed — or when it has an `on_link_click`
/// callback, which is where the app says what a link does. A **read-only**
/// editor with no callback keeps native link behaviour: a click follows the
/// link, and a middle click or Ctrl/Cmd+click opens it in a new tab. An
/// `<a href>` that a plugin's decoration widget drew follows the same rule, and
/// is never offered to `on_link_click`, which is about link marks. The
/// right-click menu's "Open link" is never touched.
pub(super) fn handle_link_click(event: &web_sys::MouseEvent) {
    let Some(anchor) = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest("a[href]").ok().flatten())
    else {
        return;
    };
    let Some(editor_el) = crate::event_delegation::editor_owning(&anchor) else {
        return;
    };
    let Some(handle) = get_nid(&editor_el.into()).and_then(|nid| registry::editor_for(nid.0))
    else {
        return;
    };
    if event.type_() == "click"
        && event.button() == 0
        && event.detail() == 0
        && let Some(link) = link_of_anchor(&anchor, &handle)
    {
        let (ctrl, meta) = (event.ctrl_key(), event.meta_key());
        let claimed = handle.dispatch_link_click(&LinkClick {
            link,
            primary: if is_mac() { meta } else { ctrl },
            ctrl,
            meta,
            shift: event.shift_key(),
            alt: event.alt_key(),
        });
        if claimed {
            event.prevent_default();
            return;
        }
    }
    if !handle.is_read_only() || handle.has_link_click_callback() {
        event.prevent_default();
    }
}

/// Handle a `mousedown`. Returns whether it landed in an editor (so the listener
/// consumes it). Mirrors `try_new_editor_click`.
pub(super) fn handle_mousedown(event: &web_sys::MouseEvent, doc: &web_sys::Document) -> bool {
    // A touch tap's compatibility `mousedown`, for a tap its `pointerup` already
    // serviced: consumed, and nothing redone (issue #302).
    if is_compat_mousedown_of_serviced_tap(event, doc) {
        return true;
    }
    // A press of any button means the browser's menu is closed: the cycle ends,
    // and the textarea is back off-screen before this press is resolved.
    end_context_menu_cycle();
    let Some(mut target) = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    let x = event.client_x() as f32;
    let y = event.client_y() as f32;
    // A press that landed on the capture textarea itself found it still parked
    // under the pointer from a right press moments before. It is not a press into
    // "another text field": now that the cycle has ended it is a press on whatever
    // the editor has at that point.
    if target.has_attribute("data-pm-capture") {
        let Some(beneath) = doc.element_from_point(x, y) else {
            return false;
        };
        target = beneath;
    }
    let Some(editor_el) = crate::event_delegation::editor_owning(&target) else {
        // Outside any editor (issue #271). A press on another text field — the
        // rule keyboard activation uses — takes the keyboard. A press on a
        // non-focusable `data-rid` handler leaves it with the editor: the browser
        // would move focus to `<body>`, so the default is prevented. Any other
        // press (a focusable control, blank page) moves focus natively, and the
        // capture textarea's `blur` releases the editor and hides its caret.
        if crate::event_delegation::in_text_control(&target) {
            set_focused_editor(None);
            blur_capture_target();
        } else if event.button() == 0
            && editor_owns_keyboard(doc)
            && is_non_focusable_handler_press(&target)
        {
            event.prevent_default();
        }
        return false;
    };
    let Some(container_nid) = get_nid(&editor_el.clone().into()).map(|n| n.0) else {
        return false;
    };
    let Some(handle) = registry::editor_for(container_nid) else {
        return false;
    };
    set_focused_editor(Some(container_nid));
    set_goal_x(None);
    // Focus the hidden capture target so the browser routes clipboard / IME events
    // to this editor (a non-`contenteditable` `<div>` would receive none).
    focus_capture_target(doc, &handle);
    // A press is the user's interaction: the spellchecker checks the whole
    // document, in slices after this event (#2862). Nothing happens unless
    // the app supplied dictionaries.
    handle.spell_user_focus();

    // A click on a leaf atom node-selects it: any node the schema calls a
    // selectable leaf (an image, a horizontal rule, an `embed` — a node view's
    // frame), which `node_selection_at_host` decides; the innermost node host is
    // asked, so a press in a paragraph's text answers `None` and places a caret.
    let leaf_selection = target
        .closest("[data-pm-type]")
        .ok()
        .flatten()
        .and_then(|leaf| get_nid(&leaf.into()).map(|n| n.0))
        .and_then(|leaf_nid| handle.node_selection_at_host(leaf_nid));

    // A single primary press on a link is the app's first (`on_link_click`),
    // before anything below moves the caret. Not a press on an image (it
    // node-selects), not a context press, not a double or triple press.
    if leaf_selection.is_none()
        && event.button() == 0
        && !is_context_press(event)
        && event.detail() <= 1
        && offer_link_click(event, doc, &target, &editor_el, &handle, container_nid)
    {
        LINK_CLAIMED_IN.with(|c| c.set(Some(container_nid)));
        registry::end_drag(None);
        refresh_caret();
        return true;
    }

    // A context press — the right button, or a Control-click on macOS: the caret
    // rule native editors follow — a press *inside* the selection keeps it (that is
    // what the menu's Cut and Copy act on), a press outside moves the caret there, or
    // node-selects the leaf it lands on — and never a drag. Then the browser's own
    // menu, through the parked textarea (issue #814), unless an app handler up the
    // chain has claimed the right-click.
    if is_context_press(event) {
        let clicked = match &leaf_selection {
            Some(_) => None,
            None => resolve_editor_point(doc, x, y)
                .filter(|hit| hit.container_nid == container_nid)
                .and_then(|hit| hit.pos(&handle)),
        };
        let sel = handle.selection();
        let inside = !sel.is_empty()
            && match (&leaf_selection, clicked) {
                (Some(leaf), _) => sel.from() <= leaf.from() && leaf.to() <= sel.to(),
                (None, Some(pos)) => sel.from() <= pos && pos <= sel.to(),
                (None, None) => false,
            };
        // A link or an image outside the selection keeps the browser's own link or
        // image menu (Open link, Copy link address, Save image): nothing is parked.
        // A native `contenteditable` moves the caret onto a link it is right-clicked
        // on and leaves the selection alone over an image (measured in Chrome 153),
        // and so does this.
        let on_link_or_image = target
            .closest("a[href], img")
            .ok()
            .flatten()
            .is_some_and(|el| editor_el.contains(Some(&el)));
        // Asked **before** the caret rule below, because the caret rule can take
        // the pressed element out of the document: a decoration plugin (the
        // spellchecker's squiggle) withholds the decoration over the word the
        // caret has just moved into, and the `<span data-pm-deco>` the press
        // landed on is discarded with it. A detached node has no ancestors, so
        // the walk for an app's `data-oncontextmenu` would find nothing and the
        // textarea would be parked over an app that *had* claimed the press.
        // "Did the app claim this press" is a question about the DOM as it was
        // when the button went down, so it is answered there.
        let claimed_by_app = context_menu_claimed_by_app(&target);
        if !inside {
            match (leaf_selection, clicked) {
                (Some(_), _) if on_link_or_image => {}
                (Some(leaf), _) => handle.set_selection(leaf),
                (None, Some(pos)) => handle.set_selection(Selection::cursor(pos)),
                (None, None) => {}
            }
        }
        registry::end_drag(None);
        refresh_caret();
        if !(on_link_or_image && !inside) && !claimed_by_app {
            start_context_menu_cycle(
                &handle,
                container_nid,
                MenuTrigger::Pointer,
                park_around(x, y),
            );
        }
        return true;
    }

    if let Some(sel) = leaf_selection {
        handle.set_selection(sel);
        registry::end_drag(None);
        refresh_caret();
        return true;
    }

    // A click in a task item's checkbox gutter (left of its content, where the CSS
    // `::before` checkbox renders) toggles its `checked` state instead of placing a
    // caret. The checkbox is a pseudo-element, so the hit is geometric: compare the
    // click x against the item's first child (its content block) left edge.
    if let Some(item) = target.closest("[data-pm-type='task_item']").ok().flatten()
        && let Some(content) = item.first_element_child()
        && (x as f64) < content.get_bounding_client_rect().left()
        && let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(clicked) = hit.pos(&handle)
        && handle.toggle_task_checked_at(clicked.0)
    {
        registry::end_drag(None);
        refresh_caret();
        return true;
    }

    if let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(clicked) = hit.pos(&handle)
    {
        match event.detail() {
            2 => {
                handle.select_word_at(clicked);
                registry::end_drag(None);
            }
            n if n >= 3 => {
                handle.select_block_at(clicked);
                registry::end_drag(None);
            }
            _ if event.shift_key() => {
                let anchor = handle.selection().anchor();
                handle.set_selection(Selection::text(anchor, clicked));
                registry::begin_drag(None, container_nid, anchor.0);
            }
            _ => {
                // A press past a wrapped line's end lands on its wrap point,
                // and belongs to the line pressed on (#301).
                let affinity = hit_affinity(&handle, clicked, y).0;
                handle.set_selection_with_affinity(Selection::cursor(clicked), affinity);
                registry::begin_drag(None, container_nid, clicked.0);
            }
        }
    }
    refresh_caret();
    true
}

/// Handle a `mousemove` while a drag-select is active. Returns whether a drag was live.
pub(super) fn handle_mousemove(event: &web_sys::MouseEvent, doc: &web_sys::Document) -> bool {
    let Some((container_nid, anchor)) = registry::drag_anchor(None) else {
        return false;
    };
    // If the primary button is no longer held (a mouseup was missed — e.g. released
    // outside the window), the drag is stale: end it instead of following the cursor.
    if event.buttons() & 1 == 0 {
        registry::end_drag(None);
        return false;
    }
    let Some(handle) = registry::editor_for(container_nid) else {
        return false;
    };
    let x = event.client_x() as f32;
    let y = event.client_y() as f32;
    if let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(head) = hit.pos(&handle)
    {
        handle.set_selection(Selection::text(Pos(anchor), head));
        refresh_caret();
    }
    true
}
