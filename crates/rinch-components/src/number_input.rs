//! NumberInput component.
//!
//! A numeric input field with increment/decrement controls.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component, InputCallback};

/// NumberInput size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberInputSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl NumberInputSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            NumberInputSize::Xs => "rinch-number-input--xs",
            NumberInputSize::Sm => "rinch-number-input--sm",
            NumberInputSize::Md => "rinch-number-input--md",
            NumberInputSize::Lg => "rinch-number-input--lg",
            NumberInputSize::Xl => "rinch-number-input--xl",
        }
    }
}

impl std::str::FromStr for NumberInputSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(NumberInputSize::Xs),
            "sm" => Ok(NumberInputSize::Sm),
            "md" => Ok(NumberInputSize::Md),
            "lg" => Ok(NumberInputSize::Lg),
            "xl" => Ok(NumberInputSize::Xl),
            _ => Err(()),
        }
    }
}

/// A numeric input component with increment/decrement controls.
///
/// Allows users to input numbers with optional step controls.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     NumberInput { label: "Quantity", min: 0.0, max: 100.0, step: 1.0 }
///     NumberInput { label: "Price", value: 9.99, decimal_scale: 2, prefix: "$" }
/// }
/// ```
#[derive(Default)]
pub struct NumberInput {
    /// Input label.
    pub label: String,
    /// Description text.
    pub description: String,
    /// Error message.
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Current value.
    pub value: Option<f64>,
    /// Default value.
    pub default_value: Option<f64>,
    /// Minimum value.
    pub min: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// Step increment.
    pub step: Option<f64>,
    /// Number of decimal places.
    pub decimal_scale: Option<u32>,
    /// Prefix text (e.g., "$").
    pub prefix: String,
    /// Suffix text (e.g., "kg").
    pub suffix: String,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Whether to hide the controls.
    pub hide_controls: bool,
    /// Whether the input is required.
    pub required: bool,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Callback when increment button is clicked.
    pub onincrement: Option<Callback>,
    /// Callback when decrement button is clicked.
    pub ondecrement: Option<Callback>,
    /// Callback when value changes (from direct input).
    pub oninput: Option<InputCallback>,
    /// Callback when the typed gesture commits (focus leaves the input after a
    /// modification, or Enter) — HTML `change` semantics, receives the final
    /// value. Fires only if the value changed since focus (issue #226).
    pub onchange: Option<InputCallback>,
}

impl std::fmt::Debug for NumberInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumberInput")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field("value", &self.value)
            .field("default_value", &self.default_value)
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("decimal_scale", &self.decimal_scale)
            .field("prefix", &self.prefix)
            .field("suffix", &self.suffix)
            .field("disabled", &self.disabled)
            .field("hide_controls", &self.hide_controls)
            .field("required", &self.required)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field(
                "onincrement",
                &self.onincrement.as_ref().map(|_| "<callback>"),
            )
            .field(
                "ondecrement",
                &self.ondecrement.as_ref().map(|_| "<callback>"),
            )
            .field("oninput", &self.oninput.as_ref().map(|_| "<callback>"))
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl NumberInput {
    /// Generate the CSS class string for this number input.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-number-input"];

        // Size class
        let size: NumberInputSize = if self.size.is_empty() {
            NumberInputSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if !self.error.is_empty() {
            classes.push("rinch-number-input--error");
        }
        if self.disabled {
            classes.push("rinch-number-input--disabled");
        }
        if self.hide_controls {
            classes.push("rinch-number-input--no-controls");
        }

        classes.join(" ")
    }
}

impl Component for NumberInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-number-input" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let required_mark = if self.required { " *" } else { "" };
            let label = rinch_macros::rsx! { label { class: "rinch-number-input__label", {format!("{}{}", label_text, required_mark)} } };
            container.append_child(&label);
        }

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-number-input__description", {desc} } };
            container.append_child(&desc_div);
        }

        // Input wrapper
        let wrapper = rinch_macros::rsx! { div { class: "rinch-number-input__wrapper" } };

        // Prefix
        if !self.prefix.is_empty() {
            let prefix = &self.prefix;
            let prefix_span =
                rinch_macros::rsx! { span { class: "rinch-number-input__prefix", {prefix} } };
            wrapper.append_child(&prefix_span);
        }

        // Input element
        let input =
            rinch_macros::rsx! { input { class: "rinch-number-input__input", r#type: "number" } };

        if let Some(v) = self.value {
            input.set_attribute("value", &v.to_string());
        }
        if let Some(min) = self.min {
            input.set_attribute("min", &min.to_string());
        }
        if let Some(max) = self.max {
            input.set_attribute("max", &max.to_string());
        }
        if let Some(step) = self.step {
            input.set_attribute("step", &step.to_string());
        } else {
            input.set_attribute("step", "any");
        }
        if !self.placeholder.is_empty() {
            input.set_attribute("placeholder", &self.placeholder);
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if self.required {
            input.set_attribute("required", "");
        }

        wrapper.append_child(&input);

        // Input handler for direct text entry — registered whenever any commit
        // consumer exists (oninput OR onchange), so the runtime can route
        // focus and typed text to this element (an onchange-only NumberInput
        // must still be focusable). An instance with NEITHER callback stays
        // inert like pre-#226: registering anyway would make a spinner-only
        // NumberInput tab-focusable and freely editable, with the edits
        // reported nowhere (#244 review).
        if self.oninput.is_some() || self.onchange.is_some() {
            let callback = self.oninput.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                if let Some(cb) = &callback {
                    cb.invoke(value);
                }
            });
            input.set_attribute("data-oninput", &handler_id.to_string());
        }

        // Change handler — the commit boundary (issue #226)
        if let Some(callback) = &self.onchange {
            let callback = callback.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                callback.invoke(value);
            });
            input.set_attribute("data-onchange", &handler_id.to_string());
        }

        // Suffix
        if !self.suffix.is_empty() {
            let suffix = &self.suffix;
            let suffix_span =
                rinch_macros::rsx! { span { class: "rinch-number-input__suffix", {suffix} } };
            wrapper.append_child(&suffix_span);
        }

        // Controls (unless hidden)
        if !self.hide_controls {
            let controls = rinch_macros::rsx! { div { class: "rinch-number-input__controls" } };

            // Increment button
            let up_btn = rinch_macros::rsx! {
                button {
                    class: "rinch-number-input__control rinch-number-input__control--up",
                    r#type: "button",
                    tabindex: "-1"
                }
            };
            up_btn.set_attribute("aria-label", "Increment");
            up_btn.append_child(&crate::icons::chevron_up_dom(__scope));

            if let Some(cb) = &self.onincrement {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                up_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            // Decrement button
            let down_btn = rinch_macros::rsx! {
                button {
                    class: "rinch-number-input__control rinch-number-input__control--down",
                    r#type: "button",
                    tabindex: "-1"
                }
            };
            down_btn.set_attribute("aria-label", "Decrement");
            down_btn.append_child(&crate::icons::chevron_down_small_dom(__scope));

            if let Some(cb) = &self.ondecrement {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                down_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            controls.append_child(&up_btn);
            controls.append_child(&down_btn);
            wrapper.append_child(&controls);
        }

        container.append_child(&wrapper);

        // Error message
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-number-input__error", {err} } };
            container.append_child(&err_div);
        }

        container
    }
}
