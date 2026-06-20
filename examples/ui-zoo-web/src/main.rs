//! UI Zoo WASM entry point with browser-native DOM rendering.
//!
//! The DOM backend (`WebDocument`), event delegation, and mount sequence live in
//! the `rinch-web` crate. This file only declares the app's component tree, its
//! global CSS, and the `#[wasm_bindgen(start)]` entry point that mounts it.

use std::rc::Rc;

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use ui_zoo::{
    init_all_sections, nav_links, overlays_demo_overlays, section_content, theme_controls,
};

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
    let current_section = Signal::new(0_usize);
    let primary_color = Signal::new("blue");
    let dark_mode = Signal::new(false);

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
                        Text { size: "xs", color: "dimmed", "Rinch Component Showcase" }
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

// -- Entry point --------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        font_family: Some(
            "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif".into(),
        ),
        dark_mode: false,
        ..Default::default()
    };

    rinch_web::mount(theme, app);

    log::info!("Rinch app mounted with browser-native DOM rendering");
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
    // This empty main is required for the binary crate.
}
