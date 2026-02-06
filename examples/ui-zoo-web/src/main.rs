//! UI Zoo WASM entry point with browser-native DOM rendering.
//!
//! Instead of painting to a Canvas 2D or WebGPU surface, this runtime creates
//! real browser DOM elements via `web_sys`. The browser handles layout, CSS,
//! text rendering, and painting natively. The reactive system (Signal/Effect)
//! and all widgets work through `NodeHandle` -> `DomDocument`, so everything
//! works automatically.

pub mod web_document;

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use rinch::prelude::*;
use rinch_core::dom::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_core::hooks::{begin_render, clear_hooks, end_render};
use ui_zoo::{init_all_sections, nav_links, overlays_demo_overlays, section_content, theme_controls};

/// Global CSS for the web app layout with sidebar.
const CSS_WEB: &str = r#"
* {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

html, body {
    height: 100vh;
    font-family: var(--rinch-font-family);
    background: var(--rinch-color-body);
    color: var(--rinch-color-text);
    overflow: hidden;
}

.app-shell {
    display: flex;
    height: 100vh;
}

.sidebar {
    width: 220px;
    min-width: 220px;
    padding: var(--rinch-spacing-md);
    border-right: 1px solid var(--rinch-color-gray-3);
    background: var(--rinch-color-body);
    overflow-y: auto;
}

.sidebar-header {
    padding: var(--rinch-spacing-sm) var(--rinch-spacing-xs);
}

.main-content {
    flex: 1;
    padding: var(--rinch-spacing-xl);
    overflow-y: auto;
}
"#;

/// The web app shell: sidebar navigation layout.
#[component]
fn app() -> NodeHandle {
    let current_section = use_signal(|| 0_usize);
    let primary_color = use_signal(|| "blue");
    let dark_mode = use_signal(|| false);

    init_all_sections();

    let nav = move |idx: usize| {
        move || {
            current_section.set(idx);
        }
    };

    rsx! {
        ThemeProvider {
            primary_color_fn: Rc::new(move || primary_color.get()),
            dark_mode_fn: Rc::new(move || dark_mode.get()),

            style { {CSS_WEB} }

            div { class: "app-shell",
                // Sidebar with navigation
                div { class: "sidebar",
                    div { class: "sidebar-header",
                        Title { order: 3, "UI Zoo" }
                        Text { size: "xs", color: "dimmed", "Rinch Widget Showcase" }
                    }
                    Space { h: "md" }
                    {nav_links(__scope, current_section, nav)}
                    Space { h: "xl" }
                    {theme_controls(__scope, primary_color, dark_mode)}
                }

                // Main content area
                div { class: "main-content",
                    {section_content(__scope, current_section)}
                }
            }

            // Overlays section demo components
            {overlays_demo_overlays(__scope)}
        }
    }
}

// -- Event delegation ---------------------------------------------------------

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
fn resolve_text_hit(browser_doc: &web_sys::Document, client_x: f32, client_y: f32) -> Option<events::TextHitInfo> {
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
        if let Ok(el) = node.clone().dyn_into::<web_sys::Element>() {
            if el.has_attribute("data-block-index") {
                block_el = Some(el);
                break;
            }
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
        if let Some(child) = children.item(i) {
            if walk_text_nodes_for_offset(&child, target, utf16_offset, byte_offset) {
                return true;
            }
        }
    }
    false
}

/// Set up global event listeners that delegate to rinch's event handler registry.
fn setup_event_delegation(doc: &web_document::WebDocument) {
    let browser_doc = doc.browser_document().clone();
    let browser_doc_for_click = browser_doc.clone();

    // Click delegation: find nearest [data-rid] ancestor and dispatch.
    let click_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                // Walk up from target to find nearest [data-rid]
                if let Ok(Some(rid_el)) = el.closest("[data-rid]") {
                    if let Some(rid_str) = rid_el.get_attribute("data-rid") {
                        if let Ok(rid) = rid_str.parse::<usize>() {
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
                            });

                            // Prevent browser default behavior (e.g. <label>
                            // synthesizing a second click on its <input>, which
                            // would double-toggle the handler).
                            event.prevent_default();
                            events::dispatch_event(events::EventHandlerId(rid));
                        }
                    }
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("click", click_closure.as_ref().unchecked_ref())
        .unwrap();
    click_closure.forget();

    // Keyboard delegation: dispatch all key presses to the keyboard interceptor.
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
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
                        if let Some(handler_str) = el.get_attribute("data-oninput") {
                            if let Ok(handler_id) = handler_str.parse::<usize>() {
                                events::dispatch_input_event(
                                    events::EventHandlerId(handler_id),
                                    value,
                                );
                                break;
                            }
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
}

// -- Entry point --------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();

    // Clear stale state from any previous run
    events::clear_handlers();
    clear_hooks();

    // Set up theme CSS before mounting
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        font_family: Some(
            "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif".into(),
        ),
        dark_mode: false,
        ..Default::default()
    };
    rinch::setup_theme_css(&theme);

    // Create WebDocument backed by the real browser DOM
    let browser_doc = web_sys::window().unwrap().document().unwrap();
    let web_doc = Rc::new(RefCell::new(web_document::WebDocument::new(browser_doc)));

    // Mount the component tree
    let doc_as_dom: Rc<RefCell<dyn DomDocument>> = web_doc.clone();
    let body_id = web_doc.borrow().body();
    let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

    set_render_scope(scope.clone());
    begin_render();

    let root = {
        let mut scope_ref = scope.borrow_mut();
        app(&mut scope_ref)
    };

    web_doc
        .borrow_mut()
        .append_child(body_id, root.node_id());

    end_render();
    clear_render_scope();

    // Inject theme CSS as a <style> element
    if let Some(css) = rinch_core::get_current_theme_css() {
        web_doc.borrow().inject_style(&css);
    }

    // Set up event delegation on the browser document
    setup_event_delegation(&web_doc.borrow());

    // When signals change, effects run synchronously and update the browser DOM
    // directly. The browser repaints automatically. We just need to re-inject
    // theme CSS if it changed (e.g., dark mode toggle).
    let doc_for_signal = web_doc.clone();
    rinch_core::set_on_signal_change(move || {
        if let Some(css) = rinch_core::get_current_theme_css() {
            doc_for_signal.borrow().update_theme_style(&css);
        }
    });

    // Keep scope and doc alive for the lifetime of the app.
    // Effects reference the scope; NodeHandles reference the doc.
    std::mem::forget(scope);
    std::mem::forget(web_doc);

    log::info!("Rinch app mounted with browser-native DOM rendering");
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
    // This empty main is required for the binary crate.
}
