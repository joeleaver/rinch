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

/// Set up global event listeners that delegate to rinch's event handler registry.
///
/// Wires `mousedown`/`mousemove`/`mouseup` (click dispatch + drag tracking +
/// text-hit resolution), `keydown` (render-surface routing, keyboard
/// interceptor, and `data-onsubmit` on Enter), and `input` (`data-oninput`
/// dispatch). Listeners are leaked via `Closure::forget` and live for the
/// lifetime of the page.
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

            // Click dispatch (data-rid fires on mousedown). The primary (left)
            // button always clicks; a non-primary button clicks only when there
            // is no contextmenu handler in the ancestry — mirroring the desktop
            // right-click gate where oncontextmenu suppresses the click.
            let click_allowed =
                event.button() == 0 || el.closest("[data-oncontextmenu]").ok().flatten().is_none();

            // Walk up from target to find nearest [data-rid]
            if click_allowed
                && let Ok(Some(rid_el)) = el.closest("[data-rid]")
                && let Some(rid_str) = rid_el.get_attribute("data-rid")
                && let Ok(rid) = rid_str.parse::<usize>()
            {
                // Set click context with mouse position and element bounds
                let rect = rid_el.get_bounding_client_rect();
                let text_hit = resolve_text_hit(
                    &browser_doc_for_click,
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
                    button: mouse_button_from_event(&event),
                    modifiers: modifiers_from_event(&event),
                });

                // Prevent browser default behavior (e.g. text selection
                // during slider drag, <label> synthesizing extra events).
                event.prevent_default();
                events::dispatch_event(events::EventHandlerId(rid));
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
    let mouseup_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        rinch_core::Drag::cancel();
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            dispatch_mouse_attr(&el, "data-onmouseup", &event);
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("mouseup", mouseup_closure.as_ref().unchecked_ref())
        .unwrap();
    mouseup_closure.forget();

    // Keyboard delegation: route to focused render surface or keyboard interceptor.
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
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
