//! Paint demo WASM entry point with browser-native DOM rendering.

pub mod web_document;

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use rinch::prelude::*;
use rinch_core::dom::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;

/// Global CSS for the paint app.
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
"#;

/// The web app wrapper — injects theme and global styles.
fn web_app(__scope: &mut RenderScope) -> NodeHandle {
    let style_node = __scope.create_element("style");
    let style_text = __scope.create_text(CSS_WEB);
    style_node.append_child(&style_text);

    let app_node = paint::app(__scope);

    let wrapper = __scope.create_element("div");
    wrapper.append_child(&style_node);
    wrapper.append_child(&app_node);
    wrapper
}

// -- Event delegation ---------------------------------------------------------

/// Set up global event listeners that delegate to rinch's event handler registry.
fn setup_event_delegation(doc: &web_document::WebDocument) {
    let browser_doc = doc.browser_document().clone();

    // Click delegation: find nearest [data-rid] ancestor and dispatch.
    let click_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>() {
                // Clear render surface focus if click is outside any surface
                if rinch::render_surface::focused_surface_id().is_some() {
                    if el.closest("[data-render-surface]").ok().flatten().is_none() {
                        rinch::render_surface::set_focused_surface(None);
                    }
                }

                // Walk up from target to find nearest [data-rid]
                if let Ok(Some(rid_el)) = el.closest("[data-rid]")
                    && let Some(rid_str) = rid_el.get_attribute("data-rid")
                        && let Ok(rid) = rid_str.parse::<usize>() {
                            let rect = rid_el.get_bounding_client_rect();

                            events::set_click_context(events::ClickContext {
                                mouse_x: event.client_x() as f32,
                                mouse_y: event.client_y() as f32,
                                element_x: rect.x() as f32,
                                element_y: rect.y() as f32,
                                element_width: rect.width() as f32,
                                element_height: rect.height() as f32,
                                text_hit: Default::default(),
                            });

                            event.prevent_default();
                            events::dispatch_event(events::EventHandlerId(rid));
                        }
            }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("click", click_closure.as_ref().unchecked_ref())
        .unwrap();
    click_closure.forget();

    // Keyboard delegation: route to focused render surface.
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
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
            let value = if let Ok(input) = target.clone().dyn_into::<web_sys::HtmlInputElement>() {
                Some(input.value())
            } else if let Ok(textarea) = target.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
                Some(textarea.value())
            } else {
                None
            };

            if let Some(value) = value {
                if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                    let mut current: Option<web_sys::Element> = Some(el);
                    while let Some(el) = current {
                        if let Some(handler_str) = el.get_attribute("data-oninput")
                            && let Ok(handler_id) = handler_str.parse::<usize>() {
                                events::dispatch_input_event(
                                    events::EventHandlerId(handler_id),
                                    value,
                                );
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
}

// -- Entry point --------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();

    // Clear stale state from any previous run
    events::clear_handlers();
    rinch_core::clear_context();

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

    let root = {
        let mut scope_ref = scope.borrow_mut();
        web_app(&mut scope_ref)
    };

    web_doc
        .borrow_mut()
        .append_child(body_id, root.node_id());

    clear_render_scope();

    // Inject theme CSS as a <style> element
    if let Some(css) = rinch_core::get_current_theme_css() {
        web_doc.borrow().inject_style(&css);
    }

    // Set up event delegation on the browser document
    setup_event_delegation(&web_doc.borrow());

    // Theme CSS update on signal change
    let doc_for_signal = web_doc.clone();
    rinch_core::set_on_signal_change(move || {
        if let Some(css) = rinch_core::get_current_theme_css() {
            doc_for_signal.borrow().update_theme_style(&css);
        }
    });

    // Keep scope and doc alive for the lifetime of the app.
    std::mem::forget(scope);
    std::mem::forget(web_doc);

    log::info!("Paint demo mounted with browser-native DOM rendering");
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
