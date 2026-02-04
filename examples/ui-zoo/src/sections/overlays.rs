//! Overlays section - Modal, Notification, and Tooltip demos.

use rinch::prelude::*;

/// State for the Overlays section, stored in context.
#[derive(Clone)]
pub struct OverlaysSectionState {
    pub modal_opened: Signal<bool>,
    pub notification_visible: Signal<bool>,
}

/// Initialize the Overlays section state.
pub fn init_overlays_state() {
    create_context(OverlaysSectionState {
        modal_opened: Signal::new(false),
        notification_visible: Signal::new(false),
    });
}

pub fn overlays_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<OverlaysSectionState>();

    let (modal_opened, notification_visible) = match state {
        Some(s) => (s.modal_opened, s.notification_visible),
        None => {
            return rsx! { div { "Error: OverlaysSectionState not initialized" } };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Overlays" }
                Text { size: "lg", color: "dimmed",
                    "Components that display content above the page."
                }
            }
            Space { h: "xl" }

            // Modal
            Title { order: 3, "Modal" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Dialog overlay that captures focus." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { weight: "600", "Open a Modal" }
                    Text { size: "sm", color: "dimmed",
                        "Click the button below to open a modal dialog."
                    }
                    Group { gap: "sm",
                        Button {
                            onclick: move || modal_opened.set(true),
                            "Open Modal"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Notification
            Title { order: 3, "Notification" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Toast notifications for transient messages." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { weight: "600", "Show Notification" }
                    Text { size: "sm", color: "dimmed",
                        "Click the button to display a notification toast."
                    }
                    Group { gap: "sm",
                        Button {
                            color: "green",
                            onclick: move || notification_visible.set(true),
                            "Show Notification"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // LoadingOverlay info
            Title { order: 3, "LoadingOverlay" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Cover a container with a loading indicator." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { weight: "600", "Usage" }
                    Text { size: "sm", color: "dimmed",
                        "LoadingOverlay covers its parent container with a dimmed overlay and centered loader. Use it to indicate background processing on a specific section of the UI."
                    }
                    Code { block: true,
                        "LoadingOverlay { visible: true }"
                    }
                }
            }
        }
    }
}
