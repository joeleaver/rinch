//! Feedback section - Alert, Progress, Loader, Skeleton with interactive demos.

use rinch::prelude::*;

/// State for the Feedback section, stored in context.
#[derive(Clone)]
pub struct FeedbackSectionState {
    pub progress_value: Signal<f64>,
    pub notification_visible: Signal<bool>,
    pub notification_error_visible: Signal<bool>,
}

/// Initialize the Feedback section state. Call this from the main app function.
pub fn init_feedback_state() {
    create_context(FeedbackSectionState {
        progress_value: Signal::new(65.0),
        notification_visible: Signal::new(false),
        notification_error_visible: Signal::new(false),
    });
}

#[component]
pub fn feedback_section() -> NodeHandle {
    let state = use_context::<FeedbackSectionState>();

    let (progress_value, notification_visible, notification_error_visible) = match state {
        Some(s) => (
            s.progress_value,
            s.notification_visible,
            s.notification_error_visible,
        ),
        None => {
            return rsx! {
                div { "Error: FeedbackSectionState not initialized" }
            };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Feedback" }
                Text { size: "lg", color: "dimmed",
                    "Components for displaying status, alerts, and loading states"
                }
            }
            Space { h: "xl" }

            // ============================================
            // ALERTS
            // ============================================
            Title { order: 3, "Alerts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Display important messages and notifications." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Alert colors
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alert Colors" }
                        Divider {}
                        Alert { color: "blue", title: "Information",
                            "This is an informational message."
                        }
                        Alert { color: "green", title: "Success",
                            "Operation completed successfully!"
                        }
                        Alert { color: "yellow", title: "Warning",
                            "Please review before continuing."
                        }
                        Alert { color: "red", title: "Error",
                            "An error occurred. Please try again."
                        }
                    }
                }

                // Alert variants
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alert Variants" }
                        Divider {}
                        Alert { variant: "light", color: "blue", title: "Light",
                            "Subtle background (default)"
                        }
                        Alert { variant: "filled", color: "blue", title: "Filled",
                            "Solid background color"
                        }
                        Alert { variant: "outline", color: "blue", title: "Outline",
                            "Border with transparent background"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // PROGRESS
            // ============================================
            Title { order: 3, "Progress" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Show completion status of tasks or processes." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Interactive progress
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Interactive" }
                            Badge { color: "blue", {|| format!("{:.0}%", progress_value.get())} }
                        }
                        Divider {}
                        Progress { value: Some(progress_value.get() as f32), color: "blue", size: "lg" }
                        Slider {
                            min: Some(0.0),
                            max: Some(100.0),
                            value_signal: Some(progress_value),
                            onchange: move |v| progress_value.set(v)
                        }
                    }
                }

                // Progress colors and sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Colors & Sizes" }
                        Divider {}
                        Stack { gap: "sm",
                            Group { gap: "sm",
                                Text { size: "xs", color: "dimmed", "xs" }
                                div { style: "flex: 1;",
                                    Progress { value: Some(30.0), color: "cyan", size: "xs" }
                                }
                            }
                            Group { gap: "sm",
                                Text { size: "xs", color: "dimmed", "sm" }
                                div { style: "flex: 1;",
                                    Progress { value: Some(50.0), color: "teal", size: "sm" }
                                }
                            }
                            Group { gap: "sm",
                                Text { size: "xs", color: "dimmed", "md" }
                                div { style: "flex: 1;",
                                    Progress { value: Some(70.0), color: "green", size: "md" }
                                }
                            }
                            Group { gap: "sm",
                                Text { size: "xs", color: "dimmed", "lg" }
                                div { style: "flex: 1;",
                                    Progress { value: Some(90.0), color: "violet", size: "lg" }
                                }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // NOTIFICATIONS
            // ============================================
            Title { order: 3, "Notifications" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Toast-style messages for temporary feedback." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Text { weight: "600", "Interactive Notifications" }
                    Text { size: "sm", color: "dimmed", "Click buttons to show toast notifications." }
                    Divider {}
                    Group { gap: "sm",
                        Button { color: "green", onclick: move || notification_visible.set(true),
                            "Show Success"
                        }
                        Button { color: "red", variant: "outline", onclick: move || notification_error_visible.set(true),
                            "Show Error"
                        }
                    }
                    Notification {
                        opened_fn: move || notification_visible.get(),
                        onclose: move || notification_visible.set(false),
                        title: "Saved!",
                        color: "green",
                        with_close_button: true,
                        "Your changes have been saved successfully."
                    }
                    Notification {
                        opened_fn: move || notification_error_visible.get(),
                        onclose: move || notification_error_visible.set(false),
                        title: "Error",
                        color: "red",
                        with_close_button: true,
                        position: "top-left",
                        "Failed to save changes. Please try again."
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // LOADERS
            // ============================================
            Title { order: 3, "Loaders" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Indicate loading or processing states." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Loader sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Sizes & Colors" }
                        Divider {}
                        Group { gap: "lg", align: "center",
                            Stack { align: "center", gap: "xs",
                                Loader { size: "xs" }
                                Text { size: "xs", color: "dimmed", "xs" }
                            }
                            Stack { align: "center", gap: "xs",
                                Loader { size: "sm", color: "cyan" }
                                Text { size: "xs", color: "dimmed", "sm" }
                            }
                            Stack { align: "center", gap: "xs",
                                Loader { size: "md", color: "green" }
                                Text { size: "xs", color: "dimmed", "md" }
                            }
                            Stack { align: "center", gap: "xs",
                                Loader { size: "lg", color: "violet" }
                                Text { size: "xs", color: "dimmed", "lg" }
                            }
                            Stack { align: "center", gap: "xs",
                                Loader { size: "xl", color: "red" }
                                Text { size: "xs", color: "dimmed", "xl" }
                            }
                        }
                    }
                }

                // Loader types
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Types" }
                        Divider {}
                        Group { gap: "xl",
                            Stack { align: "center", gap: "sm",
                                Loader { r#type: "oval", size: "lg" }
                                Text { size: "xs", color: "dimmed", "Oval" }
                            }
                            Stack { align: "center", gap: "sm",
                                Loader { r#type: "dots", size: "lg" }
                                Text { size: "xs", color: "dimmed", "Dots" }
                            }
                            Stack { align: "center", gap: "sm",
                                Loader { r#type: "bars", size: "lg" }
                                Text { size: "xs", color: "dimmed", "Bars" }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // SKELETON
            // ============================================
            Title { order: 3, "Skeleton" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Placeholder content while loading." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Basic skeleton
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Text Skeleton" }
                        Divider {}
                        Stack { gap: "sm",
                            Skeleton { height: "12px", radius: "xl" }
                            Skeleton { height: "12px", width: "80%", radius: "xl" }
                            Skeleton { height: "12px", width: "60%", radius: "xl" }
                        }
                    }
                }

                // Profile skeleton
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Profile Pattern" }
                        Divider {}
                        Group { gap: "md",
                            Skeleton { circle: true, height: "50px" }
                            Stack { gap: "xs",
                                Skeleton { height: "14px", width: "140px" }
                                Skeleton { height: "10px", width: "100px" }
                            }
                        }
                    }
                }

                // Card skeleton
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Card Pattern" }
                        Divider {}
                        Paper { radius: "md", shadow: "sm",
                            Skeleton { height: "100px" }
                            div { style: "padding: var(--rinch-spacing-md);",
                                Stack { gap: "xs",
                                    Skeleton { height: "14px", width: "70%" }
                                    Skeleton { height: "10px" }
                                    Skeleton { height: "10px", width: "85%" }
                                }
                            }
                        }
                    }
                }

                // List skeleton
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "List Pattern" }
                        Divider {}
                        Stack { gap: "sm",
                            Group { gap: "sm",
                                Skeleton { circle: true, height: "32px" }
                                Skeleton { height: "12px", width: "200px" }
                            }
                            Group { gap: "sm",
                                Skeleton { circle: true, height: "32px" }
                                Skeleton { height: "12px", width: "180px" }
                            }
                            Group { gap: "sm",
                                Skeleton { circle: true, height: "32px" }
                                Skeleton { height: "12px", width: "220px" }
                            }
                        }
                    }
                }
            }
        }
    }
}
