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
fn app(__scope: &mut RenderScope) -> NodeHandle {
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

/// Set up global event listeners that delegate to rinch's event handler registry.
fn setup_event_delegation(doc: &web_document::WebDocument) {
    let browser_doc = doc.browser_document().clone();

    // Click delegation: find nearest [data-rid] ancestor and dispatch.
    let click_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                // Walk up from target to find nearest [data-rid]
                if let Ok(Some(rid_el)) = el.closest("[data-rid]") {
                    if let Some(rid_str) = rid_el.get_attribute("data-rid") {
                        if let Ok(rid) = rid_str.parse::<usize>() {
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
