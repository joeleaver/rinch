//! Buttons section - Button, ActionIcon, and CloseButton demos.

use rinch::prelude::*;

/// State for the Buttons section, stored in context.
#[derive(Clone)]
pub struct ButtonsSectionState {
    pub counter: Signal<i32>,
}

/// Initialize the Buttons section state.
pub fn init_buttons_state() {
    create_context(ButtonsSectionState {
        counter: Signal::new(0),
    });
}

pub fn buttons_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<ButtonsSectionState>();

    let counter = match state {
        Some(s) => s.counter,
        None => {
            return rsx! { div { "Error: ButtonsSectionState not initialized" } };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Buttons" }
                Text { size: "lg", color: "dimmed",
                    "Clickable button components with multiple variants, sizes, and states."
                }
            }
            Space { h: "xl" }

            // Interactive counter demo
            Paper { p: "xl", radius: "md", with_border: true,
                Center {
                    Stack { align: "center", gap: "lg",
                        Title { order: 3, "Interactive Demo" }
                        Text { color: "dimmed", "Click buttons to change the counter value" }
                        Group { align: "center", gap: "lg",
                            Button {
                                variant: "light",
                                color: "red",
                                size: "lg",
                                onclick: move || counter.update(|n| *n -= 1),
                                "-"
                            }
                            div {
                                style: "min-width: 60px; text-align: center;",
                                Text {
                                    size: "xl",
                                    weight: "700",
                                    {|| counter.get().to_string()}
                                }
                            }
                            Button {
                                color: "green",
                                size: "lg",
                                onclick: move || counter.update(|n| *n += 1),
                                "+"
                            }
                            Button {
                                variant: "subtle",
                                color: "gray",
                                onclick: move || counter.set(0),
                                "Reset"
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Variants
            Title { order: 3, "Variants" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Different button styles for various use cases." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("md".to_string()),
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Filled" }
                        Text { size: "sm", color: "dimmed", "Primary actions, high emphasis" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "filled", "Default" }
                            Button { variant: "filled", color: "green", "Success" }
                            Button { variant: "filled", color: "red", "Danger" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Outline" }
                        Text { size: "sm", color: "dimmed", "Secondary actions, medium emphasis" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "outline", "Default" }
                            Button { variant: "outline", color: "green", "Success" }
                            Button { variant: "outline", color: "red", "Danger" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Light" }
                        Text { size: "sm", color: "dimmed", "Subtle actions, low emphasis" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "light", "Default" }
                            Button { variant: "light", color: "green", "Success" }
                            Button { variant: "light", color: "red", "Danger" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Subtle & Disabled" }
                        Text { size: "sm", color: "dimmed", "Minimal styling and non-interactive state" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "subtle", "Subtle" }
                            Button { disabled: true, "Disabled" }
                            Button { variant: "outline", disabled: true, "Disabled" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Sizes
            Title { order: 3, "Sizes" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Five size options from extra small to extra large." }
            Space { h: "md" }

            Paper { p: "lg", radius: "md", with_border: true,
                Group { align: "end", gap: "md",
                    Button { size: "xs", "Extra Small" }
                    Button { size: "sm", "Small" }
                    Button { size: "md", "Medium" }
                    Button { size: "lg", "Large" }
                    Button { size: "xl", "Extra Large" }
                }
            }

            Space { h: "xl" }

            // Colors
            Title { order: 3, "Colors" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Full color palette across all variants." }
            Space { h: "md" }

            Paper { p: "lg", radius: "md", with_border: true,
                Stack { gap: "md",
                    Group { gap: "sm",
                        Button { color: "blue", "Blue" }
                        Button { color: "cyan", "Cyan" }
                        Button { color: "teal", "Teal" }
                        Button { color: "green", "Green" }
                        Button { color: "lime", "Lime" }
                    }
                    Group { gap: "sm",
                        Button { color: "yellow", "Yellow" }
                        Button { color: "orange", "Orange" }
                        Button { color: "red", "Red" }
                        Button { color: "pink", "Pink" }
                        Button { color: "grape", "Grape" }
                    }
                }
            }

            Space { h: "xl" }

            // ActionIcon and CloseButton
            Title { order: 3, "ActionIcon & CloseButton" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Compact icon-only buttons for toolbar actions." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("md".to_string()),
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "ActionIcon Variants" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            ActionIcon { variant: "filled", "+" }
                            ActionIcon { variant: "light", "-" }
                            ActionIcon { variant: "outline", "x" }
                            ActionIcon { variant: "subtle", "?" }
                            ActionIcon { variant: "default", "i" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "CloseButton Sizes" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            CloseButton { size: "xs" }
                            CloseButton { size: "sm" }
                            CloseButton { size: "md" }
                            CloseButton { size: "lg" }
                            CloseButton { size: "xl" }
                        }
                    }
                }
            }
        }
    }
}
