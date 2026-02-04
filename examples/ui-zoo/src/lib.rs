//! UI Zoo - Interactive showcase of all rinch widgets.
//!
//! This library exports the app component and section modules so they can be
//! shared between the desktop binary and the WASM web build.

pub mod sections;

use rinch::prelude::*;
use sections::*;
use std::rc::Rc;

/// The main UI Zoo application component.
pub fn app(__scope: &mut RenderScope) -> NodeHandle {
    let current_section = use_signal(|| 0_usize);

    // Initialize section state contexts synchronously.
    init_buttons_state();
    init_inputs_state();
    init_feedback_state();
    init_overlays_state();

    // Nav helper
    #[allow(unused_variables)]
    let nav = |idx: usize| {
        move || {
            current_section.set(idx);
        }
    };

    rsx! {
        ThemeProvider {
            primary_color: "blue",
            default_radius: "md",

            style { {CSS} }

            div { class: "app-shell",
                // Sidebar navigation
                div { class: "sidebar",
                    div { class: "sidebar-header",
                        Title { order: 3, "UI Zoo" }
                        Text { size: "xs", color: "dimmed", "Rinch Widget Showcase" }
                    }

                    Space { h: "md" }

                    Stack { gap: "0",
                        NavLink {
                            label: Some("Buttons".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 0)),
                            onclick: nav(0)
                        }
                        NavLink {
                            label: Some("Inputs".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 1)),
                            onclick: nav(1)
                        }
                        NavLink {
                            label: Some("Typography".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 2)),
                            onclick: nav(2)
                        }
                        NavLink {
                            label: Some("Layout".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 3)),
                            onclick: nav(3)
                        }
                        NavLink {
                            label: Some("Feedback".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 4)),
                            onclick: nav(4)
                        }
                        NavLink {
                            label: Some("Data Display".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 5)),
                            onclick: nav(5)
                        }
                        NavLink {
                            label: Some("Navigation".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 6)),
                            onclick: nav(6)
                        }
                        NavLink {
                            label: Some("Overlays".to_string()),
                            active_fn: Some(Rc::new(move || current_section.get() == 7)),
                            onclick: nav(7)
                        }
                    }
                }

                // Main content area
                div { class: "main-content",
                    Show { when: move || current_section.get() == 0, then: |__scope| buttons_section(__scope) }
                    Show { when: move || current_section.get() == 1, then: |__scope| inputs_section(__scope) }
                    Show { when: move || current_section.get() == 2, then: |__scope| typography_section(__scope) }
                    Show { when: move || current_section.get() == 3, then: |__scope| layout_section(__scope) }
                    Show { when: move || current_section.get() == 4, then: |__scope| feedback_section(__scope) }
                    Show { when: move || current_section.get() == 5, then: |__scope| data_display_section(__scope) }
                    Show { when: move || current_section.get() == 6, then: |__scope| navigation_section(__scope) }
                    Show { when: move || current_section.get() == 7, then: |__scope| overlays_section(__scope) }
                }
            }

            // Overlay components rendered at body level for proper fixed positioning
            {overlays_demo_overlays(__scope)}
        }
    }
}

/// Renders overlay components (Modal, Notification) for the Overlays section.
/// These are rendered at the body level for proper fixed positioning.
fn overlays_demo_overlays(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<OverlaysSectionState>();

    let (modal_opened, notification_visible) = match state {
        Some(s) => (s.modal_opened, s.notification_visible),
        None => {
            return rsx! { div {} };
        }
    };

    rsx! {
        Fragment {
            Modal {
                opened_fn: Some(Rc::new(move || modal_opened.get())),
                onclose: move || modal_opened.set(false),
                title: "Example Modal",

                Text { size: "sm", color: "dimmed",
                    "This is a working modal dialog. Click outside or the X button to close."
                }
                Space { h: "lg" }
                Group { justify: "flex-end", gap: "sm",
                    Button { variant: "subtle", onclick: move || modal_opened.set(false), "Cancel" }
                    Button { onclick: move || modal_opened.set(false), "Confirm" }
                }
            }

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
pub const CSS: &str = r#"
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
