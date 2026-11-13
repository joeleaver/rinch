//! UI Zoo - Rinch Component Library Showcase
//!
//! A platform-agnostic library exporting section components and shared helpers.
//! Each platform binary (desktop, web) provides its own shell using these exports.

pub mod section;
pub mod sections;

pub use section::Section;

use rinch::prelude::*;
use sections::*;
use std::rc::Rc;

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
    init_av_state();
}

/// Render the navigation links list: one link a section, in [`Section::ALL`]'s
/// order.
///
/// Generic over the nav callback so each platform can provide its own
/// navigation behavior (e.g. closing a drawer on desktop, no-op on web).
#[component]
pub fn nav_links<F: Fn() + 'static>(
    current_section: Signal<usize>,
    nav: impl Fn(usize) -> F,
) -> NodeHandle {
    let links = rsx! { Stack { gap: "0", style: "flex: 1;" } };
    for &section in Section::ALL {
        let index = section.index();
        let link = rsx! {
            NavLink {
                label: section.label(),
                active_fn: move || current_section.get() == index,
                onclick: nav(index)
            }
        };
        links.append_child(&link);
    }
    links
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
                label: "Dark Mode",
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

/// Render the section content area: the [`Section`] at `current_section`,
/// built again when the section changes (and only then), nothing for an
/// index past the last.
pub fn section_content(__scope: &mut RenderScope, current_section: Signal<usize>) -> NodeHandle {
    // A memo, so that setting the section already shown builds nothing.
    let shown = Memo::new(move || Section::at(current_section.get()));
    rsx! {
        Fragment {
            match shown.get() {
                Some(section) => { section.render(__scope) },
                None => div { },
            }
        }
    }
}

/// Renders the overlay components (Modal, Drawer, Notification) for the Overlays section demo.
/// These are rendered at the body level (outside .main-content) for proper fixed positioning.
pub fn overlays_demo_overlays(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_store::<OverlaysSectionState>();

    let (modal_opened, modal_lg_opened, drawer_opened, drawer_right_opened, notification_visible) = (
        state.modal_opened,
        state.modal_lg_opened,
        state.drawer_opened,
        state.drawer_right_opened,
        state.notification_visible,
    );

    // The basic modal's buttons go in its pinned footer, which stays in view
    // while the body scrolls.
    let modal_footer: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            Fragment {
                Button { variant: "subtle", onclick: move || modal_opened.set(false), "Cancel" }
                Button { onclick: move || modal_opened.set(false), "Save" }
            }
        }
    });

    rsx! {
        Fragment {
            // Basic Modal
            Modal {
                opened_fn: move || modal_opened.get(),
                onclose: move || modal_opened.set(false),
                title: "Welcome!",
                footer: Some(modal_footer),

                Text { size: "sm", color: "dimmed",
                    "This is a working modal dialog. Click outside or the X button to close it. Its Cancel and Save buttons are in a footer pinned below the body."
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
                    NavLink { label: "Home", active: true }
                    NavLink { label: "Dashboard" }
                    NavLink { label: "Settings" }
                    NavLink { label: "Profile" }
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
