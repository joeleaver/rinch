//! Inputs section - Form controls and input components.

use rinch::prelude::*;

/// State for the Inputs section, stored in a store.
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
    create_store(InputsSectionState {
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
fn SliderClipTest() -> NodeHandle {
    let panel_open = Signal::new(false);
    let slider_a = Signal::new(50.0);
    let slider_b = Signal::new(25.0);
    let slider_c = Signal::new(75.0);
    let slider_d = Signal::new(10.0);
    let slider_e = Signal::new(90.0);
    let slider_f = Signal::new(40.0);
    let slider_g = Signal::new(60.0);
    let slider_h = Signal::new(30.0);

    rsx! {
        div { style: "position: relative;",
            Button {
                variant: "light",
                onclick: move || panel_open.update(|v| *v = !*v),
                {|| if panel_open.get() { "Close Slider Panel" } else { "Open Slider Panel" }}
            }
            if panel_open.get() {
                div { style: "position: absolute; top: 100%; left: 0; z-index: 100; width: 320px; margin-top: 8px;",
                    Paper { p: "md", radius: "md", with_border: true,
                        style: "box-shadow: var(--rinch-shadow-lg); background: var(--rinch-color-body);",
                        Stack { gap: "xs",
                            Group { justify: "between",
                                Text { weight: "600", "Slider Panel" }
                                Badge { color: "teal", "Clip Test" }
                            }
                            Text { size: "xs", color: "dimmed", "Sliders in a scrolling container — thumbs and bars should clip at the panel boundary." }
                            Divider {}
                            div { style: "overflow-y: auto; max-height: 200px;",
                                Stack { gap: "sm", p: "xs",
                                    Group { justify: "between",
                                        Text { size: "sm", "Alpha" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_a.get())} }
                                    }
                                    Slider { value_signal: Some(slider_a), onchange: move |v| slider_a.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Beta" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_b.get())} }
                                    }
                                    Slider { color: "teal", value_signal: Some(slider_b), onchange: move |v| slider_b.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Gamma" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_c.get())} }
                                    }
                                    Slider { color: "violet", value_signal: Some(slider_c), onchange: move |v| slider_c.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Delta" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_d.get())} }
                                    }
                                    Slider { color: "orange", value_signal: Some(slider_d), onchange: move |v| slider_d.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Epsilon" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_e.get())} }
                                    }
                                    Slider { color: "pink", value_signal: Some(slider_e), onchange: move |v| slider_e.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Zeta" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_f.get())} }
                                    }
                                    Slider { color: "cyan", value_signal: Some(slider_f), onchange: move |v| slider_f.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Eta" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_g.get())} }
                                    }
                                    Slider { color: "grape", value_signal: Some(slider_g), onchange: move |v| slider_g.set(v) }
                                    Group { justify: "between",
                                        Text { size: "sm", "Theta" }
                                        Text { size: "xs", color: "dimmed", {|| format!("{:.0}", slider_h.get())} }
                                    }
                                    Slider { color: "red", value_signal: Some(slider_h), onchange: move |v| slider_h.set(v) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn inputs_section() -> NodeHandle {
    // Get state from store (initialized in main app)
    let state = use_store::<InputsSectionState>();

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

            // Overflow clipping test: floating panel with sliders in a scrolling area
            SliderClipTest {}

            // ============================================
            // COLOR PICKER
            // ============================================
            Space { h: "xl" }
            Title { order: 3, "Color Picker" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Pick colors interactively with saturation panel, hue/alpha sliders, and preset swatches." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: "lg",
                // Basic ColorPicker
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Color Picker" }
                        Divider {}
                        ColorPicker {
                            value: "#339af0",
                            alpha: true,
                            with_input: true,
                            swatches: vec![
                                "#fa5252".into(), "#e64980".into(), "#be4bdb".into(),
                                "#7950f2".into(), "#4c6ef5".into(), "#228be6".into(),
                                "#15aabf".into(), "#12b886".into(), "#40c057".into(),
                                "#82c91e".into(), "#fab005".into(), "#fd7e14".into(),
                                "#868e96".into(), "#000000".into(),
                            ]
                        }
                    }
                }

                // ColorInput
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Color Input" }
                        Divider {}
                        ColorInput {
                            label: "Pick a color",
                            placeholder: "#000000",
                            value: "#228be6",
                            alpha: true,
                            swatches: vec![
                                "#fa5252".into(), "#e64980".into(), "#be4bdb".into(),
                                "#7950f2".into(), "#4c6ef5".into(), "#228be6".into(),
                                "#15aabf".into(),
                            ]
                        }

                        Space { h: "sm" }

                        // Color swatches row
                        Text { size: "sm", weight: "600", "Color Swatches" }
                        Group { gap: "xs",
                            ColorSwatch { color: "#fa5252", size: "32px" }
                            ColorSwatch { color: "#e64980", size: "32px" }
                            ColorSwatch { color: "#be4bdb", size: "32px" }
                            ColorSwatch { color: "#7950f2", size: "32px" }
                            ColorSwatch { color: "#4c6ef5", size: "32px" }
                            ColorSwatch { color: "#228be6", size: "32px" }
                            ColorSwatch { color: "rgba(34, 139, 230, 0.5)", size: "32px" }
                        }
                    }
                }
            }
        }
    }
}
