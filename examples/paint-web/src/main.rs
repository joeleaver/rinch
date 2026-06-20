//! Paint demo WASM entry point with browser-native DOM rendering.
//!
//! The DOM backend (`WebDocument`), event delegation, and mount sequence live in
//! the `rinch-web` crate. This file only declares the app wrapper, its global
//! CSS, and the `#[wasm_bindgen(start)]` entry point that mounts it.

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

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
    let app_node = paint::app(__scope);

    rsx! {
        div {
            style { {CSS_WEB} }
            {app_node}
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

    rinch_web::mount(theme, web_app);

    log::info!("Paint demo mounted with browser-native DOM rendering");
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
