//! Global browser event delegation for the browser-native DOM backend.
//!
//! Installs document-level listeners (mousedown/mousemove/mouseup/keydown/input)
//! that delegate to rinch's event-handler registry, drag system, and
//! render-surface focus routing. Pointer events resolve text-hit positions via
//! `caretRangeFromPoint` so contenteditable apps get accurate caret placement
//! (a no-op for apps without `data-block-index` blocks).

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_core::events;

use crate::web_document::WebDocument;

thread_local! {
    /// The element currently under the pointer that carries a `data-onenter`/
    /// `data-onleave` handler. Used to implement non-bubbling
    /// `mouseenter`/`mouseleave` semantics: enter/leave fire exactly once when
    /// the resolved handler element under the pointer changes (moving between
    /// descendants of the same handler element fires nothing). This is the
    /// correct hover model; note it does not byte-for-byte match the desktop
    /// backend, which keys on the raw hit-test node and may re-fire.
    static LAST_HOVER: std::cell::RefCell<Option<web_sys::Element>> =
        const { std::cell::RefCell::new(None) };
}

/// Map a browser `MouseEvent.button` index onto the core button enum.
fn mouse_button_from_event(event: &web_sys::MouseEvent) -> events::MouseButton {
    match event.button() {
        1 => events::MouseButton::Middle,
        2 => events::MouseButton::Right,
        _ => events::MouseButton::Left,
    }
}

/// Read the keyboard modifier state carried by a browser mouse event.
fn modifiers_from_event(event: &web_sys::MouseEvent) -> events::ModifierState {
    events::ModifierState {
        shift: event.shift_key(),
        ctrl: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

/// Compare two elements by DOM node identity.
fn same_element(a: &web_sys::Element, b: &web_sys::Element) -> bool {
    let a_node: &web_sys::Node = a.as_ref();
    let b_node: &web_sys::Node = b.as_ref();
    a_node.is_same_node(Some(b_node))
}

/// Walk up from `el` to the nearest ancestor carrying `attr`, set the
/// [`events::ClickContext`] (cursor + element bounds + button + modifiers from
/// `event`), and dispatch the registered handler. Mirrors the `data-rid`
/// pattern; used for `data-onmousedown`/`data-onmouseup`/`data-onmousemove`.
fn dispatch_mouse_attr(el: &web_sys::Element, attr: &str, event: &web_sys::MouseEvent) {
    let selector = format!("[{attr}]");
    if let Ok(Some(target_el)) = el.closest(&selector)
        && let Some(id_str) = target_el.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        let rect = target_el.get_bounding_client_rect();
        events::set_click_context(events::ClickContext {
            mouse_x: event.client_x() as f32,
            mouse_y: event.client_y() as f32,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit: Default::default(),
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: mouse_button_from_event(event),
            modifiers: modifiers_from_event(event),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

// ── Element-to-element drag-and-drop (the data-ondrag* attribute suite) ───────
//
// Synthesized from mouse events to mirror the desktop backend's DOM drag system
// (NOT the browser's native HTML5 drag events). A `draggable="true"` element
// under mousedown becomes a *pending* source; once the pointer moves past the
// threshold the drag *activates* (data-ondragstart). While active, each move
// tracks the [data-ondrop] target under the cursor (data-ondragenter/leave),
// fires data-ondragover on it and data-ondragmove on the source; mouseup fires
// data-ondrop + data-ondragend (Escape cancels). Typed payloads flow through the
// backend-agnostic `DragContext<T>` in the app's handlers — nothing here.
//
// Unlike desktop there is no drag ghost (the browser owns paint); apps render
// their own visual feedback by positioning an element from data-ondragmove.

/// Movement threshold (CSS px) before a drag activates — matches desktop.
const WEB_DRAG_THRESHOLD: f32 = 5.0;

struct WebDragState {
    /// The `draggable="true"` source element.
    source: web_sys::Element,
    /// Pointer position at mousedown (for the activation threshold).
    start_x: f32,
    start_y: f32,
    /// Last known pointer position (used by Escape-cancel).
    cursor: (f32, f32),
    /// Whether the movement threshold has been crossed.
    active: bool,
    /// The current [data-ondrop] target under the pointer.
    over_target: Option<web_sys::Element>,
}

thread_local! {
    static WEB_DRAG: std::cell::RefCell<Option<WebDragState>> =
        const { std::cell::RefCell::new(None) };
}

/// Walk up from `el` for the nearest ancestor with `attr` and dispatch its
/// handler with no ClickContext (data-ondragstart/enter/leave/drop).
fn dispatch_plain_attr(el: &web_sys::Element, attr: &str) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Dispatch `attr` with a ClickContext set to the cursor position and the
/// resolved element's bounds (data-ondragover).
fn dispatch_drag_with_bounds(el: &web_sys::Element, attr: &str, x: f32, y: f32) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        let rect = t.get_bounding_client_rect();
        events::set_click_context(events::ClickContext {
            mouse_x: x,
            mouse_y: y,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit: Default::default(),
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: events::MouseButton::Left,
            modifiers: events::ModifierState::default(),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Dispatch `attr` with a ClickContext set to the cursor position and zero
/// element bounds (data-ondragmove / data-ondragend), matching desktop.
fn dispatch_drag_cursor_only(el: &web_sys::Element, attr: &str, x: f32, y: f32) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        events::set_click_context(events::ClickContext {
            mouse_x: x,
            mouse_y: y,
            element_x: 0.0,
            element_y: 0.0,
            element_width: 0.0,
            element_height: 0.0,
            text_hit: Default::default(),
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: events::MouseButton::Left,
            modifiers: events::ModifierState::default(),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Advance an in-progress element drag on pointer move: cross the activation
/// threshold (firing data-ondragstart), then track the drop target
/// (data-ondragenter/leave) and fire data-ondragover (target) + data-ondragmove
/// (source). All WEB_DRAG borrows are released before dispatching handlers.
fn handle_web_drag_move(event: &web_sys::MouseEvent) {
    let x = event.client_x() as f32;
    let y = event.client_y() as f32;

    // Self-heal: if the primary button is no longer held, a mouseup was missed
    // (e.g. released outside the window) — cancel rather than leave the drag
    // stuck active across subsequent moves. (`&` binds looser than `==`, hence
    // the parentheses.)
    if (event.buttons() & 1) == 0 {
        cancel_web_drag();
        return;
    }

    // Snapshot current state; do not hold the borrow across dispatch.
    let Some((source, was_active, old_target, start)) = WEB_DRAG.with(|d| {
        d.borrow().as_ref().map(|s| {
            (
                s.source.clone(),
                s.active,
                s.over_target.clone(),
                (s.start_x, s.start_y),
            )
        })
    }) else {
        return;
    };

    // Record the latest cursor (used by Escape-cancel).
    WEB_DRAG.with(|d| {
        if let Some(s) = d.borrow_mut().as_mut() {
            s.cursor = (x, y);
        }
    });

    // Pending → active once the threshold is crossed.
    if !was_active {
        let dx = x - start.0;
        let dy = y - start.1;
        if (dx * dx + dy * dy).sqrt() < WEB_DRAG_THRESHOLD {
            return;
        }
        WEB_DRAG.with(|d| {
            if let Some(s) = d.borrow_mut().as_mut() {
                s.active = true;
            }
        });
        dispatch_plain_attr(&source, "data-ondragstart");
        return;
    }

    // Active: resolve the [data-ondrop] target under the cursor.
    let new_target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest("[data-ondrop]").ok().flatten());

    let changed = match (&old_target, &new_target) {
        (Some(o), Some(n)) => !same_element(o, n),
        (None, None) => false,
        _ => true,
    };
    if changed {
        if let Some(o) = &old_target {
            dispatch_plain_attr(o, "data-ondragleave");
        }
        if let Some(n) = &new_target {
            dispatch_plain_attr(n, "data-ondragenter");
        }
        WEB_DRAG.with(|d| {
            if let Some(s) = d.borrow_mut().as_mut() {
                s.over_target = new_target.clone();
            }
        });
    }

    if let Some(t) = &new_target {
        dispatch_drag_with_bounds(t, "data-ondragover", x, y);
    }
    dispatch_drag_cursor_only(&source, "data-ondragmove", x, y);
}

/// Cancel an in-progress element drag (Escape, or a mouseup missed outside the
/// window). An *active* drag fires data-ondragleave on the target and
/// data-ondragend on the source (using the last known cursor); a merely
/// *pending* drag is just cleared. Returns true if any drag — pending or active
/// — was in progress, so the caller can consume the key, matching the desktop
/// backend (which swallows Escape for both pending and active drags).
fn cancel_web_drag() -> bool {
    let Some(state) = WEB_DRAG.with(|d| d.borrow_mut().take()) else {
        return false;
    };
    if state.active {
        let (x, y) = state.cursor;
        if let Some(t) = &state.over_target {
            dispatch_plain_attr(t, "data-ondragleave");
        }
        dispatch_drag_cursor_only(&state.source, "data-ondragend", x, y);
    }
    true
}

/// Convert a UTF-16 code unit offset within a string to a UTF-8 byte offset.
fn utf16_offset_to_utf8_bytes(text: &str, utf16_offset: u32) -> usize {
    let mut utf16_count = 0u32;
    for (byte_idx, ch) in text.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        utf16_count += ch.len_utf16() as u32;
    }
    text.len() // offset is at or past the end
}

/// Use `document.caretRangeFromPoint` to resolve a click position to a text hit.
/// Returns `Some(TextHitInfo)` if the click resolved to a text position inside a block.
fn resolve_text_hit(
    browser_doc: &web_sys::Document,
    client_x: f32,
    client_y: f32,
) -> Option<events::TextHitInfo> {
    // caretRangeFromPoint is non-standard but available in Chrome/Safari/Edge
    let func = js_sys::Reflect::get(browser_doc, &"caretRangeFromPoint".into()).ok()?;
    let func: js_sys::Function = func.dyn_into().ok()?;
    let range_val = func
        .call2(
            browser_doc,
            &JsValue::from(client_x),
            &JsValue::from(client_y),
        )
        .ok()?;
    if range_val.is_null() || range_val.is_undefined() {
        return None;
    }
    let range: web_sys::Range = range_val.dyn_into().ok()?;
    let start_container = range.start_container().ok()?;
    let start_offset = range.start_offset().ok()?;

    // Walk up from start_container to find nearest ancestor with data-block-index
    let mut current: Option<web_sys::Node> = Some(start_container.clone());
    let mut block_el: Option<web_sys::Element> = None;
    while let Some(node) = current {
        if let Ok(el) = node.clone().dyn_into::<web_sys::Element>()
            && el.has_attribute("data-block-index")
        {
            block_el = Some(el);
            break;
        }
        current = node.parent_node();
    }
    let block_el = block_el?;
    let block_index: usize = block_el.get_attribute("data-block-index")?.parse().ok()?;

    // Compute byte_offset: walk all text nodes in the block before start_container,
    // summing their UTF-8 byte lengths. For start_container, convert start_offset
    // (UTF-16) to UTF-8 bytes.
    let byte_offset = compute_byte_offset_in_block(&block_el, &start_container, start_offset);

    Some(events::TextHitInfo {
        block_index,
        byte_offset,
        inline_root_node_id: 0,
        valid: true,
    })
}

/// Compute the UTF-8 byte offset within a block element by walking text nodes.
fn compute_byte_offset_in_block(
    block_el: &web_sys::Element,
    target_text_node: &web_sys::Node,
    utf16_offset_in_target: u32,
) -> usize {
    let mut byte_offset = 0usize;
    walk_text_nodes_for_offset(
        &block_el.clone().into(),
        target_text_node,
        utf16_offset_in_target,
        &mut byte_offset,
    );
    byte_offset
}

/// Depth-first walk of text nodes. Accumulates UTF-8 byte lengths.
/// When we reach `target_text_node`, adds the UTF-8 equivalent of `utf16_offset_in_target`
/// and returns true (found).
fn walk_text_nodes_for_offset(
    node: &web_sys::Node,
    target: &web_sys::Node,
    utf16_offset: u32,
    byte_offset: &mut usize,
) -> bool {
    if node.node_type() == web_sys::Node::TEXT_NODE {
        if node == target {
            let text = node.text_content().unwrap_or_default();
            *byte_offset += utf16_offset_to_utf8_bytes(&text, utf16_offset);
            return true;
        }
        let text = node.text_content().unwrap_or_default();
        *byte_offset += text.len();
        return false;
    }
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && walk_text_nodes_for_offset(&child, target, utf16_offset, byte_offset)
        {
            return true;
        }
    }
    false
}

/// Dispatch the click (`data-rid`) handler nearest to `el`, with a full
/// [`events::ClickContext`] (cursor, element bounds, text-hit, button,
/// modifiers) and `prevent_default`. Used for the mousedown click and for the
/// deferred click when a draggable's drag never activates, so both paths get
/// identical click semantics.
fn dispatch_click_at(
    el: &web_sys::Element,
    browser_doc: &web_sys::Document,
    event: &web_sys::MouseEvent,
) {
    if let Ok(Some(rid_el)) = el.closest("[data-rid]")
        && let Some(rid_str) = rid_el.get_attribute("data-rid")
        && let Ok(rid) = rid_str.parse::<usize>()
    {
        let rect = rid_el.get_bounding_client_rect();
        let text_hit = resolve_text_hit(
            browser_doc,
            event.client_x() as f32,
            event.client_y() as f32,
        )
        .unwrap_or_default();
        events::set_click_context(events::ClickContext {
            mouse_x: event.client_x() as f32,
            mouse_y: event.client_y() as f32,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit,
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: mouse_button_from_event(event),
            modifiers: modifiers_from_event(event),
        });
        // Prevent browser default behavior (e.g. text selection during slider
        // drag, <label> synthesizing extra events).
        event.prevent_default();
        events::dispatch_event(events::EventHandlerId(rid));
    }
}

/// Set up global event listeners that delegate to rinch's event handler registry.
///
/// Wires `mousedown`/`mousemove`/`mouseup` (click dispatch, `data-onmousedown`/
/// `up`/`move`, pointer-capture drag tracking, text-hit resolution, hover
/// `data-onenter`/`data-onleave`, and the element drag-and-drop `data-ondrag*`
/// suite), `keydown` (render-surface routing, keyboard interceptor,
/// `data-onsubmit` on Enter, Escape drag-cancel), `input` (`data-oninput`),
/// `contextmenu` (`data-oncontextmenu`), and capture-phase `scroll`
/// (`data-onscroll`). Listeners are leaked via `Closure::forget` and live for
/// the lifetime of the page.
pub fn setup_event_delegation(doc: &WebDocument) {
    let browser_doc = doc.browser_document().clone();
    let browser_doc_for_click = browser_doc.clone();

    // Mousedown delegation: find nearest [data-rid] ancestor and dispatch.
    // We use mousedown instead of click so that drag operations (sliders, etc.)
    // can begin tracking mouse movement immediately.
    let mousedown_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            // Clear render surface focus if click is outside any surface
            if rinch::render_surface::focused_surface_id().is_some()
                && el.closest("[data-render-surface]").ok().flatten().is_none()
            {
                rinch::render_surface::set_focused_surface(None);
            }

            // Additive: fire data-onmousedown before the click dispatch below,
            // matching DOM order (mousedown precedes click) and the desktop arm.
            dispatch_mouse_attr(&el, "data-onmousedown", &event);

            // Element drag-and-drop: a mousedown on a `draggable="true"` element
            // starts a pending drag. Its click is deferred to mouseup (and fires
            // only if the drag never activates), matching the desktop backend.
            let drag_source = el.closest("[draggable=\"true\"]").ok().flatten();
            if let Some(source) = &drag_source {
                WEB_DRAG.with(|d| {
                    *d.borrow_mut() = Some(WebDragState {
                        source: source.clone(),
                        start_x: event.client_x() as f32,
                        start_y: event.client_y() as f32,
                        cursor: (event.client_x() as f32, event.client_y() as f32),
                        active: false,
                        over_target: None,
                    });
                });
            }

            // Click dispatch (data-rid fires on mousedown). The primary (left)
            // button always clicks; a non-primary button clicks only when there
            // is no contextmenu handler in the ancestry — mirroring the desktop
            // right-click gate where oncontextmenu suppresses the click. Draggable
            // sources defer their click to mouseup (above).
            let click_allowed = drag_source.is_none()
                && (event.button() == 0
                    || el.closest("[data-oncontextmenu]").ok().flatten().is_none());

            // Dispatch the click for the nearest [data-rid] (draggables defer to
            // mouseup above).
            if click_allowed {
                dispatch_click_at(&el, &browser_doc_for_click, &event);
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("mousedown", mousedown_closure.as_ref().unchecked_ref())
        .unwrap();
    mousedown_closure.forget();

    // Mousemove delegation: feed active drag operations.
    let mousemove_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        let (drag_active, _) =
            rinch_core::update_drag(event.client_x() as f32, event.client_y() as f32);
        if drag_active {
            event.prevent_default();
        }

        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            dispatch_mouse_attr(&el, "data-onmousemove", &event);

            // Element drag-and-drop: if a drag is pending/active, advance it and
            // skip hover — an active drag owns the move (mirrors desktop).
            if WEB_DRAG.with(|d| d.borrow().is_some()) {
                handle_web_drag_move(&event);
                return;
            }

            // Hover: fire data-onleave on the old element and data-onenter on the
            // new one whenever the resolved hover element changes.
            let new_hover = el.closest("[data-onenter],[data-onleave]").ok().flatten();
            LAST_HOVER.with(|lh| {
                let changed = {
                    let cur = lh.borrow();
                    match (cur.as_ref(), new_hover.as_ref()) {
                        (Some(old), Some(new)) => !same_element(old, new),
                        (None, None) => false,
                        _ => true,
                    }
                };
                if !changed {
                    return;
                }
                // Drop all borrows before dispatching (handlers may re-enter).
                let old = lh.borrow().clone();
                if let Some(old_el) = old
                    && let Some(id_str) = old_el.get_attribute("data-onleave")
                    && let Ok(id) = id_str.parse::<usize>()
                {
                    events::dispatch_event(events::EventHandlerId(id));
                }
                if let Some(new_el) = new_hover.as_ref()
                    && let Some(id_str) = new_el.get_attribute("data-onenter")
                    && let Ok(id) = id_str.parse::<usize>()
                {
                    events::dispatch_event(events::EventHandlerId(id));
                }
                *lh.borrow_mut() = new_hover;
            });
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("mousemove", mousemove_closure.as_ref().unchecked_ref())
        .unwrap();
    mousemove_closure.forget();

    // Mouseup delegation: stop active drag operations and fire data-onmouseup.
    let browser_doc_for_up = browser_doc.clone();
    let mouseup_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        rinch_core::Drag::cancel();
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            dispatch_mouse_attr(&el, "data-onmouseup", &event);
        }

        // Element drag-and-drop: finish drop/dragend, or fire the click that was
        // deferred when this draggable's mousedown started a pending drag.
        let dnd = WEB_DRAG.with(|d| d.borrow_mut().take());
        if let Some(state) = dnd {
            if state.active {
                let x = event.client_x() as f32;
                let y = event.client_y() as f32;
                if let Some(t) = &state.over_target {
                    dispatch_plain_attr(t, "data-ondrop");
                    dispatch_plain_attr(t, "data-ondragleave");
                }
                dispatch_drag_cursor_only(&state.source, "data-ondragend", x, y);
            } else {
                // Threshold never crossed — treat as a click on the draggable,
                // with the same full click semantics as the mousedown path.
                dispatch_click_at(&state.source, &browser_doc_for_up, &event);
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("mouseup", mouseup_closure.as_ref().unchecked_ref())
        .unwrap();
    mouseup_closure.forget();

    // Keyboard delegation: route to focused render surface or keyboard interceptor.
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        // Escape cancels an in-progress element drag (consumed only if one was
        // actually active, so Escape otherwise reaches the app normally).
        if event.key() == "Escape" && cancel_web_drag() {
            event.prevent_default();
            return;
        }

        // If a render surface is focused, route keyboard events to it
        if let Some(surface_id) = rinch::render_surface::focused_surface_id() {
            let key_data = rinch::render_surface::SurfaceKeyData {
                key: event.key(),
                code: event.code(),
                ctrl: event.ctrl_key() || event.meta_key(),
                shift: event.shift_key(),
                alt: event.alt_key(),
                meta: event.meta_key(),
            };
            rinch::render_surface::dispatch_surface_event(
                surface_id,
                rinch::render_surface::SurfaceEvent::KeyDown(key_data),
            );
            // Also dispatch TextInput for printable characters
            let key = event.key();
            if key.len() == 1 && !event.ctrl_key() && !event.meta_key() && !event.alt_key() {
                rinch::render_surface::dispatch_surface_event(
                    surface_id,
                    rinch::render_surface::SurfaceEvent::TextInput(key),
                );
            }
            event.prevent_default();
            event.stop_propagation();
            return;
        }

        let key_data = events::KeyEventData {
            key: event.key(),
            code: event.code(),
            ctrl: event.ctrl_key() || event.meta_key(),
            shift: event.shift_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        };
        if events::dispatch_keyboard_event(&key_data) {
            event.prevent_default();
            event.stop_propagation();
        } else if event.key() == "Enter" {
            // Check if the target element (or ancestor) has data-onsubmit
            if let Some(target) = event.target()
                && let Ok(el) = target.dyn_into::<web_sys::Element>()
            {
                let mut current: Option<web_sys::Element> = Some(el);
                while let Some(el) = current {
                    if let Some(handler_str) = el.get_attribute("data-onsubmit")
                        && let Ok(handler_id) = handler_str.parse::<usize>()
                    {
                        events::dispatch_event(events::EventHandlerId(handler_id));
                        break;
                    }
                    current = el.parent_element();
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("keydown", keydown_closure.as_ref().unchecked_ref())
        .unwrap();
    keydown_closure.forget();

    // Input delegation: find [data-oninput] on the target or ancestors.
    let browser_doc2 = browser_doc.clone();
    let input_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(target) = event.target() {
            // Try HtmlInputElement first
            let value = if let Ok(input) = target.clone().dyn_into::<web_sys::HtmlInputElement>() {
                Some(input.value())
            } else if let Ok(textarea) = target.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
                Some(textarea.value())
            } else {
                None
            };

            if let Some(value) = value {
                // Walk up from target to find [data-oninput]
                if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                    let mut current: Option<web_sys::Element> = Some(el);
                    while let Some(el) = current {
                        if let Some(handler_str) = el.get_attribute("data-oninput")
                            && let Ok(handler_id) = handler_str.parse::<usize>()
                        {
                            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
                            break;
                        }
                        current = el.parent_element();
                    }
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc2
        .add_event_listener_with_callback("input", input_closure.as_ref().unchecked_ref())
        .unwrap();
    input_closure.forget();

    // Contextmenu delegation: dispatch data-oncontextmenu and suppress the
    // native browser menu when a handler is found.
    let contextmenu_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
            && let Ok(Some(menu_el)) = el.closest("[data-oncontextmenu]")
            && let Some(id_str) = menu_el.get_attribute("data-oncontextmenu")
            && let Ok(id) = id_str.parse::<usize>()
        {
            let rect = menu_el.get_bounding_client_rect();
            events::set_click_context(events::ClickContext {
                mouse_x: event.client_x() as f32,
                mouse_y: event.client_y() as f32,
                element_x: rect.x() as f32,
                element_y: rect.y() as f32,
                element_width: rect.width() as f32,
                element_height: rect.height() as f32,
                text_hit: Default::default(),
                viewport_width: 0.0,
                viewport_height: 0.0,
                button: events::MouseButton::Right,
                modifiers: modifiers_from_event(&event),
            });
            event.prevent_default();
            events::dispatch_event(events::EventHandlerId(id));
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback(
            "contextmenu",
            contextmenu_closure.as_ref().unchecked_ref(),
        )
        .unwrap();
    contextmenu_closure.forget();

    // Scroll delegation: the native `scroll` event fires on the scrolled element
    // and does NOT bubble, so register in the capture phase to catch all
    // descendants with a single document-level listener.
    let scroll_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
            && let Some(id_str) = el.get_attribute("data-onscroll")
            && let Ok(id) = id_str.parse::<usize>()
        {
            events::dispatch_scroll_event(events::EventHandlerId(id), el.scroll_top() as f64);
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback_and_bool(
            "scroll",
            scroll_closure.as_ref().unchecked_ref(),
            true, // capture phase
        )
        .unwrap();
    scroll_closure.forget();
}
