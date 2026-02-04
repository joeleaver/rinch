//! Inputs section - Form controls and input components.

use rinch::prelude::*;
use std::rc::Rc;

/// State for the Inputs section, stored in context.
#[derive(Clone)]
pub struct InputsSectionState {
    pub check1: Signal<bool>,
    pub check2: Signal<bool>,
    pub switch1: Signal<bool>,
    pub switch2: Signal<bool>,
}

/// Initialize the Inputs section state.
pub fn init_inputs_state() {
    create_context(InputsSectionState {
        check1: Signal::new(false),
        check2: Signal::new(true),
        switch1: Signal::new(false),
        switch2: Signal::new(true),
    });
}

pub fn inputs_section(__scope: &mut RenderScope) -> NodeHandle {
    let state = use_context::<InputsSectionState>();

    let (check1, check2, switch1, switch2) = match state {
        Some(s) => (s.check1, s.check2, s.switch1, s.switch2),
        None => {
            return rsx! { div { "Error: InputsSectionState not initialized" } };
        }
    };

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Inputs" }
                Text { size: "lg", color: "dimmed",
                    "Form controls and input components for user interaction."
                }
            }
            Space { h: "xl" }

            // TextInput
            Title { order: 3, "Text Input" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Single-line text input with label and validation." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic" }
                        Divider {}
                        TextInput { label: "Name", placeholder: "Enter your name" }
                        TextInput { label: "Email", placeholder: "you@example.com", required: true }
                        TextInput { label: "Disabled", placeholder: "Cannot edit", disabled: true }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "With Description & Error" }
                        Divider {}
                        TextInput {
                            label: "Username",
                            placeholder: "Pick a username",
                            description: "Must be 3-20 characters"
                        }
                        TextInput {
                            label: "Password",
                            placeholder: "Enter password",
                            error: "Password must be at least 8 characters"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Checkbox and Switch
            Title { order: 3, "Checkbox & Switch" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Toggle controls for boolean values." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Checkbox" }
                        Divider {}
                        Checkbox {
                            label: "Accept terms and conditions",
                            checked_fn: Some(Rc::new(move || check1.get())),
                            onchange: move || check1.update(|v| *v = !*v)
                        }
                        Checkbox {
                            label: "Subscribe to newsletter",
                            checked_fn: Some(Rc::new(move || check2.get())),
                            onchange: move || check2.update(|v| *v = !*v)
                        }
                        Checkbox {
                            label: "Disabled checkbox",
                            disabled: true
                        }
                        Checkbox {
                            label: "Disabled checked",
                            checked: true,
                            disabled: true
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Switch" }
                        Divider {}
                        Switch {
                            label: Some("Enable notifications".to_string()),
                            checked_fn: Some(Rc::new(move || switch1.get())),
                            onchange: move || switch1.update(|v| *v = !*v)
                        }
                        Switch {
                            label: Some("Dark mode".to_string()),
                            checked_fn: Some(Rc::new(move || switch2.get())),
                            onchange: move || switch2.update(|v| *v = !*v)
                        }
                        Switch {
                            label: Some("Disabled switch".to_string()),
                            disabled: true
                        }
                        Switch {
                            label: Some("Disabled on".to_string()),
                            checked: true,
                            disabled: true
                        }
                    }
                }
            }

            Space { h: "xl" }

            // Select
            Title { order: 3, "Select" }
            Space { h: "sm" }
            Text { color: "dimmed", size: "sm", "Dropdown select for choosing from options." }
            Space { h: "md" }

            SimpleGrid { cols: Some(2), spacing: Some("lg".to_string()),
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "Basic Select" }
                        Divider {}
                        Select { label: "Favorite color",
                            option { value: "blue", "Blue" }
                            option { value: "red", "Red" }
                            option { value: "green", "Green" }
                            option { value: "purple", "Purple" }
                        }
                        Select { label: "Disabled", disabled: true,
                            option { value: "a", "Option A" }
                        }
                    }
                }
                Paper { p: "xl", radius: "md", with_border: true,
                    Stack { gap: "md",
                        Text { weight: "600", "With Validation" }
                        Divider {}
                        Select {
                            label: "Country",
                            required: true,
                            description: "Select your country",
                            option { value: "", "Choose..." }
                            option { value: "us", "United States" }
                            option { value: "uk", "United Kingdom" }
                            option { value: "de", "Germany" }
                            option { value: "jp", "Japan" }
                        }
                        Select {
                            label: "Plan",
                            error: "Please select a plan",
                            option { value: "", "Choose..." }
                            option { value: "free", "Free" }
                            option { value: "pro", "Pro" }
                        }
                    }
                }
            }
        }
    }
}
