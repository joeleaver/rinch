//! Browser-native DOM backend for rinch (WASM target).
//!
//! Instead of painting to a Canvas 2D or WebGPU surface, this crate creates real
//! browser DOM elements via `web_sys`. The browser handles layout, CSS, text
//! rendering, and painting natively. The reactive system (Signal/Effect) and all
//! components work through [`rinch_core::dom::NodeHandle`] →
//! [`rinch_core::dom::DomDocument`], so everything works automatically.
//!
//! It provides three pieces:
//! - [`WebDocument`] — a [`DomDocument`](rinch_core::dom::DomDocument) implementation over `web_sys`.
//! - [`setup_event_delegation`] — document-level listeners that bridge browser
//!   events into rinch's event/drag/render-surface systems.
//! - [`mount`] — a one-shot boot helper that wires the above together.
//!
//! A typical WASM entry point keeps only its app-specific component tree and theme:
//!
//! ```ignore
//! use wasm_bindgen::prelude::*;
//! use rinch::prelude::*;
//! use rinch_core::element::ThemeProviderProps;
//!
//! #[component]
//! fn app() -> NodeHandle { rsx! { div { "Hello, web!" } } }
//!
//! #[wasm_bindgen(start)]
//! pub fn start() {
//!     let theme = ThemeProviderProps { dark_mode: false, ..Default::default() };
//!     rinch_web::mount(theme, app);
//! }
//! ```

mod event_delegation;
pub mod web_document;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;

pub use event_delegation::setup_event_delegation;
pub use web_document::WebDocument;

/// Mount a rinch component tree into the browser DOM and start the app.
///
/// This performs the full boot sequence:
/// 1. clears stale handlers/context from any previous run,
/// 2. installs theme CSS (via [`rinch::setup_theme_css`]),
/// 3. creates a [`WebDocument`] backed by the real browser document,
/// 4. builds the component tree by calling `build` with the render scope,
/// 5. mounts it under `#rinch-body`, injects the theme `<style>`,
/// 6. wires global [event delegation](setup_event_delegation), and
/// 7. re-injects theme CSS on signal change (e.g. dark-mode toggle).
///
/// The render scope and document are leaked (`mem::forget`) so the app stays
/// alive for the lifetime of the page — effects reference the scope and
/// `NodeHandle`s reference the document.
///
/// `build` receives the active [`RenderScope`]; a `#[component]` function (which
/// expands to `fn(&mut RenderScope) -> NodeHandle`) can be passed directly.
pub fn mount<F>(theme: ThemeProviderProps, build: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    // Clear stale state from any previous run.
    events::clear_handlers();
    rinch_core::clear_context();

    // Set up theme CSS before mounting.
    rinch::setup_theme_css(&theme);

    // Create WebDocument backed by the real browser DOM.
    let browser_doc = web_sys::window().unwrap().document().unwrap();
    let web_doc = Rc::new(RefCell::new(WebDocument::new(browser_doc)));

    // Mount the component tree.
    let doc_as_dom: Rc<RefCell<dyn DomDocument>> = web_doc.clone();
    let body_id = web_doc.borrow().body();
    let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

    set_render_scope(scope.clone());

    let root = {
        let mut scope_ref = scope.borrow_mut();
        build(&mut scope_ref)
    };

    web_doc.borrow_mut().append_child(body_id, root.node_id());

    clear_render_scope();

    // Inject theme CSS as a <style> element.
    if let Some(css) = rinch_core::get_current_theme_css() {
        web_doc.borrow().inject_style(&css);
    }

    // Set up event delegation on the browser document.
    setup_event_delegation(&web_doc.borrow());

    // When signals change, effects run synchronously and update the browser DOM
    // directly; the browser repaints automatically. We just need to re-inject
    // theme CSS if it changed (e.g. dark-mode toggle).
    let doc_for_signal = web_doc.clone();
    rinch_core::set_on_signal_change(move || {
        if let Some(css) = rinch_core::get_current_theme_css() {
            doc_for_signal.borrow().update_theme_style(&css);
        }
    });

    // Keep scope and doc alive for the lifetime of the app.
    std::mem::forget(scope);
    std::mem::forget(web_doc);
}
