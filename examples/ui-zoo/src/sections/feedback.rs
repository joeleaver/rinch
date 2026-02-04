//! Feedback section - Alert, Progress, Loader, and Skeleton demos.

use rinch::prelude::*;
use std::rc::Rc;

/// State for the Feedback section, stored in context.
#[derive(Clone)]
pub struct FeedbackSectionState {
    pub progress_value: Signal<f64>,
}

/// Initialize the Feedback section state.
pub fn init_feedback_state() {
    create_context(FeedbackSectionState {
        progress_value: Signal::new(65.0),
    });
}

pub fn feedback_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<FeedbackSectionState>();

    let progress_value = match state {
        Some(s) => s.progress_value,
        None => {
            return rsx! { div { "Error: FeedbackSectionState not initialized" } };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Feedback" }
                Text { size: "lg", color: "dimmed",
                    "Components for displaying status, alerts, and loading states."
                }
            }
            Space { h: "xl" }

            // Alerts
            Title { order: 3, "Alerts" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Display important messages and notifications." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alert Colors" }
                        Divider {}
                        Alert { color: "blue", title: "Information",
                            "This is an informational message."
                        }
                        Alert { color: "green", title: "Success",
                            "Your changes have been saved."
                        }
                        Alert { color: "yellow", title: "Warning",
                            "Please review before continuing."
                        }
                        Alert { color: "red", title: "Error",
                            "Something went wrong."
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Alert Variants" }
                        Divider {}
                        Alert { color: "blue", variant: "light", title: "Light",
                            "Light variant with subtle background."
                        }
                        Alert { color: "blue", variant: "filled", title: "Filled",
                            "Filled variant with solid background."
                        }
                        Alert { color: "blue", variant: "outline", title: "Outline",
                            "Outline variant with border only."
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Progress
            Title { order: 3, "Progress" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Progress bars with interactive control." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "lg",
                    Text { weight: "600", "Interactive Progress" }
                    Group { gap: "sm",
                        Button {
                            size: "xs",
                            variant: "light",
                            onclick: move || progress_value.update(|v| *v = (*v - 10.0_f64).max(0.0)),
                            "-10"
                        }
                        Button {
                            size: "xs",
                            variant: "light",
                            onclick: move || progress_value.update(|v| *v = (*v + 10.0_f64).min(100.0)),
                            "+10"
                        }
                        Text { size: "sm", color: "dimmed", {|| format!("{}%", progress_value.get() as i32)} }
                    }
                    Progress {
                        value_fn: Some(Rc::new(move || progress_value.get() as f32))
                    }
                    Divider {}
                    Text { weight: "600", "Sizes" }
                    Stack { gap: "sm",
                        Text { size: "xs", color: "dimmed", "xs" }
                        Progress { value: Some(40.0), size: "xs" }
                        Text { size: "xs", color: "dimmed", "sm" }
                        Progress { value: Some(55.0), size: "sm" }
                        Text { size: "xs", color: "dimmed", "md" }
                        Progress { value: Some(70.0), size: "md" }
                        Text { size: "xs", color: "dimmed", "lg" }
                        Progress { value: Some(85.0), size: "lg" }
                    }
                    Divider {}
                    Text { weight: "600", "Colors" }
                    Stack { gap: "sm",
                        Progress { value: Some(60.0), color: "blue" }
                        Progress { value: Some(60.0), color: "green" }
                        Progress { value: Some(60.0), color: "orange" }
                        Progress { value: Some(60.0), color: "red" }
                    }
                }
            }

            Space { h: "xl" }

            // Loader
            Title { order: 3, "Loader" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Loading indicators with multiple animation types." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Types" }
                        Divider {}
                        Group { gap: "xl",
                            Stack { align: "center", gap: "sm",
                                Loader {}
                                Text { size: "xs", color: "dimmed", "Oval" }
                            }
                            Stack { align: "center", gap: "sm",
                                Loader { r#type: "bars" }
                                Text { size: "xs", color: "dimmed", "Bars" }
                            }
                            Stack { align: "center", gap: "sm",
                                Loader { r#type: "dots" }
                                Text { size: "xs", color: "dimmed", "Dots" }
                            }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Sizes" }
                        Divider {}
                        Group { gap: "xl", align: "center",
                            Loader { size: "xs" }
                            Loader { size: "sm" }
                            Loader { size: "md" }
                            Loader { size: "lg" }
                            Loader { size: "xl" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Skeleton
            Title { order: 3, "Skeleton" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Loading placeholders that mimic content shape." }
            Space { h: "md" }

            Paper { p: "xl", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { gap: "md",
                        Skeleton { circle: true, height: "50px" }
                        Stack { gap: "sm",
                            Skeleton { height: "12px", width: "200px" }
                            Skeleton { height: "12px", width: "150px" }
                        }
                    }
                    Skeleton { height: "12px" }
                    Skeleton { height: "12px" }
                    Skeleton { height: "12px", width: "70%" }
                }
            }
        }
    }
}
