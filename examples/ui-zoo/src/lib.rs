//! UI Zoo - Rinch Component Library Showcase
//!
//! A platform-agnostic library exporting section components and shared helpers.
//! Each platform binary (desktop, web) provides its own shell using these exports.

pub mod sections;

use rinch::prelude::*;
use sections::*;

/// Initialize all section state contexts.
///
/// Must be called once before rendering any sections (before `overlays_demo_overlays()`
/// so that context is available). In fine-grained architecture, app() only runs once,
/// so this is safe.
pub fn init_all_sections() {
    init_inputs_state();
    init_overlays_state();
    init_navigation_state();
    init_buttons_state();
    init_feedback_state();
    init_icons_state();
    init_tree_state();
    init_editor_state();
}

/// Render the navigation links list.
///
/// Generic over the nav callback so each platform can provide its own
/// navigation behavior (e.g. closing a drawer on desktop, no-op on web).
#[component]
pub fn nav_links<F: Fn() + 'static>(
    current_section: Signal<usize>,
    nav: impl Fn(usize) -> F,
) -> NodeHandle {
    rsx! {
        Stack { gap: "0",
            NavLink {
                label: Some("Overview".to_string()),
                active_fn: move || current_section.get() == 0,
                onclick: nav(0)
            }
            NavLink {
                label: Some("Buttons".to_string()),
                active_fn: move || current_section.get() == 1,
                onclick: nav(1)
            }
            NavLink {
                label: Some("Inputs".to_string()),
                active_fn: move || current_section.get() == 2,
                onclick: nav(2)
            }
            NavLink {
                label: Some("Typography".to_string()),
                active_fn: move || current_section.get() == 3,
                onclick: nav(3)
            }
            NavLink {
                label: Some("Layout".to_string()),
                active_fn: move || current_section.get() == 4,
                onclick: nav(4)
            }
            NavLink {
                label: Some("Navigation".to_string()),
                active_fn: move || current_section.get() == 5,
                onclick: nav(5)
            }
            NavLink {
                label: Some("Data Display".to_string()),
                active_fn: move || current_section.get() == 6,
                onclick: nav(6)
            }
            NavLink {
                label: Some("Feedback".to_string()),
                active_fn: move || current_section.get() == 7,
                onclick: nav(7)
            }
            NavLink {
                label: Some("Overlays".to_string()),
                active_fn: move || current_section.get() == 8,
                onclick: nav(8)
            }
            NavLink {
                label: Some("Icons".to_string()),
                active_fn: move || current_section.get() == 9,
                onclick: nav(9)
            }
            NavLink {
                label: Some("Tree".to_string()),
                active_fn: move || current_section.get() == 10,
                onclick: nav(10)
            }
            NavLink {
                label: Some("Rich Text Editor".to_string()),
                active_fn: move || current_section.get() == 11,
                onclick: nav(11)
            }
            NavLink {
                label: Some("CSS Features".to_string()),
                active_fn: move || current_section.get() == 12,
                onclick: nav(12)
            }
        }
    }
}

/// Render theme controls (dark mode switch, color pickers).
#[component]
pub fn theme_controls(primary_color: Signal<&'static str>, dark_mode: Signal<bool>) -> NodeHandle {
    let set_color = |color: &'static str| move || primary_color.set(color);
    let toggle_dark = move || dark_mode.update(|v| *v = !*v);

    rsx! {
        Fragment {
            Divider { label: "Theme" }
            Space { h: "md" }

            Switch {
                label: Some("Dark Mode".to_string()),
                checked_fn: move || dark_mode.get(),
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
    }
}

/// Render the section content area with Show components for each section.
pub fn section_content(__scope: &mut RenderScope, current_section: Signal<usize>) -> NodeHandle {
    rsx! {
        Fragment {
            Show { when: move || current_section.get() == 0, then: overview_section }
            Show { when: move || current_section.get() == 1, then: buttons_section }
            Show { when: move || current_section.get() == 2, then: inputs_section }
            Show { when: move || current_section.get() == 3, then: typography_section }
            Show { when: move || current_section.get() == 4, then: layout_section }
            Show { when: move || current_section.get() == 5, then: navigation_section }
            Show { when: move || current_section.get() == 6, then: data_display_section }
            Show { when: move || current_section.get() == 7, then: feedback_section }
            Show { when: move || current_section.get() == 8, then: overlays_section }
            Show { when: move || current_section.get() == 9, then: icons_section }
            Show { when: move || current_section.get() == 10, then: tree_section }
            Show { when: move || current_section.get() == 11, then: editor_section }
            Show { when: move || current_section.get() == 12, then: css_features_section }
        }
    }
}

/// Renders the overlay components (Modal, Drawer, Notification) for the Overlays section demo.
/// These are rendered at the body level (outside .main-content) for proper fixed positioning.
pub fn overlays_demo_overlays(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<OverlaysSectionState>();

    let (modal_opened, modal_lg_opened, drawer_opened, drawer_right_opened, notification_visible) =
        match state {
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
            // Basic Modal
            Modal {
                opened_fn: move || modal_opened.get(),
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

            // Large Modal
            Modal {
                opened_fn: move || modal_lg_opened.get(),
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
                opened_fn: move || drawer_opened.get(),
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
                opened_fn: move || drawer_right_opened.get(),
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

            // Notification
            Notification {
                opened_fn: move || notification_visible.get(),
                onclose: move || notification_visible.set(false),
                title: "Success!",
                color: "green",
                with_close_button: true,
                "Your changes have been saved successfully."
            }
        }
    }
}
