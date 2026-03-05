//! UI Zoo desktop entry point.
//!
//! Desktop-specific shell: BorderlessWindow with transparent background,
//! custom titlebar with menu button, and drawer-based navigation.
//! All section content comes from the shared `ui_zoo` library.

use std::rc::Rc;

use rinch::menu::{Menu, MenuItem};
use rinch::prelude::*;
use rinch::tray::TrayIconBuilder;
use rinch::{WindowProps, run_with_window_props_and_menu};
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

fn build_app(
    current_section: Signal<usize>,
    primary_color: Signal<&'static str>,
    dark_mode: Signal<bool>,
    __scope: &mut RenderScope,
) -> NodeHandle {
    let drawer_opened = Signal::new(false);

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

    // Right section: hide-to-tray button (before window controls)
    let right_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon {
                variant: "subtle",
                size: "sm",
                onclick: hide_current_window,
                {render_tabler_icon(__scope, TablerIcon::ArrowBarDown, TablerIconStyle::Outline)}
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
                right_section: Some(right_section),
                on_minimize: minimize_current_window,
                on_maximize: toggle_maximize_current_window,
                on_close: close_current_window,

                // Main content area - uses reactive Show for section switching
                div { class: "main-content",
                    {section_content(__scope, current_section)}
                }
            }

            // Navigation Drawer
            Drawer {
                opened_fn: move || drawer_opened.get(),
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
    // Create signals in main() so menu callbacks can reference them.
    // Signal is Copy, so these can be captured by both menus and the app closure.
    let current_section = Signal::new(0_usize);
    let primary_color = Signal::new("blue");
    let dark_mode = Signal::new(false);

    // ── System tray ──────────────────────────────────────────────────────
    let tray_menu = Menu::new()
        .item(MenuItem::new("Show Window").on_click(show_current_window))
        .separator()
        .item(MenuItem::new("Quit").on_click(close_current_window));

    let _tray = TrayIconBuilder::new()
        .with_tooltip("UI Zoo")
        .with_icon_png(include_bytes!("../../../rinch-icon.png"))
        .expect("failed to load tray icon")
        .with_menu(tray_menu)
        .build()
        .expect("failed to create tray icon");

    // ── Native menu bar ──────────────────────────────────────────────────

    // Section names match the sidebar navigation
    let sections: &[&str] = &[
        "Overview",
        "Buttons",
        "Inputs",
        "Typography",
        "Layout",
        "Navigation",
        "Data Display",
        "Feedback",
        "Overlays",
        "Icons",
        "Tree",
        "Rich Text Editor",
        "CSS Features",
        "Video",
    ];

    // View > Go To submenu with all sections
    let mut goto_menu = Menu::new();
    for (i, &name) in sections.iter().enumerate() {
        goto_menu = goto_menu.item(MenuItem::new(name).on_click(move || current_section.set(i)));
    }

    let view_menu = Menu::new()
        .submenu("Go To", goto_menu)
        .separator()
        .item(
            MenuItem::new("Previous Section")
                .shortcut("Alt+Left")
                .on_click(move || {
                    current_section.update(|s| {
                        if *s > 0 {
                            *s -= 1
                        }
                    });
                }),
        )
        .item(
            MenuItem::new("Next Section")
                .shortcut("Alt+Right")
                .on_click(move || {
                    current_section.update(|s| {
                        if *s < 15 {
                            *s += 1
                        }
                    });
                }),
        )
        .separator()
        .item(
            MenuItem::new("Toggle Dark Mode")
                .shortcut("Ctrl+D")
                .on_click(move || {
                    dark_mode.update(|v| *v = !*v);
                }),
        );

    // Theme > color submenu
    let colors: &[&str] = &[
        "blue", "cyan", "teal", "green", "orange", "red", "pink", "grape", "violet", "indigo",
    ];
    let mut theme_menu = Menu::new();
    for &color in colors {
        // Capitalize first letter for the menu label
        let label = {
            let mut c = color.chars();
            match c.next() {
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        };
        let color_static: &'static str = color;
        theme_menu =
            theme_menu.item(MenuItem::new(label).on_click(move || primary_color.set(color_static)));
    }

    let menus = vec![("View", view_menu), ("Theme", theme_menu)];

    // ── Window configuration ─────────────────────────────────────────────

    let window_props = WindowProps {
        title: "UI Zoo - Rinch Component Library".into(),
        width: 1200,
        height: 800,
        borderless: true,
        transparent: true,
        menu_in_titlebar: true,
        icon: Some(include_bytes!("../../../rinch-icon.png")),
        app_id: Some("ui-zoo".into()),
        on_close_requested: Some(std::sync::Arc::new(|| {
            // Hide to tray instead of quitting
            hide_current_window();
            false // Don't exit
        })),
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

    run_with_window_props_and_menu(
        move |scope| build_app(current_section, primary_color, dark_mode, scope),
        window_props,
        Some(theme),
        Some(menus),
    );
}
