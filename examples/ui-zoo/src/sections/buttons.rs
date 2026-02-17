//! Buttons section - Button variants and states with interactive demos.

use rinch::prelude::*;

/// State for the Buttons section, stored in context.
#[derive(Clone)]
pub struct ButtonsSectionState {
    pub counter: Signal<i32>,
}

/// Initialize the Buttons section state. Call this from the main app function.
pub fn init_buttons_state() {
    create_context(ButtonsSectionState {
        counter: Signal::new(0),
    });
}

#[component]
pub fn buttons_section() -> NodeHandle {
    // Get state from context
    let state = use_context::<ButtonsSectionState>();

    let counter = match state {
        Some(s) => s.counter,
        None => {
            return rsx! {
                div { "Error: ButtonsSectionState not initialized" }
            };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Buttons" }
                Text { size: "lg", color: "dimmed",
                    "Button components with multiple variants, sizes, and states."
                }
            }
            Space { h: "xl" }

            // Interactive Demo - Hero section
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

            // Variants section
            Title { order: 3, "Variants" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Different button styles for various use cases." }
            Space { h: "md" }

            SimpleGrid { cols: Some(3), spacing: "md",
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
                        Text { weight: "600", "Subtle" }
                        Text { size: "sm", color: "dimmed", "Minimal styling, text-like" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "subtle", "Default" }
                            Button { variant: "subtle", color: "green", "Success" }
                            Button { variant: "subtle", color: "red", "Danger" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Default" }
                        Text { size: "sm", color: "dimmed", "Neutral gray styling" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { variant: "default", "Action" }
                            Button { variant: "default", "Cancel" }
                        }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Text { weight: "600", "Disabled" }
                        Text { size: "sm", color: "dimmed", "Non-interactive state" }
                        Space { h: "xs" }
                        Group { gap: "sm",
                            Button { disabled: true, "Disabled" }
                            Button { variant: "outline", disabled: true, "Disabled" }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Sizes section
            Title { order: 3, "Sizes" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Five size options from extra small to extra large." }
            Space { h: "md" }

            Paper { p: "lg", radius: "md", with_border: true,
                Group { align: "end", gap: "md",
                    Stack { align: "center", gap: "xs",
                        Button { size: "xs", "Extra Small" }
                        Text { size: "xs", color: "dimmed", "xs" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { size: "sm", "Small" }
                        Text { size: "xs", color: "dimmed", "sm" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { size: "md", "Medium" }
                        Text { size: "xs", color: "dimmed", "md" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { size: "lg", "Large" }
                        Text { size: "xs", color: "dimmed", "lg" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { size: "xl", "Extra Large" }
                        Text { size: "xs", color: "dimmed", "xl" }
                    }
                }
            }

            Space { h: "xl" }

            // Colors section
            Title { order: 3, "Colors" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Full color palette support across all variants." }
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
                    Group { gap: "sm",
                        Button { color: "violet", "Violet" }
                        Button { color: "indigo", "Indigo" }
                        Button { color: "gray", "Gray" }
                    }
                }
            }

            Space { h: "xl" }

            // Border Radius section
            Title { order: 3, "Border Radius" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Customize button corners from sharp to fully rounded." }
            Space { h: "md" }

            Paper { p: "lg", radius: "md", with_border: true,
                Group { gap: "md",
                    Stack { align: "center", gap: "xs",
                        Button { radius: "xs", "Sharp" }
                        Text { size: "xs", color: "dimmed", "xs" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { radius: "sm", "Small" }
                        Text { size: "xs", color: "dimmed", "sm" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { radius: "md", "Medium" }
                        Text { size: "xs", color: "dimmed", "md" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { radius: "lg", "Large" }
                        Text { size: "xs", color: "dimmed", "lg" }
                    }
                    Stack { align: "center", gap: "xs",
                        Button { radius: "xl", "Pill" }
                        Text { size: "xs", color: "dimmed", "xl" }
                    }
                }
            }

            Space { h: "xl" }

            // Full Width section
            Title { order: 3, "Full Width" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Buttons that expand to fill their container." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "md",
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Button { full_width: true, "Full Width Filled" }
                        Button { full_width: true, variant: "outline", "Full Width Outline" }
                        Button { full_width: true, variant: "light", "Full Width Light" }
                    }
                }
                Paper { p: "lg", radius: "md", with_border: true,
                    Stack { gap: "sm",
                        Button { full_width: true, color: "green", "Confirm Action" }
                        Button { full_width: true, variant: "outline", color: "red", "Cancel" }
                        Button { full_width: true, variant: "subtle", color: "gray", "Skip for now" }
                    }
                }
            }
        }
    }
}
