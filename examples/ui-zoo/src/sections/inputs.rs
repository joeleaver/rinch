//! Inputs section - Form controls and input components.

use rinch::prelude::*;

/// State for the Inputs section, stored in context.
#[derive(Clone)]
pub struct InputsSectionState {
    pub check1: Signal<bool>,
    pub check2: Signal<bool>,
    pub check_xs: Signal<bool>,
    pub check_sm: Signal<bool>,
    pub check_md: Signal<bool>,
    pub check_lg: Signal<bool>,
    pub switch1: Signal<bool>,
    pub switch2: Signal<bool>,
    pub switch_xs: Signal<bool>,
    pub switch_sm: Signal<bool>,
    pub switch_md: Signal<bool>,
    pub switch_lg: Signal<bool>,
    pub quantity: Signal<f64>,
    pub price: Signal<f64>,
    pub selected_plan: Signal<String>,
    pub volume: Signal<f64>,
    pub brightness: Signal<f64>,
    pub password_visible: Signal<bool>,
    pub password_value: Signal<String>,
}

/// Initialize the Inputs section state. Call this from the main app function.
pub fn init_inputs_state() {
    create_context(InputsSectionState {
        check1: Signal::new(false),
        check2: Signal::new(true),
        check_xs: Signal::new(true),
        check_sm: Signal::new(true),
        check_md: Signal::new(true),
        check_lg: Signal::new(true),
        switch1: Signal::new(false),
        switch2: Signal::new(true),
        switch_xs: Signal::new(true),
        switch_sm: Signal::new(true),
        switch_md: Signal::new(true),
        switch_lg: Signal::new(true),
        quantity: Signal::new(1.0),
        price: Signal::new(9.99),
        selected_plan: Signal::new("free".to_string()),
        volume: Signal::new(50.0),
        brightness: Signal::new(75.0),
        password_visible: Signal::new(false),
        password_value: Signal::new(String::new()), // Start empty for demo
    });
}

#[component]
pub fn inputs_section() -> NodeHandle {
    // Get state from context (initialized in main app)
    let state = use_context::<InputsSectionState>();

    let (
        check1,
        check2,
        check_xs,
        check_sm,
        check_md,
        check_lg,
        switch1,
        switch2,
        switch_xs,
        switch_sm,
        switch_md,
        switch_lg,
        quantity,
        price,
        selected_plan,
        volume,
        brightness,
        password_visible_sig,
        password_value,
    ) = (
        state.check1,
        state.check2,
        state.check_xs,
        state.check_sm,
        state.check_md,
        state.check_lg,
        state.switch1,
        state.switch2,
        state.switch_xs,
        state.switch_sm,
        state.switch_md,
        state.switch_lg,
        state.quantity,
        state.price,
        state.selected_plan,
        state.volume,
        state.brightness,
        state.password_visible,
        state.password_value,
    );

    let toggle = |sig: Signal<bool>| move || sig.update(|v| *v = !*v);

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Inputs" }
                Text { size: "lg", color: "dimmed",
                    "Form controls for collecting user input"
                }
            }
            Space { h: "xl" }

            // ============================================
            // TEXT INPUTS
            // ============================================
            Title { order: 3, "Text Inputs" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Single and multi-line text input components." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // TextInput with labels
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic TextInput" }
                        Text { size: "sm", color: "dimmed", "Text inputs with labels and descriptions" }
                        Space { h: "xs" }
                        TextInput { label: "Username", placeholder: "Enter username" }
                        TextInput {
                            label: "Email",
                            placeholder: "you@example.com",
                            description: "We'll never share your email"
                        }
                    }
                }

                // TextInput states
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Input States" }
                        Text { size: "sm", color: "dimmed", "Error and disabled states" }
                        Space { h: "xs" }
                        TextInput { label: "With Error", error: "This field is required" }
                        TextInput { label: "Disabled", disabled: true, placeholder: "Can't edit this" }
                    }
                }

                // TextInput sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Sizes" }
                        Text { size: "sm", color: "dimmed", "Four size options" }
                        Space { h: "xs" }
                        Group { align: "end", gap: "sm",
                            TextInput { size: "xs", placeholder: "xs" }
                            TextInput { size: "sm", placeholder: "sm" }
                            TextInput { size: "md", placeholder: "md" }
                            TextInput { size: "lg", placeholder: "lg" }
                        }
                    }
                }

                // Textarea
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Textarea" }
                        Text { size: "sm", color: "dimmed", "Multi-line text input" }
                        Space { h: "xs" }
                        Textarea {
                            label: "Description",
                            placeholder: "Enter a detailed description...",
                            min_rows: 3,
                        }
                    }
                }

                // PasswordInput
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Password Input" }
                        Text { size: "sm", color: "dimmed", "Secure input with visibility toggle" }
                        Space { h: "xs" }
                        PasswordInput {
                            label: "Password",
                            placeholder: "Enter password",
                            value_fn: move || password_value.get(),
                            visible_fn: move || password_visible_sig.get(),
                            oninput: move |new_value| password_value.set(new_value),
                            ontoggle: move || password_visible_sig.update(|v| *v = !*v)
                        }
                        Text { size: "xs", color: "dimmed",
                            "Length: " {|| password_value.get().len().to_string()} " chars"
                        }
                    }
                }

                // NumberInput
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Number Input" }
                        Text { size: "sm", color: "dimmed", "Numeric input with controls" }
                        Space { h: "xs" }
                        NumberInput {
                            label: "Quantity",
                            value: Some(quantity.get()),
                            min: Some(0.0),
                            step: Some(1.0),
                            onincrement: move || quantity.update(|v| *v += 1.0),
                            ondecrement: move || quantity.update(|v| *v = (*v - 1.0).max(0.0))
                        }
                        NumberInput {
                            label: "Price",
                            prefix: "$",
                            value: Some(price.get()),
                            min: Some(0.0),
                            step: Some(0.01),
                            decimal_scale: Some(2),
                            onincrement: move || price.update(|v| *v += 0.01),
                            ondecrement: move || price.update(|v| *v = (*v - 0.01).max(0.0))
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // SELECTION INPUTS
            // ============================================
            Title { order: 3, "Selection Inputs" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Checkboxes, switches, and radio buttons for selecting options." }
            Space { h: "md" }

            SimpleGrid { cols: Some(3), spacing: "lg",
                // Checkbox
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Checkbox" }
                        Text { size: "sm", color: "dimmed", "Toggle individual options" }
                        Space { h: "xs" }
                        Checkbox {
                            label: "Accept terms",
                            checked_fn: move || check1.get(),
                            onchange: toggle(check1)
                        }
                        Checkbox {
                            label: "Newsletter",
                            checked_fn: move || check2.get(),
                            onchange: toggle(check2)
                        }
                        Checkbox { label: "Disabled", disabled: true }
                        Checkbox { label: "Indeterminate", indeterminate: true }
                    }
                }

                // Checkbox sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Checkbox Sizes" }
                        Text { size: "sm", color: "dimmed", "Four size options" }
                        Space { h: "xs" }
                        Checkbox {
                            label: "Extra small", size: "xs",
                            checked_fn: move || check_xs.get(),
                            onchange: toggle(check_xs)
                        }
                        Checkbox {
                            label: "Small", size: "sm",
                            checked_fn: move || check_sm.get(),
                            onchange: toggle(check_sm)
                        }
                        Checkbox {
                            label: "Medium", size: "md",
                            checked_fn: move || check_md.get(),
                            onchange: toggle(check_md)
                        }
                        Checkbox {
                            label: "Large", size: "lg",
                            checked_fn: move || check_lg.get(),
                            onchange: toggle(check_lg)
                        }
                    }
                }

                // Radio
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Radio Group" }
                        Text { size: "sm", color: "dimmed", "Single selection" }
                        Space { h: "xs" }
                        RadioGroup { label: "Select a plan",
                            Radio {
                                name: "plan", value: "free", label: "Free - $0/mo",
                                checked_fn: move || selected_plan.get() == "free",
                                onchange: move || selected_plan.set("free".to_string())
                            }
                            Radio {
                                name: "plan", value: "pro", label: "Pro - $10/mo",
                                checked_fn: move || selected_plan.get() == "pro",
                                onchange: move || selected_plan.set("pro".to_string())
                            }
                            Radio {
                                name: "plan", value: "enterprise", label: "Enterprise - $50/mo",
                                checked_fn: move || selected_plan.get() == "enterprise",
                                onchange: move || selected_plan.set("enterprise".to_string())
                            }
                        }
                    }
                }

                // Switch
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Switch" }
                        Text { size: "sm", color: "dimmed", "Toggle settings on/off" }
                        Space { h: "xs" }
                        Switch {
                            label: "Notifications",
                            checked_fn: move || switch1.get(),
                            onchange: toggle(switch1)
                        }
                        Switch {
                            label: "Dark mode",
                            checked_fn: move || switch2.get(),
                            onchange: toggle(switch2)
                        }
                        Switch { label: "Disabled", disabled: true }
                    }
                }

                // Switch sizes
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Switch Sizes" }
                        Text { size: "sm", color: "dimmed", "Four size options" }
                        Space { h: "xs" }
                        Switch {
                            label: "Extra small", size: "xs",
                            checked_fn: move || switch_xs.get(),
                            onchange: toggle(switch_xs)
                        }
                        Switch {
                            label: "Small", size: "sm",
                            checked_fn: move || switch_sm.get(),
                            onchange: toggle(switch_sm)
                        }
                        Switch {
                            label: "Medium", size: "md",
                            checked_fn: move || switch_md.get(),
                            onchange: toggle(switch_md)
                        }
                        Switch {
                            label: "Large", size: "lg",
                            checked_fn: move || switch_lg.get(),
                            onchange: toggle(switch_lg)
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ============================================
            // SLIDER
            // ============================================
            Title { order: 3, "Slider" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Interactive slider for selecting numeric values within a range." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Volume" }
                            Badge { color: "blue", {|| format!("{:.0}%", volume.get())} }
                        }
                        Slider {
                            value_signal: Some(volume),
                            onchange: move |v| volume.set(v)
                        }
                    }
                }

                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Group { justify: "between",
                            Text { weight: "600", "Brightness" }
                            Badge { color: "orange", {|| format!("{:.0}%", brightness.get())} }
                        }
                        Slider {
                            color: "orange",
                            value_signal: Some(brightness),
                            onchange: move |v| brightness.set(v)
                        }
                    }
                }
            }
        }
    }
}
