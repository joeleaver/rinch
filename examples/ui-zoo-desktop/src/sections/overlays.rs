//! Overlays section - Modal, Drawer, Popover, Tooltip, etc.

use rinch::prelude::*;

/// State for the Overlays section, stored in context.
#[derive(Clone)]
pub struct OverlaysSectionState {
    pub modal_opened: Signal<bool>,
    pub modal_lg_opened: Signal<bool>,
    pub drawer_opened: Signal<bool>,
    pub drawer_right_opened: Signal<bool>,
    pub dropdown_opened: Signal<bool>,
    pub notification_visible: Signal<bool>,
    pub dropdown_selection: Signal<String>,
}

/// Initialize the Overlays section state. Call this from the main app function.
pub fn init_overlays_state() {
    create_context(OverlaysSectionState {
        modal_opened: Signal::new(false),
        modal_lg_opened: Signal::new(false),
        drawer_opened: Signal::new(false),
        drawer_right_opened: Signal::new(false),
        dropdown_opened: Signal::new(false),
        notification_visible: Signal::new(false),
        dropdown_selection: Signal::new("None".to_string()),
    });
}

pub fn overlays_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<OverlaysSectionState>();

    let (
        modal_opened,
        modal_lg_opened,
        drawer_opened,
        drawer_right_opened,
        dropdown_opened,
        notification_visible,
        dropdown_selection,
    ) = match state {
        Some(s) => (
            s.modal_opened,
            s.modal_lg_opened,
            s.drawer_opened,
            s.drawer_right_opened,
            s.dropdown_opened,
            s.notification_visible,
            s.dropdown_selection,
        ),
        None => {
            return rsx! {
                div { "Error: OverlaysSectionState not initialized" }
            };
        }
    };

    rsx! {
        Fragment {
            // ============================================
            // PAGE CONTENT
            // ============================================
            Stack { gap: "xs",
                Title { order: 1, "Overlays" }
                Text { size: "lg", color: "dimmed",
                    "Components that appear above other content"
                }
            }
            Space { h: "xl" }

            // ============================================
            // MODAL
            // ============================================
            Title { order: 3, "Modal" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Centered dialogs for focused user interactions." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Basic modal
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic Modal" }
                        Text { size: "sm", color: "dimmed", "Click to open a modal dialog." }
                        Divider {}
                        Button { onclick: move || modal_opened.set(true), "Open Modal" }
                    }
                }

                // Large modal
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Large Modal" }
                        Text { size: "sm", color: "dimmed", "Modals support xs, sm, md, lg, xl, and full sizes." }
                        Divider {}
                        Button { variant: "outline", onclick: move || modal_lg_opened.set(true), "Open Large Modal" }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // DRAWER
            // ============================================
            Title { order: 3, "Drawer" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Slide-out panels from screen edges." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Drawers
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Position Variants" }
                        Text { size: "sm", color: "dimmed", "Drawers can slide in from any edge." }
                        Divider {}
                        Group { gap: "sm",
                            Button { onclick: move || drawer_opened.set(true), "Left Drawer" }
                            Button { variant: "outline", onclick: move || drawer_right_opened.set(true), "Right Drawer" }
                        }
                        Space { h: "sm" }
                        Group { gap: "xs",
                            Badge { color: "blue", variant: "light", "left" }
                            Badge { color: "cyan", variant: "light", "right" }
                            Badge { color: "teal", variant: "light", "top" }
                            Badge { color: "green", variant: "light", "bottom" }
                        }
                    }
                }

                // Drawer info
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Drawer Sizes" }
                        Text { size: "sm", color: "dimmed", "Control drawer width/height with size prop." }
                        Divider {}
                        Stack { gap: "xs",
                            Group { gap: "sm",
                                Badge { "xs" }
                                Text { size: "xs", color: "dimmed", "280px" }
                            }
                            Group { gap: "sm",
                                Badge { "sm" }
                                Text { size: "xs", color: "dimmed", "320px" }
                            }
                            Group { gap: "sm",
                                Badge { "md" }
                                Text { size: "xs", color: "dimmed", "380px (default)" }
                            }
                            Group { gap: "sm",
                                Badge { "lg" }
                                Text { size: "xs", color: "dimmed", "500px" }
                            }
                            Group { gap: "sm",
                                Badge { "xl" }
                                Text { size: "xs", color: "dimmed", "680px" }
                            }
                            Group { gap: "sm",
                                Badge { "full" }
                                Text { size: "xs", color: "dimmed", "100%" }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // DROPDOWN & TOOLTIP
            // ============================================
            Title { order: 3, "Dropdown & Tooltip" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Contextual menus and hover hints." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                // Dropdown
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Dropdown Menu" }
                            Badge { color: "violet", variant: "light", {|| dropdown_selection.get()} }
                        }
                        Text { size: "sm", color: "dimmed", "Click to open a menu of actions." }
                        Divider {}
                        DropdownMenu { opened: dropdown_opened.get(),
                            DropdownMenuTarget {
                                Button { onclick: move || dropdown_opened.update(|v| *v = !*v), "Open Menu" }
                            }
                            DropdownMenuDropdown {
                                DropdownMenuItem {
                                    onclick: move || {
                                        dropdown_selection.set("Edit".to_string());
                                        dropdown_opened.set(false);
                                    },
                                    "Edit"
                                }
                                DropdownMenuItem {
                                    onclick: move || {
                                        dropdown_selection.set("Duplicate".to_string());
                                        dropdown_opened.set(false);
                                    },
                                    "Duplicate"
                                }
                                DropdownMenuDivider {}
                                DropdownMenuItem {
                                    color: "red",
                                    onclick: move || {
                                        dropdown_selection.set("Delete".to_string());
                                        dropdown_opened.set(false);
                                    },
                                    "Delete"
                                }
                            }
                        }
                    }
                }

                // Tooltip
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Tooltip" }
                        Text { size: "sm", color: "dimmed", "Brief hints that appear on hover." }
                        Divider {}
                        Group { gap: "md",
                            Tooltip { label: "This is a tooltip",
                                Button { variant: "outline", "Hover me" }
                            }
                            Tooltip { label: "Right position", position: "right",
                                Button { variant: "light", "Or me" }
                            }
                            Tooltip { label: "Bottom tooltip", position: "bottom",
                                Badge { size: "lg", "Badge" }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // NOTIFICATION
            // ============================================
            Title { order: 3, "Notification" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Toast-style messages for temporary feedback." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { weight: "600", "Interactive Notification" }
                    Text { size: "sm", color: "dimmed", "Notifications appear as floating toasts." }
                    Divider {}
                    Button { color: "green", onclick: move || notification_visible.set(true), "Show Notification" }
                }
            }

            // Note: Actual Modal, Drawer, and Notification overlay components are rendered
            // at the body level in main.rs for proper fixed positioning over the entire window.
        }
    }
}
