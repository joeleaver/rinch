//! UI Zoo desktop entry point.
//!
//! Desktop-specific shell: BorderlessWindow with transparent background,
//! custom titlebar with menu button, and drawer-based navigation.
//! All section content comes from the shared `ui_zoo` library.

use std::rc::Rc;

use rinch::prelude::*;
use rinch::{WindowProps, run_rinch_with_window_props};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use ui_zoo::{
    init_all_sections, nav_links, overlays_demo_overlays, section_content, theme_controls,
};

/// Global CSS for the desktop app layout.
const CSS_DESKTOP: &str = r#"
* {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

html, body {
    height: 100vh;
}

body {
    font-family: var(--rinch-font-family);
    background: transparent; /* For transparent window */
    color: var(--rinch-color-text);
    overflow: hidden;
}

.main-content {
    padding: var(--rinch-spacing-xl);
}
"#;

#[component]
fn app() -> NodeHandle {
    let current_section = use_signal(|| 0_usize);
    let primary_color = use_signal(|| "blue");
    let dark_mode = use_signal(|| false);
    let drawer_opened = use_signal(|| false);

    init_all_sections();

    // Create left section renderer for menu button
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon {
                variant: "subtle",
                size: "lg",
                onclick: move || drawer_opened.update(|v| *v = !*v),
                {render_tabler_icon(__scope, TablerIcon::Menu2, TablerIconStyle::Outline)}
            }
        }
    });

    let nav = move |idx: usize| {
        move || {
            current_section.set(idx);
            drawer_opened.set(false);
        }
    };

    rsx! {
        ThemeProvider {
            primary_color_fn: Rc::new(move || primary_color.get()),
            dark_mode_fn: Rc::new(move || dark_mode.get()),

            style { {CSS_DESKTOP} }

            BorderlessWindow {
                title: "UI Zoo",
                radius: "md",
                left_section: Some(left_section),
                on_minimize: || minimize_current_window(),
                on_maximize: || toggle_maximize_current_window(),
                on_close: || close_current_window(),

                // Main content area - uses reactive Show for section switching
                div { class: "main-content",
                    {section_content(__scope, current_section)}
                }
            }

            // Navigation Drawer
            Drawer {
                opened_fn: Some(Rc::new(move || drawer_opened.get())),
                onclose: move || drawer_opened.set(false),
                position: "left",
                size: "xs",
                title: "UI Zoo",
                with_overlay: true,

                {nav_links(__scope, current_section, nav)}

                Space { h: "xl" }
                {theme_controls(__scope, primary_color, dark_mode)}
            }

            // Overlays section demo components (rendered at body level for proper fixed positioning)
            {overlays_demo_overlays(__scope)}
        }
    }
}

fn main() {
    let window_props = WindowProps {
        title: "UI Zoo - Rinch Component Library".into(),
        width: 1200,
        height: 800,
        borderless: true,
        transparent: true,
        ..Default::default()
    };

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    // Set up theme CSS (loads into thread-local, picked up by rinch-dom runtime)
    rinch::setup_theme_css(&theme);

    run_rinch_with_window_props(app, window_props);
}
