//! PasswordInput component.
//!
//! A password input field with visibility toggle.
//! Uses paint-level masking via `type="password"` (rinch-dom renders bullets at the paint layer).
//!
//! # Fine-Grained Reactivity
//!
//! For real-time updates without re-rendering, use the reactive props:
//! - `value_fn`: Closure that returns the current password value
//! - `visible_fn`: Closure that returns whether password is visible
//!
//! These create Effects that update only the affected DOM nodes when signals change.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component, InputCallback};
use std::rc::Rc;

/// PasswordInput size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PasswordInputSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl PasswordInputSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            PasswordInputSize::Xs => "rinch-password-input--xs",
            PasswordInputSize::Sm => "rinch-password-input--sm",
            PasswordInputSize::Md => "rinch-password-input--md",
            PasswordInputSize::Lg => "rinch-password-input--lg",
            PasswordInputSize::Xl => "rinch-password-input--xl",
        }
    }
}

impl std::str::FromStr for PasswordInputSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(PasswordInputSize::Xs),
            "sm" => Ok(PasswordInputSize::Sm),
            "md" => Ok(PasswordInputSize::Md),
            "lg" => Ok(PasswordInputSize::Lg),
            "xl" => Ok(PasswordInputSize::Xl),
            _ => Err(()),
        }
    }
}

/// Reactive callback type for password value.
pub type ReactiveString = Rc<dyn Fn() -> String>;
/// Reactive callback type for visibility.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// A password input component with visibility toggle.
///
/// Uses paint-level masking: `type="password"` causes rinch-dom to render
/// bullet characters at the paint layer. Toggling visibility switches
/// between `type="password"` and `type="text"`.
///
/// # Fine-Grained Reactivity Example
///
/// ```ignore
/// let visible = Signal::new(false);
/// let password = Signal::new(String::new());
///
/// rsx! {
///     PasswordInput {
///         label: "Password",
///         value_fn: move || password.get(),
///         visible_fn: move || visible.get(),
///         oninput: move |new_value| password.set(new_value),
///         ontoggle: move || visible.update(|v| *v = !*v),
///     }
/// }
/// ```
pub struct PasswordInput {
    /// Input label.
    pub label: String,
    /// Description text.
    pub description: String,
    /// Error message.
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Current value (static, for initial render or non-reactive use).
    pub value: String,
    /// Reactive value getter - use this for fine-grained updates.
    /// When provided, the input value updates automatically when the signal changes.
    pub value_fn: Option<ReactiveString>,
    /// Whether the password is visible (static).
    pub visible: bool,
    /// Reactive visibility getter - use this for fine-grained updates.
    pub visible_fn: Option<ReactiveBool>,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Whether the input is required.
    pub required: bool,
    /// Whether to autofocus this input.
    pub autofocus: bool,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether to show the visibility toggle button.
    pub toggle_visibility: bool,
    /// Callback when visibility toggle is clicked.
    pub ontoggle: Option<Callback>,
    /// Callback when input value changes. Receives the actual password value.
    pub oninput: Option<InputCallback>,
}

impl std::fmt::Debug for PasswordInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordInput")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field(
                "value",
                &if self.value.is_empty() {
                    ""
                } else {
                    "[REDACTED]"
                },
            )
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("visible", &self.visible)
            .field(
                "visible_fn",
                &self.visible_fn.as_ref().map(|_| "<reactive>"),
            )
            .field("disabled", &self.disabled)
            .field("required", &self.required)
            .field("autofocus", &self.autofocus)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("toggle_visibility", &self.toggle_visibility)
            .field("ontoggle", &self.ontoggle.as_ref().map(|_| "<callback>"))
            .field("oninput", &self.oninput.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for PasswordInput {
    fn default() -> Self {
        Self {
            label: String::new(),
            description: String::new(),
            error: String::new(),
            placeholder: String::new(),
            value: String::new(),
            value_fn: None,
            visible: false,
            visible_fn: None,
            disabled: false,
            required: false,
            autofocus: false,
            size: String::new(),
            radius: String::new(),
            toggle_visibility: true,
            ontoggle: None,
            oninput: None,
        }
    }
}

impl PasswordInput {
    /// Generate the CSS class string.
    fn class_string(&self) -> String {
        let mut classes = vec!["rinch-password-input"];

        let size: PasswordInputSize = if self.size.is_empty() {
            PasswordInputSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if !self.error.is_empty() {
            classes.push("rinch-password-input--error");
        }
        if self.disabled {
            classes.push("rinch-password-input--disabled");
        }

        classes.join(" ")
    }
}

/// HTML-escape a string for safe use in attributes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Component for PasswordInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container_class = self.class_string();

        // Get initial values
        let initial_password = if let Some(ref f) = self.value_fn {
            f()
        } else {
            self.value.clone()
        };

        let initial_visible = if let Some(ref f) = self.visible_fn {
            f()
        } else {
            self.visible
        };

        let container = rinch_macros::rsx! { div { class: "rinch-password-input" } };
        container.set_attribute("class", &container_class);

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let required_mark = if self.required { " *" } else { "" };
            let label = rinch_macros::rsx! { label { class: "rinch-password-input__label" } };
            let label_text_node =
                __scope.create_text(&format!("{}{}", label_text, required_mark));
            label.append_child(&label_text_node);
            container.append_child(&label);
        }

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-password-input__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Input wrapper (contains input + toggle button)
        let wrapper = rinch_macros::rsx! { div { class: "rinch-password-input__wrapper" } };

        // Input element — uses type="password" for paint-level masking
        let input = rinch_macros::rsx! {
            input {
                class: "rinch-password-input__input",
                autocomplete: "off",
                spellcheck: "false"
            }
        };

        // Set initial type based on visibility
        let input_type = if initial_visible { "text" } else { "password" };
        input.set_attribute("type", input_type);

        // Reactive value binding (following TextInput pattern)
        if let Some(ref value_fn) = self.value_fn {
            let initial_value = value_fn();
            input.set_attribute("value", &html_escape(&initial_value));

            let value_fn = value_fn.clone();
            let input_clone = input.clone();
            __scope.create_effect(move || {
                let current_value = value_fn();
                input_clone.set_attribute("value", &html_escape(&current_value));
            });
        } else if !initial_password.is_empty() {
            input.set_attribute("value", &html_escape(&initial_password));
        }

        // Reactive visibility binding — toggles type attribute
        if let Some(ref visible_fn) = self.visible_fn {
            let visible_fn = visible_fn.clone();
            let input_clone = input.clone();
            __scope.create_effect(move || {
                let is_visible = visible_fn();
                input_clone.set_attribute(
                    "type",
                    if is_visible { "text" } else { "password" },
                );
            });
        }

        if !self.placeholder.is_empty() {
            input.set_attribute("placeholder", &html_escape(&self.placeholder));
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if self.required {
            input.set_attribute("required", "");
        }
        if self.autofocus {
            input.set_attribute("autofocus", "");
        }

        // Always register input handler so runtime routes text input
        {
            let callback = self.oninput.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                if let Some(cb) = &callback {
                    cb.invoke(value);
                }
            });
            input.set_attribute("data-oninput", &handler_id.to_string());
        }

        wrapper.append_child(&input);

        // Visibility toggle button (Mantine-style)
        if self.toggle_visibility && !self.disabled {
            // Pre-create both icons
            let eye_icon = crate::icons::eye_dom(__scope);
            let eye_off_icon = crate::icons::eye_off_dom(__scope);

            // Set initial visibility: eye-off shown when masked, eye shown when revealed
            if initial_visible {
                eye_off_icon.set_attribute("style", "display: none");
            } else {
                eye_icon.set_attribute("style", "display: none");
            }

            let toggle_btn = rinch_macros::rsx! {
                button {
                    class: "rinch-password-input__toggle",
                    r#type: "button",
                    tabindex: "-1"
                }
            };

            let aria_label = if initial_visible {
                "Hide password"
            } else {
                "Show password"
            };
            toggle_btn.set_attribute("aria-label", aria_label);

            toggle_btn.append_child(&eye_icon);
            toggle_btn.append_child(&eye_off_icon);

            // Reactive icon swap via visible_fn
            if let Some(ref visible_fn) = self.visible_fn {
                let visible_fn = visible_fn.clone();
                let eye_clone = eye_icon.clone();
                let eye_off_clone = eye_off_icon.clone();
                let btn_clone = toggle_btn.clone();
                __scope.create_effect(move || {
                    let is_visible = visible_fn();
                    if is_visible {
                        eye_clone.set_attribute("style", "");
                        eye_off_clone.set_attribute("style", "display: none");
                        btn_clone.set_attribute("aria-label", "Hide password");
                    } else {
                        eye_clone.set_attribute("style", "display: none");
                        eye_off_clone.set_attribute("style", "");
                        btn_clone.set_attribute("aria-label", "Show password");
                    }
                });
            }

            if let Some(cb) = &self.ontoggle {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                toggle_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            wrapper.append_child(&toggle_btn);
        }

        container.append_child(&wrapper);

        // Error message
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-password-input__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
