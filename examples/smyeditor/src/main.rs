//! UI Zoo - Rinch Component Library Showcase
//!
//! A comprehensive demonstration of all rinch widgets organized into sections.
//! Uses signal-based navigation to switch between different widget categories.

mod sections;

use rinch::prelude::*;
use rinch::{run_rinch_with_window_props, WindowProps};
use rinch_tabler_icons::{render_tabler_icon, TablerIcon, TablerIconStyle};
use sections::*;
use std::rc::Rc;

fn app(__scope: &mut RenderScope) -> NodeHandle {
    let current_section = use_signal(|| 0_usize);
    let primary_color = use_signal(|| "blue");
    let dark_mode = use_signal(|| false);
    let drawer_opened = use_signal(|| false);

    // Initialize section state contexts synchronously.
    // These must be called BEFORE overlays_demo_overlays() so context is available.
    // In fine-grained architecture, app() only runs once, so this is safe.
    init_inputs_state();
    init_overlays_state();
    init_navigation_state();
    init_buttons_state();
    init_feedback_state();
    init_icons_state();
    init_tree_state();
    init_editor_state();

    // Nav helper (also closes drawer)
    #[allow(unused_variables)]
    let nav = |idx: usize| {
        move || {
            current_section.set(idx);
            drawer_opened.set(false);
        }
    };

    // Color picker helper
    #[allow(unused_variables)]
    let set_color = |color: &'static str| {
        move || primary_color.set(color)
    };

    // Dark mode toggle
    #[allow(unused_variables)]
    let toggle_dark = move || dark_mode.update(|v| *v = !*v);

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

    // Note: Don't create nested html/head/body - the runtime already creates those.
    // Widget CSS is injected via theme system. App CSS goes in a style element in body.
    rsx! {
        ThemeProvider {
            primary_color_fn: Rc::new(move || primary_color.get()),
            dark_mode_fn: Rc::new(move || dark_mode.get()),

            style { {CSS} }

            BorderlessWindow {
                title: "UI Zoo",
                radius: "md",
                left_section: Some(left_section),
                on_minimize: || minimize_current_window(),
                on_maximize: || toggle_maximize_current_window(),
                on_close: || close_current_window(),

                // Main content area - uses reactive Show for section switching
                div { class: "main-content",
                Show { when: move || current_section.get() == 0, then: |__scope| overview_section(__scope) }
                Show { when: move || current_section.get() == 1, then: |__scope| buttons_section(__scope) }
                Show { when: move || current_section.get() == 2, then: |__scope| inputs_section(__scope) }
                Show { when: move || current_section.get() == 3, then: |__scope| typography_section(__scope) }
                Show { when: move || current_section.get() == 4, then: |__scope| layout_section(__scope) }
                Show { when: move || current_section.get() == 5, then: |__scope| navigation_section(__scope) }
                Show { when: move || current_section.get() == 6, then: |__scope| data_display_section(__scope) }
                Show { when: move || current_section.get() == 7, then: |__scope| feedback_section(__scope) }
                Show { when: move || current_section.get() == 8, then: |__scope| overlays_section(__scope) }
                Show { when: move || current_section.get() == 9, then: |__scope| icons_section(__scope) }
                Show { when: move || current_section.get() == 10, then: |__scope| tree_section(__scope) }
                Show { when: move || current_section.get() == 11, then: |__scope| editor_section(__scope) }
            }
            }

            // Navigation Drawer (inside BorderlessWindow but fixed positioning)
            Drawer {
                opened_fn: Some(Rc::new(move || drawer_opened.get())),
                onclose: move || drawer_opened.set(false),
                position: "left",
                size: "xs",
                title: "UI Zoo",
                with_overlay: true,

                // Navigation links - use active_fn for fine-grained reactive updates
                Stack { gap: "0",
                    NavLink {
                        label: Some("Overview".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 0)),
                        onclick: nav(0)
                    }
                    NavLink {
                        label: Some("Buttons".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 1)),
                        onclick: nav(1)
                    }
                    NavLink {
                        label: Some("Inputs".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 2)),
                        onclick: nav(2)
                    }
                    NavLink {
                        label: Some("Typography".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 3)),
                        onclick: nav(3)
                    }
                    NavLink {
                        label: Some("Layout".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 4)),
                        onclick: nav(4)
                    }
                    NavLink {
                        label: Some("Navigation".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 5)),
                        onclick: nav(5)
                    }
                    NavLink {
                        label: Some("Data Display".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 6)),
                        onclick: nav(6)
                    }
                    NavLink {
                        label: Some("Feedback".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 7)),
                        onclick: nav(7)
                    }
                    NavLink {
                        label: Some("Overlays".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 8)),
                        onclick: nav(8)
                    }
                    NavLink {
                        label: Some("Icons".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 9)),
                        onclick: nav(9)
                    }
                    NavLink {
                        label: Some("Tree".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 10)),
                        onclick: nav(10)
                    }
                    NavLink {
                        label: Some("Rich Text Editor".to_string()),
                        active_fn: Some(Rc::new(move || current_section.get() == 11)),
                        onclick: nav(11)
                    }
                }

                Space { h: "xl" }
                Divider { label: "Theme" }
                Space { h: "md" }

                // Theme controls
                Switch {
                    label: Some("Dark Mode".to_string()),
                    checked_fn: Some(Rc::new(move || dark_mode.get())),
                    onchange: toggle_dark
                }

                Space { h: "md" }
                Text { size: "sm", "Primary Color" }
                Space { h: "xs" }
                Group { gap: "xs",
                    ActionIcon { variant: "filled", color: "blue", onclick: set_color("blue") }
                    ActionIcon { variant: "filled", color: "cyan", onclick: set_color("cyan") }
                    ActionIcon { variant: "filled", color: "teal", onclick: set_color("teal") }
                    ActionIcon { variant: "filled", color: "green", onclick: set_color("green") }
                    ActionIcon { variant: "filled", color: "orange", onclick: set_color("orange") }
                }
                Space { h: "xs" }
                Group { gap: "xs",
                    ActionIcon { variant: "filled", color: "red", onclick: set_color("red") }
                    ActionIcon { variant: "filled", color: "pink", onclick: set_color("pink") }
                    ActionIcon { variant: "filled", color: "grape", onclick: set_color("grape") }
                    ActionIcon { variant: "filled", color: "violet", onclick: set_color("violet") }
                    ActionIcon { variant: "filled", color: "indigo", onclick: set_color("indigo") }
                }
            }

            // Overlays section demo components (rendered at body level for proper fixed positioning)
            {overlays_demo_overlays(__scope)}
        }
    }
}

/// Renders the overlay components (Modal, Drawer, Notification) for the Overlays section demo.
/// These are rendered at the body level (outside .main-content) for proper fixed positioning.
fn overlays_demo_overlays(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<OverlaysSectionState>();

    let (
        modal_opened,
        modal_lg_opened,
        drawer_opened,
        drawer_right_opened,
        notification_visible,
    ) = match state {
        Some(s) => (
            s.modal_opened,
            s.modal_lg_opened,
            s.drawer_opened,
            s.drawer_right_opened,
            s.notification_visible,
        ),
        None => {
            return rsx! { div { } };
        }
    };

    rsx! {
        Fragment {
            // Basic Modal - use opened_fn for fine-grained reactive updates
            Modal {
                opened_fn: Some(Rc::new(move || modal_opened.get())),
                onclose: move || modal_opened.set(false),
                title: "Welcome!",

                Text { size: "sm", color: "dimmed",
                    "This is a working modal dialog. Click outside or the X button to close it."
                }
                Space { h: "lg" }
                Group { justify: "flex-end", gap: "sm",
                    Button { variant: "subtle", onclick: move || modal_opened.set(false), "Cancel" }
                    Button { onclick: move || modal_opened.set(false), "Confirm" }
                }
            }

            // Large Modal - use opened_fn for fine-grained reactive updates
            Modal {
                opened_fn: Some(Rc::new(move || modal_lg_opened.get())),
                onclose: move || modal_lg_opened.set(false),
                title: "Large Modal",
                size: "lg",

                Stack { gap: "md",
                    Text { size: "sm", color: "dimmed",
                        "This modal uses size=\"lg\" for a wider dialog. Large modals are useful for forms with many fields or displaying detailed content."
                    }
                    Alert { color: "blue", title: "Tip",
                        "You can also use 'xl' or 'full' for even larger modals."
                    }
                }
                Space { h: "lg" }
                Button { full_width: true, onclick: move || modal_lg_opened.set(false), "Got it!" }
            }

            // Left Drawer
            Drawer {
                opened_fn: Some(Rc::new(move || drawer_opened.get())),
                onclose: move || drawer_opened.set(false),
                title: "Navigation",
                position: "left",

                Stack { gap: "0",
                    NavLink { label: Some("Home".to_string()), active: true }
                    NavLink { label: Some("Dashboard".to_string()) }
                    NavLink { label: Some("Settings".to_string()) }
                    NavLink { label: Some("Profile".to_string()) }
                }
            }

            // Right Drawer
            Drawer {
                opened_fn: Some(Rc::new(move || drawer_right_opened.get())),
                onclose: move || drawer_right_opened.set(false),
                title: "Details Panel",
                position: "right",

                Stack { gap: "md",
                    Text { size: "sm", color: "dimmed",
                        "This drawer slides in from the right. Use right drawers for detail panels, filters, or secondary navigation."
                    }
                    Alert { color: "blue",
                        "Drawers also support 'top' and 'bottom' positions."
                    }
                }
            }

            // Notification - use opened_fn for fine-grained reactive updates
            Notification {
                opened_fn: Some(Rc::new(move || notification_visible.get())),
                onclose: move || notification_visible.set(false),
                title: "Success!",
                color: "green",
                with_close_button: true,
                "Your changes have been saved successfully."
            }
        }
    }
}

/// Global CSS for the app layout.
/// BorderlessWindow widget handles titlebar, window controls, and layout.
/// We only need styles for the content area.
const CSS: &str = r#"
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

fn main() {
    let window_props = WindowProps {
        title: "UI Zoo - Rinch Component Library".into(),
        width: 1200,
        height: 800,
        // NOTE: borderless/transparent not supported in rinch-dom yet, disable for now
        borderless: false,
        transparent: false,
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

    // Use rinch-dom runtime instead of blitz
    run_rinch_with_window_props(app, window_props);
}
